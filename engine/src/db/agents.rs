use crate::db::Db;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    pub id: String,
    pub user_id: String,
    pub git_account_id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub trigger_event: String,
    pub trigger_label: Option<String>,
    pub project_path: String,
    pub model: String,
    pub permission_mode: String,
    pub auto_pr: bool,
    pub auto_close: bool,
    pub comment_on_start: bool,
    pub require_approval: bool,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRun {
    pub id: String,
    pub agent_config_id: String,
    pub session_id: Option<String>,
    pub status: String,
    pub trigger_payload: serde_json::Value,
    pub pr_url: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn row_to_config(row: &rusqlite::Row) -> rusqlite::Result<AgentConfig> {
    Ok(AgentConfig {
        id: row.get("id")?,
        user_id: row.get::<_, Option<String>>("user_id")?.unwrap_or_default(),
        git_account_id: row.get("git_account_id")?,
        repo_owner: row.get("repo_owner")?,
        repo_name: row.get("repo_name")?,
        trigger_event: row.get("trigger_event")?,
        trigger_label: row.get("trigger_label")?,
        project_path: row.get("project_path")?,
        model: row.get("model")?,
        permission_mode: row.get("permission_mode")?,
        auto_pr: row.get::<_, i64>("auto_pr")? != 0,
        auto_close: row.get::<_, Option<i64>>("auto_close")?.unwrap_or(1) != 0,
        comment_on_start: row.get::<_, Option<i64>>("comment_on_start")?.unwrap_or(1) != 0,
        require_approval: row.get::<_, i64>("require_approval")? != 0,
        enabled: row.get::<_, i64>("enabled")? != 0,
        created_at: row.get("created_at")?,
    })
}

fn row_to_run(row: &rusqlite::Row) -> rusqlite::Result<AgentRun> {
    let payload_str: String = row.get("trigger_payload")?;
    let trigger_payload = serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);
    Ok(AgentRun {
        id: row.get("id")?,
        agent_config_id: row.get("agent_config_id")?,
        session_id: row.get("session_id")?,
        status: row.get("status")?,
        trigger_payload,
        pr_url: row.get("pr_url")?,
        error: row.get("error")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

// Agent configs
pub fn list_agent_configs(db: &Db, user_id: Option<&str>) -> Vec<AgentConfig> {
    let conn = db.conn();
    if let Some(uid) = user_id {
        let mut stmt = conn
            .prepare("SELECT * FROM agent_configs WHERE user_id = ?1 ORDER BY created_at DESC")
            .unwrap();
        stmt.query_map([uid], row_to_config)
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    } else {
        let mut stmt = conn
            .prepare("SELECT * FROM agent_configs ORDER BY created_at DESC")
            .unwrap();
        stmt.query_map([], row_to_config)
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }
}

pub fn get_agent_config(db: &Db, id: &str) -> Option<AgentConfig> {
    let conn = db.conn();
    conn.query_row(
        "SELECT * FROM agent_configs WHERE id = ?1",
        [id],
        row_to_config,
    )
    .ok()
}

pub fn get_agent_config_owned(db: &Db, id: &str, user_id: &str) -> Option<AgentConfig> {
    let conn = db.conn();
    conn.query_row(
        "SELECT * FROM agent_configs WHERE id = ?1 AND user_id = ?2",
        [id, user_id],
        row_to_config,
    )
    .ok()
}

pub fn create_agent_config(
    db: &Db,
    user_id: &str,
    git_account_id: &str,
    repo_owner: &str,
    repo_name: &str,
    trigger_event: &str,
    trigger_label: Option<&str>,
    project_path: &str,
    model: &str,
    permission_mode: &str,
    auto_pr: bool,
    auto_close: bool,
    comment_on_start: bool,
    require_approval: bool,
) -> AgentConfig {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO agent_configs (id, git_account_id, repo_owner, repo_name, trigger_event, trigger_label, project_path, model, permission_mode, auto_pr, auto_close, comment_on_start, require_approval, enabled, created_at, user_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 1, ?14, ?15)",
        rusqlite::params![
            id, git_account_id, repo_owner, repo_name,
            trigger_event, trigger_label, project_path, model,
            permission_mode, auto_pr as i64, auto_close as i64, comment_on_start as i64,
            require_approval as i64, now, user_id
        ],
    )
    .unwrap();
    get_agent_config(db, &id).unwrap()
}

pub fn update_agent_config(db: &Db, id: &str, body: &serde_json::Value) -> Option<AgentConfig> {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(v) = body.get("enabled").and_then(|v| v.as_bool()) {
        conn.execute(
            "UPDATE agent_configs SET enabled = ?1 WHERE id = ?2",
            rusqlite::params![v as i64, id],
        )
        .ok();
    }
    if let Some(v) = body.get("requireApproval").and_then(|v| v.as_bool()) {
        conn.execute(
            "UPDATE agent_configs SET require_approval = ?1 WHERE id = ?2",
            rusqlite::params![v as i64, id],
        )
        .ok();
    }
    if let Some(v) = body.get("autoPr").and_then(|v| v.as_bool()) {
        conn.execute(
            "UPDATE agent_configs SET auto_pr = ?1 WHERE id = ?2",
            rusqlite::params![v as i64, id],
        )
        .ok();
    }
    if let Some(v) = body.get("autoClose").and_then(|v| v.as_bool()) {
        conn.execute(
            "UPDATE agent_configs SET auto_close = ?1 WHERE id = ?2",
            rusqlite::params![v as i64, id],
        )
        .ok();
    }
    if let Some(v) = body.get("commentOnStart").and_then(|v| v.as_bool()) {
        conn.execute(
            "UPDATE agent_configs SET comment_on_start = ?1 WHERE id = ?2",
            rusqlite::params![v as i64, id],
        )
        .ok();
    }
    if let Some(v) = body.get("model").and_then(|v| v.as_str()) {
        conn.execute(
            "UPDATE agent_configs SET model = ?1 WHERE id = ?2",
            rusqlite::params![v, id],
        )
        .ok();
    }
    if let Some(v) = body.get("triggerLabel") {
        let label: Option<&str> = v.as_str();
        conn.execute(
            "UPDATE agent_configs SET trigger_label = ?1 WHERE id = ?2",
            rusqlite::params![label, id],
        )
        .ok();
    }
    conn.execute(
        "UPDATE agent_configs SET created_at = created_at WHERE id = ?1",
        [id],
    )
    .ok(); // touch
    let _ = now; // suppress unused warning
    get_agent_config(db, id)
}

pub fn delete_agent_config(db: &Db, id: &str) {
    let conn = db.conn();
    conn.execute("DELETE FROM agent_configs WHERE id = ?1", [id])
        .ok();
}

pub fn delete_agent_config_owned(db: &Db, id: &str, user_id: &str) -> bool {
    let conn = db.conn();
    conn.execute(
        "DELETE FROM agent_configs WHERE id = ?1 AND user_id = ?2",
        [id, user_id],
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

// Agent runs
pub fn list_agent_runs(db: &Db, config_id: Option<&str>, user_id: Option<&str>) -> Vec<AgentRun> {
    let conn = db.conn();
    match (config_id, user_id) {
        (Some(cid), Some(uid)) => {
            let mut stmt = conn
                .prepare(
                    "SELECT ar.* FROM agent_runs ar
                 JOIN agent_configs ac ON ac.id = ar.agent_config_id
                 WHERE ar.agent_config_id = ?1 AND ac.user_id = ?2
                 ORDER BY ar.created_at DESC LIMIT 50",
                )
                .unwrap();
            stmt.query_map([cid, uid], row_to_run)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
        (Some(cid), None) => {
            let mut stmt = conn.prepare(
                "SELECT * FROM agent_runs WHERE agent_config_id = ?1 ORDER BY created_at DESC LIMIT 50"
            ).unwrap();
            stmt.query_map([cid], row_to_run)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
        (None, Some(uid)) => {
            let mut stmt = conn
                .prepare(
                    "SELECT ar.* FROM agent_runs ar
                 JOIN agent_configs ac ON ac.id = ar.agent_config_id
                 WHERE ac.user_id = ?1
                 ORDER BY ar.created_at DESC LIMIT 50",
                )
                .unwrap();
            stmt.query_map([uid], row_to_run)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
        (None, None) => {
            let mut stmt = conn
                .prepare("SELECT * FROM agent_runs ORDER BY created_at DESC LIMIT 50")
                .unwrap();
            stmt.query_map([], row_to_run)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
    }
}

pub fn get_agent_run(db: &Db, id: &str) -> Option<AgentRun> {
    let conn = db.conn();
    conn.query_row("SELECT * FROM agent_runs WHERE id = ?1", [id], row_to_run)
        .ok()
}

pub fn get_agent_run_owned(db: &Db, id: &str, user_id: &str) -> Option<AgentRun> {
    let conn = db.conn();
    conn.query_row(
        "SELECT ar.* FROM agent_runs ar
         JOIN agent_configs ac ON ac.id = ar.agent_config_id
         WHERE ar.id = ?1 AND ac.user_id = ?2",
        [id, user_id],
        row_to_run,
    )
    .ok()
}

pub fn create_agent_run(db: &Db, config_id: &str, payload: &str, status: &str) -> AgentRun {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO agent_runs (id, agent_config_id, status, trigger_payload, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        rusqlite::params![id, config_id, status, payload, now],
    )
    .unwrap();
    get_agent_run(db, &id).unwrap()
}

pub fn update_agent_run_status(db: &Db, id: &str, status: &str, error: Option<&str>) {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agent_runs SET status = ?1, error = ?2, updated_at = ?3 WHERE id = ?4",
        rusqlite::params![status, error, now, id],
    )
    .ok();
}

/// Find all pending (ready-to-run) agent runs
pub fn find_pending_runs(db: &Db) -> Vec<AgentRun> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT * FROM agent_runs WHERE status = 'pending' ORDER BY created_at ASC LIMIT 10",
        )
        .unwrap();
    stmt.query_map([], row_to_run)
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
}

/// Update run status and optionally set the PR URL
pub fn update_agent_run_with_pr(db: &Db, id: &str, status: &str, pr_url: Option<&str>) {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agent_runs SET status = ?1, pr_url = ?2, updated_at = ?3 WHERE id = ?4",
        rusqlite::params![status, pr_url, now, id],
    )
    .ok();
}

/// Mark a run as running (atomically checks it's still pending to avoid double-execution)
pub fn try_claim_run(db: &Db, id: &str) -> bool {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agent_runs SET status = 'running', updated_at = ?1 WHERE id = ?2 AND status = 'pending'",
        rusqlite::params![now, id],
    ).map(|n| n > 0).unwrap_or(false)
}

/// Return all enabled configs that have polling enabled (all enabled configs, used by poller).
pub fn list_enabled_configs_with_tokens(db: &Db) -> Vec<(AgentConfig, String)> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT ac.*, ga.token as _token FROM agent_configs ac
         JOIN git_accounts ga ON ga.id = ac.git_account_id
         WHERE ac.enabled = 1",
        )
        .unwrap();
    stmt.query_map([], |row| {
        let config = row_to_config(row)?;
        let token: String = row.get("_token")?;
        Ok((config, token))
    })
    .unwrap()
    .filter_map(|r| r.ok())
    .collect()
}

/// Check if we've already processed this issue for this config.
pub fn is_issue_seen(db: &Db, config_id: &str, issue_number: i64) -> bool {
    let conn = db.conn();
    conn.query_row(
        "SELECT 1 FROM seen_agent_issues WHERE agent_config_id = ?1 AND issue_number = ?2",
        rusqlite::params![config_id, issue_number],
        |_| Ok(()),
    )
    .is_ok()
}

/// Mark an issue as seen so we don't trigger on it again.
pub fn mark_issue_seen(db: &Db, config_id: &str, issue_number: i64) {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO seen_agent_issues (agent_config_id, issue_number, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![config_id, issue_number, now],
    ).ok();
}

/// Update the last_polled_at timestamp on a config.
pub fn update_last_polled_at(db: &Db, config_id: &str) {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agent_configs SET last_polled_at = ?1 WHERE id = ?2",
        rusqlite::params![now, config_id],
    )
    .ok();
}

/// Find all enabled agent configs matching a webhook payload
pub fn find_matching_configs(
    db: &Db,
    repo_owner: &str,
    repo_name: &str,
    event: &str,
) -> Vec<(AgentConfig, String)> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT ac.*, ga.token as _token FROM agent_configs ac
         JOIN git_accounts ga ON ga.id = ac.git_account_id
         WHERE ac.enabled = 1
           AND lower(ac.repo_owner) = lower(?1)
           AND lower(ac.repo_name) = lower(?2)
           AND ac.trigger_event = ?3",
        )
        .unwrap();
    let rows: Vec<(AgentConfig, String)> = stmt
        .query_map(rusqlite::params![repo_owner, repo_name, event], |row| {
            let config = row_to_config(row)?;
            let token: String = row.get("_token")?;
            Ok((config, token))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    rows
}
