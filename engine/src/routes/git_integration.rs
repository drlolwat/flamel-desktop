use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::AppState;
use crate::db::git_accounts;
use crate::routes::admin::{get_current_user_id, get_user_home};

#[derive(Deserialize)]
pub struct ProjectQuery {
    #[serde(rename = "projectPath")]
    project_path: Option<String>,
    state: Option<String>,
}

struct ResolvedRepo {
    base_url: String,
    token: String,
    account_type: String,
    owner: String,
    repo: String,
}

/// Reads the git remote and matches it to a stored git account.
fn resolve_repo(
    state: &AppState,
    user_id: Option<&str>,
    project_path: &str,
) -> Option<ResolvedRepo> {
    let user_id = user_id?;
    let user_home = get_user_home(state, Some(user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);
    super::path_guard::guard_existing(&access_root, project_path).ok()?;
    let remote =
        crate::git_operations::git_output(project_path, &["remote", "get-url", "origin"]).ok()?;
    let remote = remote.trim();

    // Parse owner/repo from SSH or HTTPS remote URL.
    // Strip trailing .git, then take the last two slash-or-colon-separated segments.
    let trimmed = remote.trim_end_matches(".git");
    let slash = trimmed.rfind('/')?;
    let repo = trimmed[slash + 1..].to_string();
    let before_repo = &trimmed[..slash];
    let sep = before_repo.rfind(|c| c == '/' || c == ':')?;
    let owner = before_repo[sep + 1..].to_string();

    let remote_host = extract_host(remote)?;
    let is_github = remote_host == "github.com";
    let accounts = git_accounts::list_git_accounts_with_tokens(
        &state.db,
        &state.credential_vault,
        Some(user_id),
    );
    let account = accounts.into_iter().find(|a| {
        let account_host = extract_host(&a.base_url);
        (is_github
            && a.account_type == "github"
            && matches!(
                account_host.as_deref(),
                Some("github.com" | "api.github.com")
            ))
            || (a.account_type == "gitea" && account_host.as_deref() == Some(remote_host.as_str()))
    })?;

    Some(ResolvedRepo {
        base_url: account.base_url,
        token: account.token,
        account_type: account.account_type,
        owner,
        repo,
    })
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("flamel")
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_default()
}

struct CloneAskPass {
    path: PathBuf,
}

impl Drop for CloneAskPass {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn extract_host(raw_url: &str) -> Option<String> {
    let trimmed = raw_url.trim();
    if let Some(rest) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
    {
        let authority = rest.split('/').next()?.rsplit('@').next()?;
        let host = authority.split(':').next()?.to_ascii_lowercase();
        return (!host.is_empty()).then_some(host);
    }

    if let Some(rest) = trimmed.strip_prefix("git@") {
        let host = rest
            .split(|c| c == ':' || c == '/')
            .next()?
            .to_ascii_lowercase();
        return (!host.is_empty()).then_some(host);
    }

    None
}

fn select_clone_credentials(
    accounts: &[git_accounts::GitAccount],
    clone_url: &str,
) -> Option<(String, String)> {
    let clone_host = extract_host(clone_url)?;
    let is_github_clone = clone_host == "github.com";

    for account in accounts {
        if account.token.is_empty() || account.token == "***" {
            continue;
        }
        let Some(account_host) = extract_host(&account.base_url) else {
            continue;
        };
        let is_github_account = account.account_type == "github"
            || account_host == "github.com"
            || account_host == "api.github.com";
        if account_host != clone_host && !(is_github_account && is_github_clone) {
            continue;
        }
        let username = if is_github_account {
            "x-access-token"
        } else {
            "oauth2"
        };
        return Some((username.to_string(), account.token.clone()));
    }

    None
}

fn create_token_askpass(username: &str, git_token: &str) -> Option<CloneAskPass> {
    let rand_suffix: u64 = {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64 ^ (d.as_secs() << 32))
            .unwrap_or(42)
    };
    let mut script_path = std::env::temp_dir();
    script_path.push(format!("flamel_git_askpass_{rand_suffix}.sh"));

    let script = format!(
        "#!/bin/sh\ncase \"$1\" in\n  *Username*) printf '%s\\n' {} ;;\n  *Password*) printf '%s\\n' {} ;;\n  *) printf '\\n' ;;\nesac\n",
        shell_single_quote(username),
        shell_single_quote(git_token),
    );
    fs::write(&script_path, script).ok()?;

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&script_path).ok()?.permissions();
        perms.set_mode(0o700);
        fs::set_permissions(&script_path, perms).ok()?;
    }

    Some(CloneAskPass { path: script_path })
}

