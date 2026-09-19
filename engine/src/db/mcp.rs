use crate::db::Db;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: String,
    pub project_path: Option<String>,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub enabled: bool,
    pub is_builtin: bool,
}

pub fn list_mcp_configs(
    db: &Db,
    project_path: Option<&str>,
    user_id: Option<&str>,
) -> Vec<McpServerConfig> {
    let conn = db.conn();
    let rows: Vec<McpServerConfig> = match (project_path, user_id) {
        (Some(pp), Some(uid)) => {
            let mut stmt = conn
                .prepare("SELECT * FROM mcp_configs WHERE (project_path IS NULL OR project_path = ?1) AND user_id = ?2 ORDER BY name")
                .unwrap();
            stmt.query_map(rusqlite::params![pp, uid], row_to_config)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
        (Some(pp), None) => {
            let mut stmt = conn
                .prepare("SELECT * FROM mcp_configs WHERE project_path IS NULL OR project_path = ?1 ORDER BY name")
                .unwrap();
            stmt.query_map([pp], row_to_config)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
        (None, Some(uid)) => {
            let mut stmt = conn
                .prepare("SELECT * FROM mcp_configs WHERE user_id = ?1 ORDER BY name")
                .unwrap();
            stmt.query_map([uid], row_to_config)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
        (None, None) => {
            let mut stmt = conn
                .prepare("SELECT * FROM mcp_configs ORDER BY name")
                .unwrap();
            stmt.query_map([], row_to_config)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
    };
    rows
}

pub fn create_mcp_config(
    db: &Db,
    name: &str,
    command: &str,
    args: &[String],
    project_path: Option<&str>,
    enabled: bool,
    user_id: &str,
) -> McpServerConfig {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let args_json = serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO mcp_configs (id, project_path, name, command, args, enabled, is_builtin, user_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
        rusqlite::params![id, project_path, name, command, args_json, if enabled { 1 } else { 0 }, user_id],
    )
    .unwrap();
    McpServerConfig {
        id,
        project_path: project_path.map(|s| s.to_string()),
        name: name.to_string(),
        command: command.to_string(),
        args: args.to_vec(),
        enabled,
        is_builtin: false,
    }
}

pub fn get_mcp_config_user_id(db: &Db, id: &str) -> Option<String> {
    let conn = db.conn();
    conn.query_row("SELECT user_id FROM mcp_configs WHERE id = ?1", [id], |r| {
        r.get::<_, Option<String>>(0)
    })
    .ok()
    .flatten()
}

pub fn update_mcp_config(
    db: &Db,
    id: &str,
    updates: &serde_json::Value,
) -> Option<McpServerConfig> {
    let conn = db.conn();
    if let Some(enabled) = updates.get("enabled").and_then(|v| v.as_bool()) {
        conn.execute(
            "UPDATE mcp_configs SET enabled = ?1 WHERE id = ?2",
            rusqlite::params![if enabled { 1 } else { 0 }, id],
        )
        .ok();
    }
    if let Some(args) = updates.get("args") {
        let args_json = serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "UPDATE mcp_configs SET args = ?1 WHERE id = ?2",
            rusqlite::params![args_json, id],
        )
        .ok();
    }
    if let Some(name) = updates.get("name").and_then(|v| v.as_str()) {
        conn.execute(
            "UPDATE mcp_configs SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, id],
        )
        .ok();
    }
    if let Some(command) = updates.get("command").and_then(|v| v.as_str()) {
        conn.execute(
            "UPDATE mcp_configs SET command = ?1 WHERE id = ?2",
            rusqlite::params![command, id],
        )
        .ok();
    }
    conn.query_row(
        "SELECT * FROM mcp_configs WHERE id = ?1",
        [id],
        row_to_config,
    )
    .ok()
}

pub fn delete_mcp_config(db: &Db, id: &str) {
    let conn = db.conn();
    conn.execute("DELETE FROM mcp_configs WHERE id = ?1", [id])
        .ok();
}

fn row_to_config(row: &rusqlite::Row) -> rusqlite::Result<McpServerConfig> {
    let args_str: Option<String> = row.get("args")?;
    let args: Vec<String> = args_str
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    Ok(McpServerConfig {
        id: row.get("id")?,
        project_path: row.get("project_path")?,
        name: row.get("name")?,
        command: row.get("command")?,
        args,
        enabled: row.get::<_, i64>("enabled")? == 1,
        is_builtin: false,
    })
}
