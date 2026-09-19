use crate::{credential_vault::CredentialVault, db::Db};
use rusqlite::OptionalExtension;
use serde::Serialize;

fn get_setting(db: &Db, key: &str) -> Option<String> {
    let conn = db.conn();
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        [key],
        |r| r.get(0),
    )
    .ok()
}

pub fn delete_setting(db: &Db, key: &str) {
    let conn = db.conn();
    conn.execute("DELETE FROM app_settings WHERE key = ?1", [key])
        .ok();
}

fn set_setting(db: &Db, key: &str, value: &str) {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .ok();
}

pub fn get_anthropic_api_key(db: &Db) -> Option<String> {
    let vault = CredentialVault::load().ok()?;
    get_setting(db, "anthropic_api_key").and_then(|value| vault.open(&value).ok())
}

pub fn set_anthropic_api_key(db: &Db, key: &str) -> Result<(), String> {
    let vault = CredentialVault::load()?;
    set_setting(db, "anthropic_api_key", &vault.seal(key)?);
    Ok(())
}

pub fn clear_anthropic_api_key(db: &Db) {
    let conn = db.conn();
    conn.execute(
        "DELETE FROM app_settings WHERE key = 'anthropic_api_key'",
        [],
    )
    .ok();
}

#[derive(Debug, Clone, Serialize)]
pub struct UserSettings {
    pub git_name: Option<String>,
    pub git_email: Option<String>,
    pub git_signing_key: Option<String>,
    pub preferred_provider: String,
    pub cross_provider_fallback: bool,
    pub codex_planning_reasoning_effort: String,
    pub claude_planning_reasoning_effort: String,
}

pub fn get_user_settings(db: &Db, user_id: &str) -> UserSettings {
    let conn = db.conn();
    conn.query_row(
        "SELECT git_name, git_email, git_signing_key, preferred_provider, cross_provider_fallback, codex_planning_reasoning_effort, claude_planning_reasoning_effort FROM user_settings WHERE user_id = ?1",
        [user_id],
        |row| {
            Ok(UserSettings {
                git_name: row.get(0)?,
                git_email: row.get(1)?,
                git_signing_key: row.get(2)?,
                preferred_provider: row
                    .get::<_, Option<String>>(3)?
                    .filter(|value| matches!(value.as_str(), "claude" | "codex"))
                    .unwrap_or_else(|| "codex".to_string()),
                cross_provider_fallback: row.get::<_, Option<i64>>(4)?.unwrap_or(1) != 0,
                codex_planning_reasoning_effort: row
                    .get::<_, Option<String>>(5)?
                    .filter(|value| matches!(value.as_str(), "high" | "xhigh" | "max" | "ultra"))
                    .unwrap_or_else(|| "xhigh".to_string()),
                claude_planning_reasoning_effort: row
                    .get::<_, Option<String>>(6)?
                    .filter(|value| matches!(value.as_str(), "high" | "xhigh" | "max" | "ultra"))
                    .unwrap_or_else(|| "xhigh".to_string()),
            })
        },
    )
    .unwrap_or(UserSettings {
        git_name: None,
        git_email: None,
        git_signing_key: None,
        preferred_provider: "codex".to_string(),
        cross_provider_fallback: true,
        codex_planning_reasoning_effort: "xhigh".to_string(),
        claude_planning_reasoning_effort: "xhigh".to_string(),
    })
}

// ── MCP token — stable across restarts ───────────────────────────────────────

pub fn get_or_create_mcp_token(db: &Db, vault: &CredentialVault) -> Result<String, String> {
    // Previous releases copied `mcp_token` into tenant provider configs. Never
    // reuse that legacy shared secret after adopting account-scoped tokens.
    const COORDINATOR_KEY: &str = "mcp_coordinator_token_v2";
    if let Some(token) = get_setting(db, COORDINATOR_KEY) {
        return vault.open(&token);
    }
    let token = hex::encode(rand::random::<[u8; 32]>());
    let sealed = vault.seal(&token)?;
    let conn = db.conn();
    conn.execute(
        "INSERT OR IGNORE INTO app_settings (key, value) VALUES (?1, ?2)",
        rusqlite::params![COORDINATOR_KEY, sealed],
    )
    .map_err(|error| format!("Could not persist MCP coordinator credential: {error}"))?;
    let persisted: String = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [COORDINATOR_KEY],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not read MCP coordinator credential: {error}"))?;
    vault.open(&persisted)
}