fn api_url(ctx: &ResolvedRepo, path: &str) -> String {
    if ctx.account_type == "github" {
        format!("https://api.github.com{path}")
    } else {
        format!("{}/api/v1{path}", ctx.base_url)
    }
}

fn map_issue(i: &Value) -> Value {
    json!({
        "number": i["number"],
        "title": i["title"],
        "body": i["body"].as_str().unwrap_or(""),
        "state": i["state"],
        "labels": i["labels"].as_array().map(|ls| ls.iter().map(|l| l["name"].as_str().unwrap_or("")).collect::<Vec<_>>()).unwrap_or_default(),
        "author": i["user"]["login"].as_str().or_else(|| i["user"]["username"].as_str()).unwrap_or(""),
        "htmlUrl": i["html_url"],
        "createdAt": i["created_at"],
    })
}

fn map_pull(p: &Value) -> Value {
    json!({
        "number": p["number"],
        "title": p["title"],
        "body": p["body"].as_str().unwrap_or(""),
        "state": if p["merged_at"].is_string() { "merged" } else { p["state"].as_str().unwrap_or("") },
        "headBranch": p["head"]["ref"].as_str().unwrap_or(""),
        "baseBranch": p["base"]["ref"].as_str().unwrap_or(""),
        "author": p["user"]["login"].as_str().or_else(|| p["user"]["username"].as_str()).unwrap_or(""),
        "htmlUrl": p["html_url"],
        "draft": p["draft"].as_bool().unwrap_or(false),
        "createdAt": p["created_at"],
    })
}

