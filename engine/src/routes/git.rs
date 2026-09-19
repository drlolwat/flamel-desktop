use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Component, Path},
    sync::Arc,
};

use crate::AppState;
use crate::routes::admin::{get_current_user_id, get_user_home};

#[derive(Deserialize)]
pub struct GitQuery {
    path: Option<String>,
    file: Option<String>,
    mode: Option<String>,
}

async fn run_git(args: &[&str], cwd: &str) -> (String, String, i32) {
    run_git_timeout(args, cwd, 5).await
}

async fn run_git_timeout(args: &[&str], cwd: &str, timeout_seconds: u64) -> (String, String, i32) {
    let args: Vec<String> = args.iter().map(|value| (*value).to_owned()).collect();
    let cwd = cwd.to_owned();
    match tokio::task::spawn_blocking(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::git_operations::command_output(
            "git",
            &args,
            &cwd,
            &[],
            None,
            std::time::Duration::from_secs(timeout_seconds),
        )
    })
    .await
    {
        Ok(Ok(output)) => (output.stdout, output.stderr, output.code),
        Ok(Err(error)) => (String::new(), error, 1),
        Err(error) => (
            String::new(),
            format!("Git operation interrupted: {error}"),
            1,
        ),
    }
}

async fn run_git_owned(args: &[String], cwd: &str, timeout_seconds: u64) -> (String, String, i32) {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_git_timeout(&refs, cwd, timeout_seconds).await
}

fn git_status_code(ch: char) -> &'static str {
    match ch {
        'A' => "A",
        'M' => "M",
        'D' => "D",
        'R' => "R",
        'C' => "C",
        'T' => "T",
        'U' => "U",
        '?' => "?",
        '!' => "!",
        _ => " ",
    }
}

fn primary_status(index: char, worktree: char, untracked: bool) -> &'static str {
    if untracked {
        "?"
    } else if worktree != ' ' {
        git_status_code(worktree)
    } else {
        git_status_code(index)
    }
}

#[derive(Clone, Copy, Default)]
struct NumStat {
    staged_added: i64,
    staged_removed: i64,
    unstaged_added: i64,
    unstaged_removed: i64,
}

fn add_numstat(stats: &mut HashMap<String, NumStat>, output: &str, staged: bool) {
    let mut records = output.split('\0').filter(|record| !record.is_empty());
    while let Some(line) = records.next() {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() != 3 {
            continue;
        }
        let added: i64 = parts[0].parse().unwrap_or(0);
        let removed: i64 = parts[1].parse().unwrap_or(0);
        // -z encodes renames as counts + empty path, old path, new path.
        let path = if parts[2].is_empty() {
            let _ = records.next();
            let Some(path) = records.next() else { break };
            path
        } else {
            parts[2]
        };
        let entry = stats.entry(path.to_string()).or_default();
        if staged {
            entry.staged_added += added;
            entry.staged_removed += removed;
        } else {
            entry.unstaged_added += added;
            entry.unstaged_removed += removed;
        }
    }
}

fn parse_status(output: &str) -> Result<Vec<Value>, String> {
    let mut files = Vec::new();
    let mut records = output.split('\0').filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        let bytes = record.as_bytes();
        if bytes.len() < 4 || bytes[2] != b' ' || !bytes[0].is_ascii() || !bytes[1].is_ascii() {
            return Err("Git returned an invalid status record".into());
        }
        let index = bytes[0] as char;
        let worktree = bytes[1] as char;
        let untracked = &bytes[..2] == b"??";
        let original_path = if matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
            Some(records.next().ok_or("Git returned an incomplete rename")?)
        } else {
            None
        };
        files.push(json!({
            "status": primary_status(index, worktree, untracked),
            "path": &record[3..],
            "originalPath": original_path,
            "staged": !untracked && index != ' ',
            "unstaged": untracked || worktree != ' ',
            "untracked": untracked,
            "indexStatus": git_status_code(index),
            "worktreeStatus": git_status_code(worktree),
        }));
    }
    Ok(files)
}

fn git_failure(detail: impl AsRef<str>) -> (StatusCode, Json<Value>) {
    let detail: String = detail.as_ref().trim().chars().take(600).collect();
    (
        StatusCode::CONFLICT,
        Json(json!({"error": format!("Could not read repository state: {detail}")})),
    )
}

