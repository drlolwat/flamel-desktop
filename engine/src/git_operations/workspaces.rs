//! Additional active coding sessions get their own checkout, not copies of dirty files.
use super::{MutationGuard, command_output, git_output};
use crate::db::Db;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionWorkspace {
    pub source_path: String,
    pub project_path: String,
    pub branch: Option<String>,
    pub base_sha: Option<String>,
    pub base_branch: Option<String>,
    pub isolated: bool,
}

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    // A reservation can precede chat admission, so failed/retried admission keeps
    // the same checkout. Never delete a user's worktree when a chat is deleted.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_workspaces (
            session_id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            common_dir TEXT NOT NULL,
            metadata_json TEXT NOT NULL
         );",
    )
}

pub fn load(db: &Db, session_id: &str, user_id: &str) -> Result<Option<SessionWorkspace>, String> {
    let value: Option<String> = db
        .conn()
        .query_row(
            "SELECT metadata_json FROM session_workspaces WHERE session_id=?1 AND user_id=?2",
            params![session_id, user_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    value
        .map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
        .transpose()
}

pub struct PreparedWorkspace {
    pub path: String,
    pub workspace: Option<SessionWorkspace>,
    // Hold until the caller has durably admitted the run, not merely until Git exits.
    pub _guard: Option<MutationGuard>,
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "Workspace paths must be valid UTF-8".into())
}

fn canonical(path: &str) -> Result<String, String> {
    path_text(
        &std::fs::canonicalize(path).map_err(|error| format!("Workspace unavailable: {error}"))?,
    )
}

fn common_dir(path: &str) -> Result<Option<String>, String> {
    let result = command_output(
        "git",
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        path,
        &[("LC_ALL", "C")],
        None,
        Duration::from_secs(5),
    )?;
    if result.code == 0 {
        canonical(result.stdout.strip_suffix('\n').unwrap_or(&result.stdout)).map(Some)
    } else if result.stderr.contains("not a git repository") {
        Ok(None)
    } else {
        Err(result.stderr)
    }
}

fn active_paths(db: &Db, session_id: &str) -> Result<Vec<(String, Option<String>)>, String> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT DISTINCT s.project_path, w.common_dir FROM sessions s
         LEFT JOIN session_workspaces w ON w.session_id=s.id
         WHERE s.id<>?1 AND EXISTS (
           SELECT 1 FROM chat_runs r WHERE r.session_id=s.id
             AND r.state IN ('preparing','running','recovering','queued','waiting_input','waiting_approval')
         )",
    ).map_err(|error| error.to_string())?;
    stmt.query_map([session_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| error.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())
}