// GET /api/git/issues
pub async fn handle_list_issues(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ProjectQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let Some(project_path) = q.project_path else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "projectPath required"})),
        );
    };
    let user_home = get_user_home(&state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);
    if let Err(error) = super::path_guard::guard_existing(&access_root, &project_path) {
        return error;
    }
    let Some(ctx) = resolve_repo(&state, Some(&user_id), &project_path) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "No git account configured for this repo"})),
        );
    };
    let issue_state = q.state.as_deref().unwrap_or("open");
    let client = http_client();

    let url = if ctx.account_type == "github" {
        format!(
            "{}/repos/{}/{}/issues?state={issue_state}&per_page=50",
            api_url(&ctx, ""),
            ctx.owner,
            ctx.repo
        )
    } else {
        format!(
            "{}/repos/{}/{}/issues?state={issue_state}&limit=50",
            api_url(&ctx, ""),
            ctx.owner,
            ctx.repo
        )
    };

    let res = client.get(&url).bearer_auth(&ctx.token).send().await;
    match res {
        Ok(r) if r.status().is_success() => {
            let data: Vec<Value> = r.json().await.unwrap_or_default();
            (
                StatusCode::OK,
                Json(json!(data.iter().map(map_issue).collect::<Vec<_>>())),
            )
        }
        Ok(r) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": format!("Git API error: {}", r.status())})),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

// GET /api/git/issues/:number
pub async fn handle_get_issue(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(number): Path<u64>,
    Query(q): Query<ProjectQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let Some(project_path) = q.project_path else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "projectPath required"})),
        );
    };
    let Some(ctx) = resolve_repo(&state, Some(&user_id), &project_path) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "No git account configured"})),
        );
    };
    let url = if ctx.account_type == "github" {
        format!(
            "https://api.github.com/repos/{}/{}/issues/{number}",
            ctx.owner, ctx.repo
        )
    } else {
        format!(
            "{}/api/v1/repos/{}/{}/issues/{number}",
            ctx.base_url, ctx.owner, ctx.repo
        )
    };
    let res = http_client().get(&url).bearer_auth(&ctx.token).send().await;
    match res {
        Ok(r) if r.status().is_success() => {
            let data: Value = r.json().await.unwrap_or_default();
            (StatusCode::OK, Json(map_issue(&data)))
        }
        Ok(r) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": format!("Git API error: {}", r.status())})),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

// GET /api/git/pulls
pub async fn handle_list_pulls(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ProjectQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let Some(project_path) = q.project_path else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "projectPath required"})),
        );
    };
    let Some(ctx) = resolve_repo(&state, Some(&user_id), &project_path) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "No git account configured"})),
        );
    };
    let pull_state = q.state.as_deref().unwrap_or("open");
    let url = if ctx.account_type == "github" {
        format!(
            "https://api.github.com/repos/{}/{}/pulls?state={pull_state}&per_page=50",
            ctx.owner, ctx.repo
        )
    } else {
        format!(
            "{}/api/v1/repos/{}/{}/pulls?state={pull_state}&limit=50",
            ctx.base_url, ctx.owner, ctx.repo
        )
    };
    let res = http_client().get(&url).bearer_auth(&ctx.token).send().await;
    match res {
        Ok(r) if r.status().is_success() => {
            let data: Vec<Value> = r.json().await.unwrap_or_default();
            (
                StatusCode::OK,
                Json(json!(data.iter().map(map_pull).collect::<Vec<_>>())),
            )
        }
        Ok(r) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": format!("Git API error: {}", r.status())})),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

// GET /api/git/pulls/:number
pub async fn handle_get_pull(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(number): Path<u64>,
    Query(q): Query<ProjectQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let Some(project_path) = q.project_path else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "projectPath required"})),
        );
    };
    let Some(ctx) = resolve_repo(&state, Some(&user_id), &project_path) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "No git account configured"})),
        );
    };
    let url = if ctx.account_type == "github" {
        format!(
            "https://api.github.com/repos/{}/{}/pulls/{number}",
            ctx.owner, ctx.repo
        )
    } else {
        format!(
            "{}/api/v1/repos/{}/{}/pulls/{number}",
            ctx.base_url, ctx.owner, ctx.repo
        )
    };
    let res = http_client().get(&url).bearer_auth(&ctx.token).send().await;
    match res {
        Ok(r) if r.status().is_success() => {
            let data: Value = r.json().await.unwrap_or_default();
            (StatusCode::OK, Json(map_pull(&data)))
        }
        Ok(r) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": format!("Git API error: {}", r.status())})),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

// POST /api/git/pulls
pub async fn handle_create_pull(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let project_path = match body.get("projectPath").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "projectPath required"})),
            );
        }
    };
    let title = match body.get("title").and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "title required"})),
            );
        }
    };
    let Some(ctx) = resolve_repo(&state, Some(&user_id), &project_path) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "No git account configured"})),
        );
    };

    // Auto-push branch
    let _ = std::process::Command::new("git")
        .args(["push", "-u", "origin", "HEAD"])
        .current_dir(&project_path)
        .output();

    let description = body
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let head_branch = body
        .get("headBranch")
        .and_then(|v| v.as_str())
        .unwrap_or("HEAD");
    let base_branch = body
        .get("baseBranch")
        .and_then(|v| v.as_str())
        .unwrap_or("main");
    let draft = body.get("draft").and_then(|v| v.as_bool()).unwrap_or(false);

    let payload = json!({
        "title": title,
        "body": description,
        "head": head_branch,
        "base": base_branch,
        "draft": draft,
    });

    let url = if ctx.account_type == "github" {
        format!(
            "https://api.github.com/repos/{}/{}/pulls",
            ctx.owner, ctx.repo
        )
    } else {
        format!(
            "{}/api/v1/repos/{}/{}/pulls",
            ctx.base_url, ctx.owner, ctx.repo
        )
    };

    let res = http_client()
        .post(&url)
        .bearer_auth(&ctx.token)
        .json(&payload)
        .send()
        .await;
    match res {
        Ok(r) if r.status().is_success() => {
            let data: Value = r.json().await.unwrap_or_default();
            (StatusCode::CREATED, Json(map_pull(&data)))
        }
        Ok(r) => {
            let status = r.status().as_u16();
            let err = r.text().await.unwrap_or_default();
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": format!("Git API error ({status}): {err}")})),
            )
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

// GET /api/git/branch
pub async fn handle_get_current_branch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ProjectQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let Some(project_path) = q.project_path else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "projectPath required"})),
        );
    };
    let user_home = get_user_home(&state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);
    if let Err(error) = super::path_guard::guard_existing(&access_root, &project_path) {
        return error;
    }
    let branch = match crate::git_operations::git_output(
        &project_path,
        &["rev-parse", "--abbrev-ref", "HEAD"],
    ) {
        Ok(branch) => branch.trim().to_owned(),
        Err(error) => return (StatusCode::CONFLICT, Json(json!({"error": error}))),
    };

    let resolved = resolve_repo(&state, Some(&user_id), &project_path);
    (
        StatusCode::OK,
        Json(json!({
            "branch": branch,
            "owner": resolved.as_ref().map(|r| &r.owner),
            "repo": resolved.as_ref().map(|r| &r.repo),
            "hasAccount": resolved.is_some(),
        })),
    )
}