fn safe_relative_git_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn request_repo<'a>(
    state: &AppState,
    headers: &HeaderMap,
    path: &'a str,
) -> Result<&'a str, (StatusCode, Json<Value>)> {
    let user_id = get_current_user_id(state, headers)
        .ok_or_else(|| (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))))?;
    let user_home = get_user_home(state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);
    super::path_guard::guard_existing(&access_root, path)?;
    Ok(path)
}

async fn ensure_repo(path: &str) -> Result<(), (StatusCode, Json<Value>)> {
    let (_, stderr, code) = run_git(&["rev-parse", "--git-dir"], path).await;
    if code == 0 {
        Ok(())
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Not a git repository", "detail": stderr.trim()})),
        ))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitCommitSummary {
    id: String,
    short_id: String,
    parents: Vec<String>,
    author_name: String,
    author_email: String,
    authored_at: String,
    subject: String,
    refs: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitBranchSummary {
    name: String,
    current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
}

fn parse_branch_summary(line: &str) -> Option<GitBranchSummary> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() < 5 {
        return None;
    }
    let full_ref = fields[0];
    let short_ref = fields[1];
    if full_ref.starts_with("refs/remotes/") && short_ref.ends_with("/HEAD") {
        return None;
    }
    let remote = full_ref
        .strip_prefix("refs/remotes/")
        .and_then(|name| name.split('/').next())
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    Some(GitBranchSummary {
        name: short_ref.to_string(),
        current: fields[4] == "*",
        remote,
        upstream: (!fields[3].is_empty()).then(|| fields[3].to_string()),
        commit: (!fields[2].is_empty()).then(|| fields[2].to_string()),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitDiffLine {
    kind: &'static str,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_line: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitDiffFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    old_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_path: Option<String>,
    status: String,
    binary: bool,
    additions: u32,
    deletions: u32,
    lines: Vec<GitDiffLine>,
}

fn clean_diff_path(raw: &str) -> Option<String> {
    let value = raw.trim().trim_matches('"');
    if value == "/dev/null" {
        return None;
    }
    Some(
        value
            .strip_prefix("a/")
            .or_else(|| value.strip_prefix("b/"))
            .unwrap_or(value)
            .to_string(),
    )
}

fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    let mut parts = line.split_whitespace();
    parts.next()?;
    let old = parts
        .next()?
        .trim_start_matches('-')
        .split(',')
        .next()?
        .parse()
        .ok()?;
    let new = parts
        .next()?
        .trim_start_matches('+')
        .split(',')
        .next()?
        .parse()
        .ok()?;
    Some((old, new))
}

fn parse_structured_diff(raw: &str) -> Vec<GitDiffFile> {
    let mut files = Vec::new();
    let mut current: Option<GitDiffFile> = None;
    let mut old_line = 0u32;
    let mut new_line = 0u32;

    for line in raw.lines() {
        if line.starts_with("diff --git ") {
            if let Some(file) = current.take() {
                files.push(file);
            }
            current = Some(GitDiffFile {
                old_path: None,
                new_path: None,
                status: "M".to_string(),
                binary: false,
                additions: 0,
                deletions: 0,
                lines: vec![GitDiffLine {
                    kind: "metadata",
                    content: line.to_string(),
                    old_line: None,
                    new_line: None,
                }],
            });
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue;
        };

        if let Some(path) = line.strip_prefix("--- ") {
            file.old_path = clean_diff_path(path);
            if file.old_path.is_none() {
                file.status = "A".to_string();
            }
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ ") {
            file.new_path = clean_diff_path(path);
            if file.new_path.is_none() {
                file.status = "D".to_string();
            }
            continue;
        }
        if let Some(path) = line.strip_prefix("rename from ") {
            file.old_path = Some(path.to_string());
            file.status = "R".to_string();
        } else if let Some(path) = line.strip_prefix("rename to ") {
            file.new_path = Some(path.to_string());
            file.status = "R".to_string();
        } else if line.starts_with("new file mode ") {
            file.status = "A".to_string();
        } else if line.starts_with("deleted file mode ") {
            file.status = "D".to_string();
        }

        if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            file.binary = true;
            file.lines.push(GitDiffLine {
                kind: "metadata",
                content: line.to_string(),
                old_line: None,
                new_line: None,
            });
        } else if line.starts_with("@@") {
            if let Some((old, new)) = parse_hunk_header(line) {
                old_line = old;
                new_line = new;
            }
            file.lines.push(GitDiffLine {
                kind: "hunk",
                content: line.to_string(),
                old_line: None,
                new_line: None,
            });
        } else if let Some(content) = line.strip_prefix('+') {
            file.additions += 1;
            file.lines.push(GitDiffLine {
                kind: "addition",
                content: content.to_string(),
                old_line: None,
                new_line: Some(new_line),
            });
            new_line += 1;
        } else if let Some(content) = line.strip_prefix('-') {
            file.deletions += 1;
            file.lines.push(GitDiffLine {
                kind: "deletion",
                content: content.to_string(),
                old_line: Some(old_line),
                new_line: None,
            });
            old_line += 1;
        } else if let Some(content) = line.strip_prefix(' ') {
            file.lines.push(GitDiffLine {
                kind: "context",
                content: content.to_string(),
                old_line: Some(old_line),
                new_line: Some(new_line),
            });
            old_line += 1;
            new_line += 1;
        } else if line == "\\ No newline at end of file" {
            file.lines.push(GitDiffLine {
                kind: "no_newline",
                content: line.to_string(),
                old_line: None,
                new_line: None,
            });
        } else if !line.is_empty() {
            file.lines.push(GitDiffLine {
                kind: "metadata",
                content: line.to_string(),
                old_line: None,
                new_line: None,
            });
        }
    }
    if let Some(file) = current {
        files.push(file);
    }
    files
}

async fn load_raw_diff(path: &str, file: Option<&str>, mode: &str) -> Result<String, String> {
    let mut args: Vec<String> = match (file, mode) {
        (Some(file), "staged") => vec!["diff", "--cached", "--", file],
        (Some(file), "head") => vec!["diff", "HEAD", "--", file],
        (Some(file), _) => vec!["diff", "--", file],
        (None, "staged") => vec!["diff", "--cached"],
        (None, "head") => vec!["diff", "HEAD"],
        (None, _) => vec!["diff"],
    }
    .into_iter()
    .map(String::from)
    .collect();
    args.insert(1, "--find-renames".to_string());
    args.insert(1, "--no-ext-diff".to_string());

    let (mut stdout, stderr, code) = run_git_owned(&args, path, 10).await;
    if code != 0 {
        return Err(stderr);
    }
    if stdout.is_empty() && mode != "staged" {
        if let Some(file) = file {
            let file_path = Path::new(path).join(file);
            if safe_relative_git_path(file) && file_path.is_file() {
                let (_, _, tracked_code) =
                    run_git(&["ls-files", "--error-unmatch", "--", file], path).await;
                if tracked_code != 0 {
                    let args = vec![
                        "diff".to_string(),
                        "--no-index".to_string(),
                        "--no-ext-diff".to_string(),
                        "--".to_string(),
                        "/dev/null".to_string(),
                        file_path.to_string_lossy().to_string(),
                    ];
                    let (untracked_diff, stderr, code) = run_git_owned(&args, path, 10).await;
                    if code != 0 && code != 1 {
                        return Err(stderr);
                    }
                    stdout = untracked_diff;
                }
            }
        }
    }
    Ok(stdout)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHistoryQuery {
    path: String,
    limit: Option<u16>,
    skip: Option<u32>,
    file: Option<String>,
}

#[derive(Deserialize)]
pub struct GitFileQuery {
    path: String,
    file: String,
    version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOperationRequest {
    expected_revision: Option<String>,
    path: String,
    operation: String,
    files: Option<Vec<String>>,
    message: Option<String>,
    target: Option<String>,
    remote: Option<String>,
    force_with_lease: Option<bool>,
}

fn validate_ref_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !value.starts_with('-')
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
}

pub async fn handle_git_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<GitQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let user_home = get_user_home(&state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);

    let path = match &query.path {
        Some(p) => p.as_str(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "path required"})),
            );
        }
    };

    if let Err(e) = super::path_guard::guard_existing(&access_root, path) {
        return e;
    }

    // Check if git repo
    let (_, stderr, code) = run_git(&["rev-parse", "--git-dir"], path).await;
    if code != 0 {
        if !stderr.contains("not a git repository") {
            return git_failure(stderr);
        }
        return (
            StatusCode::OK,
            Json(json!({
                "isRepo": false, "branch": "", "files": [],
                "added": 0, "removed": 0, "uncommittedCount": 0,
            })),
        );
    }

    // Get branch
    let revision_path = path.to_owned();
    let revision = match tokio::task::spawn_blocking(move || {
        crate::git_operations::index_revision(&revision_path)
    })
    .await
    {
        Ok(Ok(revision)) => revision,
        Ok(Err(error)) => return git_failure(error),
        Err(error) => return git_failure(error.to_string()),
    };
    let (branch_out, _, branch_code) =
        run_git(&["symbolic-ref", "--quiet", "--short", "HEAD"], path).await;
    let branch = if branch_code == 0 {
        branch_out.trim()
    } else {
        "HEAD"
    };

    // Get porcelain status. The first column is index/staged state and the second
    // column is worktree state, which lets the UI show both without extra git calls.
    let (status_out, stderr, code) = run_git(
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        path,
    )
    .await;
    if code != 0 {
        return git_failure(stderr);
    }
    let files = match parse_status(&status_out) {
        Ok(files) => files,
        Err(error) => return git_failure(error),
    };

    let (unstaged_numstat, stderr, code) =
        run_git(&["diff", "--no-ext-diff", "--numstat", "-z"], path).await;
    if code != 0 {
        return git_failure(stderr);
    }
    let (staged_numstat, stderr, code) = run_git(
        &["diff", "--no-ext-diff", "--cached", "--numstat", "-z"],
        path,
    )
    .await;
    if code != 0 {
        return git_failure(stderr);
    }
    let mut file_stats: HashMap<String, NumStat> = HashMap::new();
    add_numstat(&mut file_stats, &unstaged_numstat, false);
    add_numstat(&mut file_stats, &staged_numstat, true);

    let mut total_added: i64 = 0;
    let mut total_removed: i64 = 0;
    let enriched_files: Vec<Value> = files
        .iter()
        .map(|f| {
            let fp = f.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let mut obj = f.clone();
            let stats = file_stats.get(fp).copied().unwrap_or_default();
            let added = stats.staged_added + stats.unstaged_added;
            let removed = stats.staged_removed + stats.unstaged_removed;
            total_added += added;
            total_removed += removed;
            if let Some(map) = obj.as_object_mut() {
                if added > 0 {
                    map.insert("added".to_string(), json!(added));
                }
                if removed > 0 {
                    map.insert("removed".to_string(), json!(removed));
                }
                if stats.staged_added > 0 {
                    map.insert("stagedAdded".to_string(), json!(stats.staged_added));
                }
                if stats.staged_removed > 0 {
                    map.insert("stagedRemoved".to_string(), json!(stats.staged_removed));
                }
                if stats.unstaged_added > 0 {
                    map.insert("unstagedAdded".to_string(), json!(stats.unstaged_added));
                }
                if stats.unstaged_removed > 0 {
                    map.insert("unstagedRemoved".to_string(), json!(stats.unstaged_removed));
                }
            }
            obj
        })
        .collect();

    let uncommitted_count = enriched_files.len();
    let (remote, _, code) = run_git(&["remote", "get-url", "origin"], path).await;
    let repository_url = (code == 0)
        .then(|| crate::git_operations::repository_web_url(&remote))
        .flatten();

    (
        StatusCode::OK,
        Json(json!({
            "isRepo": true,
            "revision": revision,
            "repositoryUrl": repository_url,
            "branch": branch,
            "files": enriched_files,
            "added": total_added,
            "removed": total_removed,
            "uncommittedCount": uncommitted_count,
        })),
    )
}

