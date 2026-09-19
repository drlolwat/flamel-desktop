//! Local profiles preserve persisted ownership. Cloud authentication is optional.
use crate::db::{Db, flamel_users};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub label: String,
}

pub fn migrate(db: &Db) -> Result<(), String> {
    db.conn()
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS native_profiles (
        user_id TEXT PRIMARY KEY, label TEXT NOT NULL, created_at TEXT NOT NULL
    );",
        )
        .map_err(|e| e.to_string())
}

pub fn profiles(db: &Db) -> Result<Vec<Profile>, String> {
    migrate(db)?;
    let users = flamel_users::list_flamel_users(db);
    let mut ids: BTreeSet<String> = users.iter().map(|u| u.flamel_user_id.clone()).collect();
    let conn = db.conn();
    // Read owner IDs, never rewrite ownership during startup.
    for table in [
        "sessions",
        "projects",
        "native_profiles",
        "user_settings",
        "provider_credentials",
        "claude_credentials",
        "git_accounts",
        "plans",
        "todos",
        "prompts",
        "mcp_configs",
        "agent_configs",
    ] {
        let mut stmt = match conn.prepare(&format!(
            "SELECT DISTINCT user_id FROM {table} WHERE user_id IS NOT NULL AND user_id <> ''"
        )) {
            Ok(stmt) => stmt,
            Err(_) => continue, // Legacy versions do not have every table.
        };
        for id in stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
        {
            ids.insert(id.map_err(|e| e.to_string())?);
        }
    }
    Ok(ids
        .into_iter()
        .map(|id| Profile {
            label: users
                .iter()
                .find(|u| u.flamel_user_id == id)
                .map(|u| u.email.clone())
                .unwrap_or_else(|| id.clone()),
            id,
        })
        .collect())
}

/// Called only by the trusted native host, after profile selection if ambiguous.
pub fn select(db: &Db, requested: Option<&str>) -> Result<String, String> {
    let choices = profiles(db)?;
    let id = match (requested, choices.as_slice()) {
        (Some(id), _) if choices.iter().any(|p| p.id == id) => id.to_owned(),
        (Some(_), _) => return Err("Unknown local profile".into()),
        (None, []) => crate::FLAMEL_LOCAL_USER_ID.to_owned(),
        (None, [profile]) => profile.id.clone(),
        _ => return Err("Choose a local profile before opening this workspace".into()),
    };
    let label = choices
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.label.as_str())
        .unwrap_or("Local workspace");
    let mut conn = db.conn();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("INSERT OR IGNORE INTO native_profiles(user_id,label,created_at) VALUES (?1,?2,datetime('now'))", params![id,label]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn is_bound(db: &Db, id: &str) -> bool {
    db.conn()
        .query_row(
            "SELECT 1 FROM native_profiles WHERE user_id=?1",
            [id],
            |_| Ok(true),
        )
        .optional()
        .ok()
        .flatten()
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_profile_needs_no_cloud_account() {
        let db = Db::open(":memory:");
        assert_eq!(select(&db, None).unwrap(), crate::FLAMEL_LOCAL_USER_ID);
        assert_eq!(select(&db, None).unwrap(), crate::FLAMEL_LOCAL_USER_ID);
    }
    #[test]
    fn ambiguous_legacy_profiles_require_choice_and_preserve_roles() {
        let db = Db::open(":memory:");
        db.conn().execute_batch("CREATE TABLE flamel_dev_users(flamel_user_id TEXT,email TEXT,role TEXT,created_at TEXT); INSERT INTO flamel_dev_users VALUES('first','one@example.test','member','1'),('second','two@example.test','admin','2');").unwrap();
        assert!(select(&db, None).is_err());
        assert!(select(&db, Some("unknown")).is_err());
        assert_eq!(select(&db, Some("first")).unwrap(), "first");
        assert_eq!(
            db.conn()
                .query_row(
                    "SELECT role FROM flamel_dev_users WHERE flamel_user_id='first'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .ok()
                .as_deref(),
            Some("member")
        );
        assert_eq!(profiles(&db).unwrap().len(), 2);
    }
}