// POST /api/git/clone — clone a repo into the user's home directory
pub async fn handle_clone_repo(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let user_home = get_user_home(&state, Some(&user_id));

    let clone_url = match body.get("cloneUrl").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "cloneUrl required"})),
            );
        }
    };
    let repo_name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "name required"})),
            );
        }
    };

    let target = user_home.join(&repo_name);
    if target.exists() {
        // Already cloned — just return the path
        return (
            StatusCode::OK,
            Json(json!({
                "path": target.to_string_lossy(),
                "alreadyExists": true,
            })),
        );
    }

    let flamel_host = extract_host(&crate::routes::flamel_git::forge_url());
    let clone_host = extract_host(&clone_url);
    let flamel_repository_id = (flamel_host.is_some() && flamel_host == clone_host)
        .then(|| {
            body.get("repositoryId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .flatten();
    let askpass = if flamel_host.is_some() && flamel_host == clone_host {
        let Some(repository_id) = flamel_repository_id.as_deref() else {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"repositoryId is required for Flamel Git"})),
            );
        };
        match crate::routes::flamel_git::repository_token(
            &state,
            &headers,
            &json!({"repository_id":repository_id,"scopes":["repo:read"]}),
        )
        .await
        {
            Ok(response) => response
                .get("token")
                .and_then(Value::as_str)
                .and_then(|token| create_token_askpass("flamel", token)),
            Err((status, error)) => return (status, Json(json!({"error":error}))),
        }
    } else {
        let accounts = git_accounts::list_git_accounts_with_tokens(
            &state.db,
            &state.credential_vault,
            Some(&user_id),
        );
        select_clone_credentials(&accounts, &clone_url)
            .and_then(|(username, token)| create_token_askpass(&username, &token))
    };

    let target_path = target.to_string_lossy().to_string();
    let mut command = tokio::process::Command::new("git");
    command
        .args(["clone", "--depth", "1", &clone_url, &target_path])
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(ref askpass) = askpass {
        command.env("GIT_ASKPASS", &askpass.path);
    }

    let output = tokio::time::timeout(Duration::from_secs(120), command.output()).await;

    match output {
        Ok(Ok(out)) if out.status.success() => {
            let mut response = json!({
                "path": target.to_string_lossy(),
                "alreadyExists": false,
            });
            if let Some(repository_id) = flamel_repository_id.as_deref() {
                let project_key = hex::encode(Sha256::digest(target_path.as_bytes()));
                match crate::routes::flamel_git::link_project(
                    &state,
                    &headers,
                    repository_id,
                    &json!({
                        "project_key":project_key,
                        "project_name":repo_name,
                        "actions_enabled":false,
                    }),
                )
                .await
                {
                    Ok(_) => response["forgeLinked"] = Value::Bool(true),
                    Err((_, error)) => {
                        tracing::warn!(%error, "cloned Flamel Git repository but could not link project");
                        response["forgeLinked"] = Value::Bool(false);
                        response["warning"] = Value::String(error);
                    }
                }
            }
            (StatusCode::OK, Json(response))
        }
        Ok(Ok(out)) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            tracing::warn!("git clone failed: {stderr}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("git clone failed: {stderr}")})),
            )
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("failed to run git: {e}")})),
        ),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(json!({"error": "git clone timed out"})),
        ),
    }
}