pub async fn handle_git_diff(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<GitQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let user_home = get_user_home(&state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);

    let path = match &query.path {
        Some(p) => p.as_str(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "path required"})),
            );
        }
    };

    if let Err(e) = super::path_guard::guard_existing(&access_root, path) {
        return e;
    }

    let mode = query.mode.as_deref().unwrap_or("worktree");
    if !matches!(mode, "worktree" | "staged" | "head")
        || query
            .file
            .as_deref()
            .is_some_and(|file| !safe_relative_git_path(file))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid diff mode or file path"})),
        );
    }
    let stdout = match load_raw_diff(path, query.file.as_deref(), mode).await {
        Ok(stdout) => stdout,
        Err(error) => return git_failure(error),
    };

    (StatusCode::OK, Json(json!({"diff": stdout})))
}

pub async fn handle_git_structured_diff(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<GitQuery>,
) -> (StatusCode, Json<Value>) {
    let Some(path) = query.path.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "path required"})),
        );
    };
    if let Err(error) = request_repo(&state, &headers, path) {
        return error;
    }
    if let Err(error) = ensure_repo(path).await {
        return error;
    }
    if let Some(file) = query.file.as_deref() {
        if !safe_relative_git_path(file) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid file path"})),
            );
        }
    }

    let mode = query.mode.as_deref().unwrap_or("worktree");
    if !matches!(mode, "worktree" | "staged" | "head") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid diff mode"})),
        );
    }
    let raw = match load_raw_diff(path, query.file.as_deref(), mode).await {
        Ok(raw) => raw,
        Err(error) => return git_failure(error),
    };
    let files = parse_structured_diff(&raw);
    (StatusCode::OK, Json(json!({"files": files})))
}

