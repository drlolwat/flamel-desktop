use crate::{credential_vault::CredentialVault, db::Db};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCredentials {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix timestamp (seconds) when access_token expires
    pub expires_at: i64,
    pub scopes: String,
    pub updated_at: String,
}

pub fn get_credentials(
    db: &Db,
    vault: &CredentialVault,
    user_id: &str,
) -> Option<ClaudeCredentials> {
    let conn = db.conn();
    conn.query_row(
        "SELECT user_id, access_token, refresh_token, expires_at, scopes, updated_at
         FROM claude_credentials WHERE user_id = ?1",
        [user_id],
        |row| {
            Ok(ClaudeCredentials {
                user_id: row.get(0)?,
                access_token: row.get(1)?,
                refresh_token: row.get(2)?,
                expires_at: row.get(3)?,
                scopes: row.get(4)?,
                updated_at: row.get(5)?,
            })
        },
    )
    .ok()
    .and_then(|mut credentials| {
        credentials.access_token = vault.open(&credentials.access_token).ok()?;
        credentials.refresh_token = credentials
            .refresh_token
            .map(|value| vault.open(&value))
            .transpose()
            .ok()?;
        Some(credentials)
    })
}

pub fn upsert_credentials(
    db: &Db,
    vault: &CredentialVault,
    user_id: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: i64,
    scopes: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let access_token = vault.seal(access_token)?;
    let refresh_token = refresh_token.map(|value| vault.seal(value)).transpose()?;
    let conn = db.conn();
    conn.execute(
        "INSERT INTO claude_credentials (user_id, access_token, refresh_token, expires_at, scopes, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(user_id) DO UPDATE SET
             access_token = excluded.access_token,
             refresh_token = excluded.refresh_token,
             expires_at = excluded.expires_at,
             scopes = excluded.scopes,
             updated_at = excluded.updated_at",
        rusqlite::params![user_id, access_token, refresh_token, expires_at, scopes, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_credentials(db: &Db, user_id: &str) {
    let conn = db.conn();
    conn.execute(
        "DELETE FROM claude_credentials WHERE user_id = ?1",
        [user_id],
    )
    .ok();
}

pub fn has_credentials(db: &Db, user_id: &str) -> bool {
    let conn = db.conn();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM claude_credentials WHERE user_id = ?1",
            [user_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    count > 0
}
