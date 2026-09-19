//! Provider-agnostic credential storage.
//!
//! Stores OAuth tokens for non-Claude providers (Codex, etc.) in the
//! `provider_credentials` table. Claude credentials remain in `claude_credentials`
//! for backward compatibility — this module handles everything else.

use crate::{credential_vault::CredentialVault, db::Db};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ProviderCredentials {
    pub user_id: String,
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub expires_at: i64,
    pub scopes: String,
    pub extra_json: Option<String>,
    pub updated_at: String,
}

pub fn get_credentials(
    db: &Db,
    vault: &CredentialVault,
    user_id: &str,
    provider: &str,
) -> Option<ProviderCredentials> {
    let conn = db.conn();
    conn.query_row(
        "SELECT user_id, provider, access_token, refresh_token, id_token, expires_at, scopes, extra_json, updated_at
         FROM provider_credentials WHERE user_id = ?1 AND provider = ?2",
        rusqlite::params![user_id, provider],
        |row| {
            Ok(ProviderCredentials {
                user_id: row.get(0)?,
                provider: row.get(1)?,
                access_token: row.get(2)?,
                refresh_token: row.get(3)?,
                id_token: row.get(4)?,
                expires_at: row.get(5)?,
                scopes: row.get(6)?,
                extra_json: row.get(7)?,
                updated_at: row.get(8)?,
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
        credentials.id_token = credentials
            .id_token
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
    provider: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    id_token: Option<&str>,
    expires_at: i64,
    scopes: &str,
    extra_json: Option<&str>,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let access_token = vault.seal(access_token)?;
    let refresh_token = refresh_token.map(|value| vault.seal(value)).transpose()?;
    let id_token = id_token.map(|value| vault.seal(value)).transpose()?;
    let conn = db.conn();
    conn.execute(
        "INSERT INTO provider_credentials (user_id, provider, access_token, refresh_token, id_token, expires_at, scopes, extra_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(user_id, provider) DO UPDATE SET
             access_token = excluded.access_token,
             refresh_token = excluded.refresh_token,
             id_token = excluded.id_token,
             expires_at = excluded.expires_at,
             scopes = excluded.scopes,
             extra_json = excluded.extra_json,
             updated_at = excluded.updated_at",
        rusqlite::params![user_id, provider, access_token, refresh_token, id_token, expires_at, scopes, extra_json, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_credentials(db: &Db, user_id: &str, provider: &str) {
    let conn = db.conn();
    conn.execute(
        "DELETE FROM provider_credentials WHERE user_id = ?1 AND provider = ?2",
        rusqlite::params![user_id, provider],
    )
    .ok();
}

pub fn has_credentials(db: &Db, user_id: &str, provider: &str) -> bool {
    let conn = db.conn();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM provider_credentials WHERE user_id = ?1 AND provider = ?2",
            rusqlite::params![user_id, provider],
            |r| r.get(0),
        )
        .unwrap_or(0);
    count > 0
}