pub async fn handle_git_history(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<GitHistoryQuery>,
) -> (StatusCode, Json<Value>) {
    if let Err(error) = request_repo(&state, &headers, &query.path) {
        return error;
    }
    if let Err(error) = ensure_repo(&query.path).await {
        return error;
    }
    if let Some(file) = query.file.as_deref() {
        if !safe_relative_git_path(file) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid file path"})),
            );
        }
    }

    let limit = query.limit.unwrap_or(50).clamp(1, 200).to_string();
    let skip = query.skip.unwrap_or(0).to_string();
    let mut args = vec![
        "log".to_string(),
        format!("--max-count={limit}"),
        format!("--skip={skip}"),
        "--date=iso-strict".to_string(),
        "--pretty=format:%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%s%x1f%D%x1e".to_string(),
    ];
    if let Some(file) = query.file.as_deref() {
        args.extend(["--".to_string(), file.to_string()]);
    }
    let (stdout, stderr, code) = run_git_owned(&args, &query.path, 10).await;
    if code != 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "git history failed", "detail": stderr.trim()})),
        );
    }
    let commits: Vec<GitCommitSummary> = stdout
        .split('\u{1e}')
        .filter_map(|record| {
            let fields: Vec<&str> = record.trim().split('\u{1f}').collect();
            if fields.len() < 8 {
                return None;
            }
            Some(GitCommitSummary {
                id: fields[0].to_string(),
                short_id: fields[1].to_string(),
                parents: fields[2].split_whitespace().map(String::from).collect(),
                author_name: fields[3].to_string(),
                author_email: fields[4].to_string(),
                authored_at: fields[5].to_string(),
                subject: fields[6].to_string(),
                refs: fields[7]
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(String::from)
                    .collect(),
            })
        })
        .collect();
    (StatusCode::OK, Json(json!({"commits": commits})))
}