// ── flamel.dev server registration ───────────────────────────────────────────

fn local_secret_setting_key(user_id: &str, name: &str) -> Result<String, String> {
    let valid_user = !user_id.is_empty()
        && user_id.len() <= 128
        && user_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    let valid_name = !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if !valid_user || !valid_name {
        return Err("invalid local secret name".into());
    }
    Ok(format!("local_secret.{user_id}.{name}"))
}

pub fn get_local_secret(
    db: &Db,
    vault: &CredentialVault,
    user_id: &str,
    name: &str,
) -> Result<Option<String>, String> {
    let key = local_secret_setting_key(user_id, name)?;
    db.conn()
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .map(|value| vault.open(&value))
        .transpose()
}

pub fn set_local_secret(
    db: &Db,
    vault: &CredentialVault,
    user_id: &str,
    name: &str,
    value: &str,
) -> Result<(), String> {
    let key = local_secret_setting_key(user_id, name)?;
    let sealed = vault.seal(value)?;
    db.conn()
        .execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![key, sealed],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn delete_local_secret(db: &Db, user_id: &str, name: &str) -> Result<bool, String> {
    let key = local_secret_setting_key(user_id, name)?;
    db.conn()
        .execute("DELETE FROM app_settings WHERE key=?1", [key])
        .map(|deleted| deleted > 0)
        .map_err(|error| error.to_string())
}

/// Transient state token for the flamel.dev SSO login flow (one active at a time).

/// "individual" (default) or "org_api_key"
pub fn get_claude_access_mode(db: &Db) -> String {
    get_setting(db, "claude_access_mode").unwrap_or_else(|| "individual".to_string())
}
pub fn set_claude_access_mode(db: &Db, mode: &str) {
    set_setting(db, "claude_access_mode", mode);
}

pub fn update_user_settings(
    db: &Db,
    user_id: &str,
    git_name: Option<&str>,
    git_email: Option<&str>,
    git_signing_key: Option<&str>,
    preferred_provider: Option<&str>,
    cross_provider_fallback: Option<bool>,
    codex_planning_reasoning_effort: Option<&str>,
    claude_planning_reasoning_effort: Option<&str>,
) -> UserSettings {
    {
        let conn = db.conn();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO user_settings (user_id, git_name, git_email, git_signing_key, preferred_provider, cross_provider_fallback, codex_planning_reasoning_effort, claude_planning_reasoning_effort, updated_at)
             VALUES (?1, ?2, ?3, ?4, COALESCE(?5, 'codex'), COALESCE(?6, 1), COALESCE(?7, 'xhigh'), COALESCE(?8, 'xhigh'), ?9)
             ON CONFLICT(user_id) DO UPDATE SET
               git_name = excluded.git_name,
               git_email = excluded.git_email,
               git_signing_key = excluded.git_signing_key,
               preferred_provider = COALESCE(?5, user_settings.preferred_provider),
               cross_provider_fallback = COALESCE(?6, user_settings.cross_provider_fallback),
               codex_planning_reasoning_effort = COALESCE(?7, user_settings.codex_planning_reasoning_effort),
               claude_planning_reasoning_effort = COALESCE(?8, user_settings.claude_planning_reasoning_effort),
               updated_at = excluded.updated_at",
            rusqlite::params![
                user_id,
                git_name,
                git_email,
                git_signing_key,
                preferred_provider.filter(|value| matches!(*value, "claude" | "codex")),
                cross_provider_fallback.map(i64::from),
                codex_planning_reasoning_effort.filter(|value| matches!(*value, "high" | "xhigh" | "max" | "ultra")),
                claude_planning_reasoning_effort.filter(|value| matches!(*value, "high" | "xhigh" | "max" | "ultra")),
                now
            ],
        )
        .ok();
    } // drop conn lock before re-acquiring in get_user_settings

    get_user_settings(db, user_id)
}