/// The caller has already authenticated the session and checked the source path.
/// This runs on a blocking thread; Git and file-lock waits never block Tokio.
pub fn prepare(
    db: &Db,
    session_id: &str,
    user_id: &str,
    requested: &str,
    user_home: &Path,
    remote_execution: bool,
) -> Result<PreparedWorkspace, String> {
    let requested = canonical(requested)?;
    let reserved_owner: Option<String> = db
        .conn()
        .query_row(
            "SELECT user_id FROM session_workspaces WHERE session_id=?1",
            [session_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if reserved_owner
        .as_deref()
        .is_some_and(|owner| owner != user_id)
    {
        return Err("This session belongs to a different user".into());
    }
    let saved = load(db, session_id, user_id)?;
    let existing = crate::db::sessions::get_session(db, session_id);
    if existing
        .as_ref()
        .is_some_and(|session| session.user_id.as_deref() != Some(user_id))
    {
        return Err("This session belongs to a different user".into());
    }
    let effective = if let Some(workspace) = &saved {
        if workspace.isolated && remote_execution {
            return Err("This separate workspace runs locally. Remote worker workspace support is not available yet".into());
        }
        if requested != workspace.source_path && requested != canonical(&workspace.project_path)? {
            return Err("This session already owns another workspace; start a new session to change folders".into());
        }
        canonical(&workspace.project_path)?
    } else if let Some(existing) = &existing {
        let owned = canonical(&existing.project_path)?;
        if requested != owned {
            return Err(
                "This session already owns another folder; start a new session to change folders"
                    .into(),
            );
        }
        owned
    } else {
        requested.clone()
    };
    let Some(common) = common_dir(&effective)? else {
        return Ok(PreparedWorkspace {
            path: effective,
            workspace: saved,
            _guard: None,
        });
    };
    let guard = MutationGuard::acquire_wait(&effective, Duration::from_secs(15))?;
    // A simultaneous retry may have created the reservation while we waited.
    if let Some(workspace) = load(db, session_id, user_id)? {
        if workspace.isolated && remote_execution {
            return Err("This separate workspace runs locally. Remote worker workspace support is not available yet".into());
        }
        return Ok(PreparedWorkspace {
            path: workspace.project_path.clone(),
            workspace: Some(workspace),
            _guard: Some(guard),
        });
    }
    let mut concurrent = false;
    for (path, recorded_common) in active_paths(db, session_id)? {
        if recorded_common.as_deref() == Some(&common)
            || (recorded_common.is_none()
                && Path::new(&path).exists()
                && common_dir(&path)?.as_deref() == Some(&common))
        {
            concurrent = true;
            break;
        }
    }
    // Never relocate a resumed session or its uncommitted work. This also leaves
    // existing shared sessions alone; isolation is a default for new sessions.
    let isolate = concurrent && existing.is_none();
    if isolate && remote_execution {
        return Err("Another session is using this repository. Separate workspaces are not yet supported on remote workers; choose local execution or wait for that work to finish".into());
    }
    let top = git_output(&effective, &["rev-parse", "--show-toplevel"])?;
    let top = canonical(top.strip_suffix('\n').unwrap_or(&top))?;
    let relative = Path::new(&effective)
        .strip_prefix(&top)
        .map_err(|error| error.to_string())?;
    let base_sha = git_output(&effective, &["rev-parse", "--verify", "HEAD"])
        .ok()
        .map(|sha| sha.trim().to_owned());
    let base_branch = git_output(&effective, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .map(|branch| branch.trim().to_owned());
    let mut workspace = SessionWorkspace {
        source_path: effective.clone(),
        project_path: effective.clone(),
        branch: base_branch.clone(),
        base_sha: base_sha.clone(),
        base_branch,
        isolated: isolate,
    };
    if isolate {
        let sha = base_sha.ok_or("This repository needs its first commit before a separate workspace can be created. Finish the other session first; no files were moved")?;
        let root = user_home.join(".local/share/flamel/workspaces");
        let existing_parent = root
            .ancestors()
            .find(|path| path.exists())
            .ok_or("Workspace parent is unavailable")?;
        let owned_home = std::fs::canonicalize(user_home).map_err(|error| error.to_string())?;
        if !std::fs::canonicalize(existing_parent)
            .map_err(|error| error.to_string())?
            .starts_with(&owned_home)
        {
            return Err("Workspace directory escapes the user's home".into());
        }
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&root).map_err(|error| error.to_string())?;
        let key = format!(
            "{:x}",
            Sha256::digest(format!("{user_id}\0{session_id}").as_bytes())
        );
        let destination = root.join(&key[..32]);
        if destination.exists() {
            return Err(format!(
                "Workspace path already exists and was left untouched: {}",
                destination.display()
            ));
        }
        let branch = format!("flamel/session-{}", &key[..16]);
        let destination_text = path_text(&destination)?;
        git_output(&effective, &["worktree", "add", "-b", &branch, "--", &destination_text, &sha])
            .map_err(|error| format!("Could not create the separate workspace; the original folder was not changed: {error}"))?;
        workspace.project_path = canonical(&path_text(&destination.join(relative))?)?;
        workspace.branch = Some(branch);
    }
    db.conn().execute(
        "INSERT INTO session_workspaces (session_id,user_id,common_dir,metadata_json) VALUES (?1,?2,?3,?4)",
        params![session_id, user_id, common, serde_json::to_string(&workspace).map_err(|error| error.to_string())?],
    ).map_err(|error| format!("Could not save workspace ownership; any new worktree was retained for recovery: {error}"))?;
    Ok(PreparedWorkspace {
        path: workspace.project_path.clone(),
        workspace: Some(workspace),
        _guard: Some(guard),
    })
}

pub fn review(db: &Db, session_id: &str, user_id: &str) -> Result<serde_json::Value, String> {
    let workspace = load(db, session_id, user_id)?
        .filter(|workspace| workspace.isolated)
        .ok_or("No separate workspace for this session")?;
    let base = workspace
        .base_sha
        .as_deref()
        .ok_or("Workspace has no starting revision")?;
    let diff = git_output(
        &workspace.project_path,
        &["diff", "--no-ext-diff", "--stat", base, "HEAD", "--"],
    )?;
    let patch = git_output(
        &workspace.project_path,
        &["diff", "--no-ext-diff", base, "HEAD", "--"],
    )?;
    let uncommitted = git_output(&workspace.project_path, &["status", "--porcelain=v1", "-z"])?;
    Ok(
        serde_json::json!({"workspace": workspace, "summary": diff, "diff": patch, "hasUncommittedChanges": !uncommitted.is_empty()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};

    struct TestDirectory(std::path::PathBuf);
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(committed: bool) -> (TestDirectory, Arc<Db>, String, std::path::PathBuf) {
        let temp = TestDirectory(
            std::env::temp_dir().join(format!("flamel-workspace-test-{}", uuid::Uuid::new_v4())),
        );
        let repo = temp.0.join("repo");
        let home = temp.0.join("home");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir(&home).unwrap();
        let repo = path_text(&std::fs::canonicalize(&repo).unwrap()).unwrap();
        git_output(&repo, &["init", "--initial-branch=main"]).unwrap();
        git_output(&repo, &["config", "user.name", "Test"]).unwrap();
        git_output(&repo, &["config", "user.email", "test@example.invalid"]).unwrap();
        git_output(&repo, &["config", "core.autocrlf", "false"]).unwrap();
        if committed {
            std::fs::write(Path::new(&repo).join("src/code.txt"), "saved version\n").unwrap();
            std::fs::write(Path::new(&repo).join(".gitignore"), ".env\n").unwrap();
            git_output(&repo, &["add", "."]).unwrap();
            git_output(&repo, &["commit", "-m", "Initial"]).unwrap();
        }
        (temp, Arc::new(Db::open(":memory:")), repo, home)
    }

    fn admit(db: &Db, session: &str, path: &str, state: &str) {
        db.conn().execute(
            "INSERT INTO sessions(id,project_path,user_id,created_at,updated_at) VALUES (?1,?2,'user','now','now')",
            params![session, path],
        ).unwrap();
        assert!(crate::db::realtime::create_run(
            db,
            &format!("run-{session}"),
            session,
            "user",
            &serde_json::json!({"workingDirectory": path}),
            state
        ));
    }

    #[test]
    fn additional_active_session_keeps_dirty_original_and_secrets_untouched() {
        let (_temp, db, repo, home) = fixture(true);
        std::fs::write(Path::new(&repo).join("src/code.txt"), "unsaved work\n").unwrap();
        std::fs::write(Path::new(&repo).join("local-notes.txt"), "untracked work").unwrap();
        std::fs::write(Path::new(&repo).join(".env"), "PRIVATE_CONFIG=local").unwrap();
        let status = git_output(&repo, &["status", "--porcelain=v1", "-z"]).unwrap();
        let first = prepare(&db, "first", "user", &repo, &home, false).unwrap();
        assert_eq!(first.path, repo);
        assert!(!first.workspace.as_ref().unwrap().isolated);
        admit(&db, "first", &first.path, "preparing");
        drop(first);
        let second = prepare(&db, "second", "user", &repo, &home, false).unwrap();
        let workspace = second.workspace.as_ref().unwrap();
        assert!(workspace.isolated);
        assert_ne!(second.path, repo);
        assert_eq!(
            std::fs::read_to_string(Path::new(&second.path).join("src/code.txt")).unwrap(),
            "saved version\n"
        );
        assert!(!Path::new(&second.path).join(".env").exists());
        assert!(!Path::new(&second.path).join("local-notes.txt").exists());
        assert_eq!(
            git_output(&repo, &["status", "--porcelain=v1", "-z"]).unwrap(),
            status
        );
        assert_eq!(
            git_output(&repo, &["branch", "--show-current"])
                .unwrap()
                .trim(),
            "main"
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&repo).join(".env")).unwrap(),
            "PRIVATE_CONFIG=local"
        );
        assert!(
            MutationGuard::acquire(&second.path).is_err(),
            "sibling worktrees share the admission lock"
        );
        let owned_path = second.path.clone();
        drop(second);
        // A lost admission response or an old tab can retry with either path.
        for request_path in [&repo, &owned_path] {
            let retried = prepare(&db, "second", "user", request_path, &home, false).unwrap();
            assert_eq!(retried.path, owned_path);
        }
        assert!(prepare(&db, "second", "other-user", &repo, &home, false).is_err());
    }

    #[test]
    fn simultaneous_first_sends_wait_until_the_other_run_is_admitted() {
        let (_temp, db, repo, home) = fixture(true);
        let first = prepare(&db, "first", "user", &repo, &home, false).unwrap();
        let (tx, rx) = mpsc::channel();
        let thread_db = Arc::clone(&db);
        let thread_repo = repo.clone();
        let join = std::thread::spawn(move || {
            let next = prepare(&thread_db, "second", "user", &thread_repo, &home, false).unwrap();
            tx.send(next.workspace.unwrap().isolated).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        admit(&db, "first", &first.path, "preparing");
        drop(first);
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap());
        join.join().unwrap();
    }

    #[test]
    fn completed_or_idle_sessions_do_not_force_isolation() {
        let (_temp, db, repo, home) = fixture(true);
        admit(&db, "old", &repo, "completed");
        let next = prepare(&db, "new", "user", &repo, &home, false).unwrap();
        assert_eq!(next.path, repo);
        assert!(!next.workspace.unwrap().isolated);
    }

    #[test]
    fn existing_sessions_are_never_moved_and_non_git_folders_still_work() {
        let (_temp, db, repo, home) = fixture(true);
        admit(&db, "existing", &repo, "completed");
        admit(&db, "other", &repo, "running");
        let resumed = prepare(&db, "existing", "user", &repo, &home, false).unwrap();
        assert_eq!(resumed.path, repo);
        drop(resumed);
        let plain = prepare(
            &db,
            "plain",
            "user",
            &path_text(&home).unwrap(),
            &home,
            false,
        )
        .unwrap();
        assert!(plain.workspace.is_none());
        assert!(plain._guard.is_none());
    }

    #[test]
    fn unsupported_additional_workspaces_fail_without_sharing() {
        let (_temp, db, repo, home) = fixture(false);
        admit(&db, "first", &repo, "running");
        let error = prepare(&db, "second", "user", &repo, &home, false)
            .err()
            .unwrap();
        assert!(error.contains("first commit"), "{error}");
        let error = prepare(&db, "remote", "user", &repo, &home, true)
            .err()
            .unwrap();
        assert!(error.contains("remote workers"), "{error}");
        assert!(load(&db, "second", "user").unwrap().is_none());
    }

    #[test]
    fn review_back_shows_saved_commits_without_touching_original() {
        let (_temp, db, repo, home) = fixture(true);
        admit(&db, "first", &repo, "waiting_input");
        let second = prepare(&db, "second", "user", &repo, &home, false).unwrap();
        let path = second.path.clone();
        drop(second);
        std::fs::write(Path::new(&path).join("src/code.txt"), "improved version\n").unwrap();
        git_output(&path, &["add", "."]).unwrap();
        git_output(&path, &["commit", "-m", "Improvement"]).unwrap();
        let review = review(&db, "second", "user").unwrap();
        assert!(
            review["diff"]
                .as_str()
                .unwrap()
                .contains("+improved version")
        );
        assert_eq!(review["hasUncommittedChanges"], false);
        assert_eq!(
            std::fs::read_to_string(Path::new(&repo).join("src/code.txt")).unwrap(),
            "saved version\n"
        );
        assert!(super::review(&db, "second", "other-user").is_err());
    }

    #[test]
    fn first_workspace_publish_sets_tracking_for_only_its_branch() {
        let (temp, db, repo, home) = fixture(true);
        let remote = temp.0.join("remote.git");
        git_output(&repo, &["init", "--bare", &path_text(&remote).unwrap()]).unwrap();
        git_output(
            &repo,
            &["remote", "add", "origin", &path_text(&remote).unwrap()],
        )
        .unwrap();
        admit(&db, "first", &repo, "running");
        let second = prepare(&db, "second", "user", &repo, &home, false).unwrap();
        let branch = second
            .workspace
            .as_ref()
            .unwrap()
            .branch
            .as_deref()
            .unwrap();
        assert!(crate::git_operations::branch_needs_upstream(&second.path, branch).unwrap());
        git_output(&second.path, &["push", "--set-upstream", "origin", branch]).unwrap();
        assert!(!crate::git_operations::branch_needs_upstream(&second.path, branch).unwrap());
        assert!(
            git_output(
                &path_text(&remote).unwrap(),
                &["show-ref", "--verify", "refs/heads/main"]
            )
            .is_err()
        );
    }
}