pub async fn handle_git_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<GitFileQuery>,
) -> (StatusCode, Json<Value>) {
    if let Err(error) = request_repo(&state, &headers, &query.path) {
        return error;
    }
    if let Err(error) = ensure_repo(&query.path).await {
        return error;
    }
    if !safe_relative_git_path(&query.file) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid file path"})),
        );
    }
    let spec = match query.version.as_deref().unwrap_or("head") {
        "head" => format!("HEAD:{}", query.file),
        "index" => format!(":{}", query.file),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid file version"})),
            );
        }
    };
    let (stdout, stderr, code) = run_git(&["show", &spec], &query.path).await;
    if code != 0 {
        return git_failure(stderr);
    }
    if stdout.len() > 5 * 1024 * 1024 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"error": "file is too large to preview"})),
        );
    }
    if stdout.contains('\0') {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(json!({"error": "binary file preview is unavailable"})),
        );
    }
    (StatusCode::OK, Json(json!({"content": stdout})))
}

pub async fn handle_git_branches(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<GitQuery>,
) -> (StatusCode, Json<Value>) {
    let Some(path) = query.path.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "path required"})),
        );
    };
    if let Err(error) = request_repo(&state, &headers, path) {
        return error;
    }
    if let Err(error) = ensure_repo(path).await {
        return error;
    }
    let (stdout, stderr, code) = run_git(
        &[
            "for-each-ref",
            "--format=%(refname)%09%(refname:short)%09%(objectname:short)%09%(upstream:short)%09%(HEAD)",
            "refs/heads",
            "refs/remotes",
        ],
        path,
    )
    .await;
    if code != 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "git branches failed", "detail": stderr.trim()})),
        );
    }
    let branches: Vec<GitBranchSummary> = stdout.lines().filter_map(parse_branch_summary).collect();
    (StatusCode::OK, Json(json!({"branches": branches})))
}

pub async fn handle_git_operation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<GitOperationRequest>,
) -> (StatusCode, Json<Value>) {
    if let Err(error) = request_repo(&state, &headers, &body.path) {
        return error;
    }
    if let Err(error) = ensure_repo(&body.path).await {
        return error;
    }
    let lock_path = body.path.clone();
    let _guard = match tokio::task::spawn_blocking(move || {
        crate::git_operations::MutationGuard::acquire(&lock_path)
    })
    .await
    {
        Ok(Ok(guard)) => guard,
        Ok(Err(error)) => return (StatusCode::CONFLICT, Json(json!({"error": error}))),
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": error.to_string()})),
            );
        }
    };
    let files = body.files.unwrap_or_default();
    if matches!(body.operation.as_str(), "commit" | "amend") {
        if let Some(expected) = &body.expected_revision {
            let revision_path = body.path.clone();
            let current = tokio::task::spawn_blocking(move || {
                crate::git_operations::index_revision(&revision_path)
            })
            .await;
            match current {
                Ok(Ok(current)) if &current == expected => {}
                Ok(Ok(_)) => {
                    return (
                        StatusCode::CONFLICT,
                        Json(
                            json!({"error": "The staged changes or branch changed. Refresh and review them before saving this version.", "code": "revision_conflict"}),
                        ),
                    );
                }
                Ok(Err(error)) => return git_failure(error),
                Err(error) => return git_failure(error.to_string()),
            }
        }
    }
    if files.iter().any(|file| !safe_relative_git_path(file)) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid file path"})),
        );
    }
    let target = body.target.as_deref().unwrap_or("");
    if !target.is_empty() && !validate_ref_name(target) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid git target"})),
        );
    }
    let remote = body.remote.as_deref().unwrap_or("origin");
    if !validate_ref_name(remote) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid remote"})),
        );
    }

    let mut args: Vec<String> = match body.operation.as_str() {
        "stage" if !files.is_empty() => vec!["add".into(), "--".into()],
        "unstage" if !files.is_empty() => vec!["restore".into(), "--staged".into(), "--".into()],
        "discard" if !files.is_empty() => vec!["restore".into(), "--worktree".into(), "--".into()],
        "commit" => {
            let message = body.message.as_deref().map(str::trim).unwrap_or("");
            if message.is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "commit message required"})),
                );
            }
            vec!["commit".into(), "-m".into(), message.into()]
        }
        "amend" => {
            let message = body.message.as_deref().map(str::trim).unwrap_or("");
            if message.is_empty() {
                vec!["commit".into(), "--amend".into(), "--no-edit".into()]
            } else {
                vec![
                    "commit".into(),
                    "--amend".into(),
                    "-m".into(),
                    message.into(),
                ]
            }
        }
        "fetch" => vec!["fetch".into(), remote.into(), "--prune".into()],
        "pull" => vec!["pull".into(), "--ff-only".into(), remote.into()],
        "push" => {
            let mut command = vec!["push".into()];
            let (branch, stderr, code) =
                run_git(&["symbolic-ref", "--quiet", "--short", "HEAD"], &body.path).await;
            if code != 0 {
                return git_failure(stderr);
            }
            let branch = branch.trim();
            let needs_upstream =
                match crate::git_operations::branch_needs_upstream(&body.path, branch) {
                    Ok(value) => value,
                    Err(error) => return git_failure(error),
                };
            if needs_upstream {
                command.push("--set-upstream".into());
            }
            if body.force_with_lease.unwrap_or(false) {
                match crate::git_operations::force_lease(&body.path, remote, branch) {
                    Ok(lease) => command.push(lease),
                    Err(error) => return (StatusCode::CONFLICT, Json(json!({"error": error}))),
                }
            }
            command.push(remote.into());
            if needs_upstream {
                command.push(branch.into());
            }
            command
        }
        "sync" => vec!["pull".into(), "--rebase".into(), remote.into()],
        "branch_create" if !target.is_empty() => vec!["switch".into(), "-c".into(), target.into()],
        "branch_checkout" if !target.is_empty() => vec!["switch".into(), target.into()],
        "branch_delete" if !target.is_empty() => vec!["branch".into(), "-d".into(), target.into()],
        "tag_create" if !target.is_empty() => vec!["tag".into(), target.into()],
        "stash_push" => {
            let mut command = vec!["stash".into(), "push".into(), "--include-untracked".into()];
            if let Some(message) = body
                .message
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                command.extend(["-m".into(), message.into()]);
            }
            command
        }
        "stash_apply" => {
            let mut command = vec!["stash".into(), "apply".into()];
            if !target.is_empty() {
                command.push(target.into());
            }
            command
        }
        "stash_pop" => {
            let mut command = vec!["stash".into(), "pop".into()];
            if !target.is_empty() {
                command.push(target.into());
            }
            command
        }
        "merge" if !target.is_empty() => vec!["merge".into(), "--no-edit".into(), target.into()],
        "rebase" if !target.is_empty() => vec!["rebase".into(), target.into()],
        "cherry_pick" if !target.is_empty() => vec!["cherry-pick".into(), target.into()],
        "revert" if !target.is_empty() => vec!["revert".into(), "--no-edit".into(), target.into()],
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid git operation or missing arguments"})),
            );
        }
    };
    if matches!(body.operation.as_str(), "stage" | "unstage" | "discard") {
        args.extend(files);
    }
    let timeout = if matches!(body.operation.as_str(), "fetch" | "pull" | "push" | "sync") {
        120
    } else {
        30
    };
    let (stdout, stderr, code) = run_git_owned(&args, &body.path, timeout).await;
    let status = if code == 0 {
        StatusCode::OK
    } else {
        StatusCode::CONFLICT
    };
    (
        status,
        Json(json!({
            "ok": code == 0,
            "error": (code != 0).then(|| if stderr.is_empty() { "Git operation failed" } else { stderr.trim() }),
            "operation": body.operation,
            "stdout": stdout,
            "stderr": stderr,
            "exitCode": code,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::{add_numstat, parse_branch_summary, parse_status, parse_structured_diff};

    #[test]
    fn nul_status_preserves_whitespace_unicode_and_rename_paths() {
        let files = parse_status(" M  leading -> trailing \0?? tab\tand\nnewline-é.txt\0R  renamed.txt\0old -> name.txt\0").unwrap();
        assert_eq!(files[0]["path"], " leading -> trailing ");
        assert_eq!(files[1]["path"], "tab\tand\nnewline-é.txt");
        assert_eq!(files[2]["path"], "renamed.txt");
        assert_eq!(files[2]["originalPath"], "old -> name.txt");
        assert!(parse_status("R  missing-old\0").is_err());
        let mut stats = std::collections::HashMap::new();
        add_numstat(
            &mut stats,
            "2\t1\ttab\tand\nnewline-é.txt\03\t0\t\0old -> name.txt\0renamed.txt\0",
            true,
        );
        assert_eq!(stats["tab\tand\nnewline-é.txt"].staged_added, 2);
        assert_eq!(stats["renamed.txt"].staged_added, 3);
    }

    #[tokio::test]
    async fn failed_diff_is_not_reported_as_an_empty_success() {
        assert!(
            super::load_raw_diff("/definitely/missing/flamel-repository", None, "worktree")
                .await
                .is_err()
        );
    }

    #[test]
    fn structured_diff_tracks_context_line_numbers_once() {
        let files = parse_structured_diff(
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -4,3 +4,3 @@\n same\n-old\n+new\n tail\n",
        );
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 1);
        assert_eq!(files[0].lines[2].old_line, Some(4));
        assert_eq!(files[0].lines[3].old_line, Some(5));
        assert_eq!(files[0].lines[4].new_line, Some(5));
        assert_eq!(files[0].lines[5].old_line, Some(6));
        assert_eq!(files[0].lines[5].new_line, Some(6));
    }

    #[test]
    fn branch_parser_does_not_treat_local_slashes_as_remotes() {
        let local = parse_branch_summary(
            "refs/heads/feature/editor\tfeature/editor\tabc1234\torigin/feature/editor\t*",
        )
        .expect("local branch");
        assert_eq!(local.name, "feature/editor");
        assert!(local.current);
        assert_eq!(local.remote, None);

        let remote = parse_branch_summary("refs/remotes/origin/main\torigin/main\tdef5678\t\t ")
            .expect("remote branch");
        assert_eq!(remote.remote.as_deref(), Some("origin"));
        assert!(!remote.current);

        assert!(
            parse_branch_summary("refs/remotes/origin/HEAD\torigin/HEAD\tdef5678\t\t ").is_none()
        );
    }
}
