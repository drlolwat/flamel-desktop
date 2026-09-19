use crate::{credential_vault::CredentialVault, db::Db};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitAccount {
    pub id: String,
    #[serde(rename = "type")]
    pub account_type: String,
    pub label: String,
    pub base_url: String,
    pub token: String,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

fn row_to_account(row: &rusqlite::Row) -> rusqlite::Result<GitAccount> {
    Ok(GitAccount {
        id: row.get("id")?,
        account_type: row.get("type")?,
        label: row.get("label")?,
        base_url: row.get("base_url")?,
        token: "***".to_string(),
        username: row.get("username")?,
        avatar_url: row.get("avatar_url")?,
        created_at: row.get("created_at")?,
        user_id: row.get::<_, Option<String>>("user_id").unwrap_or(None),
    })
}

pub fn list_git_accounts(db: &Db, user_id: Option<&str>) -> Vec<GitAccount> {
    let conn = db.conn();
    if let Some(uid) = user_id {
        let mut stmt = conn
            .prepare("SELECT * FROM git_accounts WHERE user_id = ?1 ORDER BY created_at DESC")
            .unwrap();
        stmt.query_map([uid], row_to_account)
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    } else {
        let mut stmt = conn
            .prepare("SELECT * FROM git_accounts ORDER BY created_at DESC")
            .unwrap();
        stmt.query_map([], row_to_account)
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }
}

pub fn get_git_account_with_token(
    db: &Db,
    vault: &CredentialVault,
    id: &str,
) -> Option<GitAccount> {
    let conn = db.conn();
    conn.query_row("SELECT * FROM git_accounts WHERE id = ?1", [id], |row| {
        Ok(GitAccount {
            id: row.get("id")?,
            account_type: row.get("type")?,
            label: row.get("label")?,
            base_url: row.get("base_url")?,
            token: row.get("token")?,
            username: row.get("username")?,
            avatar_url: row.get("avatar_url")?,
            created_at: row.get("created_at")?,
            user_id: row.get::<_, Option<String>>("user_id").unwrap_or(None),
        })
    })
    .ok()
    .and_then(|mut account| {
        account.token = vault.open(&account.token).ok()?;
        Some(account)
    })
}

pub fn create_git_account(
    db: &Db,
    vault: &CredentialVault,
    user_id: Option<&str>,
    account_type: &str,
    label: &str,
    base_url: &str,
    token: &str,
    username: Option<&str>,
    avatar_url: Option<&str>,
) -> Result<GitAccount, String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let sealed_token = vault.seal(token)?;
    conn.execute(
        "INSERT INTO git_accounts (id, type, label, base_url, token, username, avatar_url, created_at, user_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![id, account_type, label, base_url, sealed_token, username, avatar_url, now, user_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(GitAccount {
        id,
        account_type: account_type.to_string(),
        label: label.to_string(),
        base_url: base_url.to_string(),
        token: "***".to_string(),
        username: username.map(|s| s.to_string()),
        avatar_url: avatar_url.map(|s| s.to_string()),
        created_at: now,
        user_id: user_id.map(|s| s.to_string()),
    })
}

pub fn delete_git_account(db: &Db, id: &str) {
    let conn = db.conn();
    conn.execute("DELETE FROM git_accounts WHERE id = ?1", [id])
        .ok();
}

/// Returns all accounts including their real tokens (for internal use — never expose to frontend).
pub fn list_git_accounts_with_tokens(
    db: &Db,
    vault: &CredentialVault,
    user_id: Option<&str>,
) -> Vec<GitAccount> {
    let conn = db.conn();
    let query = if user_id.is_some() {
        "SELECT * FROM git_accounts WHERE user_id = ?1 ORDER BY created_at DESC"
    } else {
        "SELECT * FROM git_accounts ORDER BY created_at DESC"
    };
    let mut stmt = conn.prepare(query).unwrap();
    let map_row = |row: &rusqlite::Row| {
        Ok(GitAccount {
            id: row.get("id")?,
            account_type: row.get("type")?,
            label: row.get("label")?,
            base_url: row.get("base_url")?,
            token: row.get("token")?,
            username: row.get("username")?,
            avatar_url: row.get("avatar_url")?,
            created_at: row.get("created_at")?,
            user_id: row.get::<_, Option<String>>("user_id").unwrap_or(None),
        })
    };
    if let Some(uid) = user_id {
        stmt.query_map([uid], map_row)
    } else {
        stmt.query_map([], map_row)
    }
    .unwrap()
    .filter_map(|result| {
        let mut account = result.ok()?;
        match vault.open(&account.token) {
            Ok(token) => {
                account.token = token;
                Some(account)
            }
            Err(error) => {
                tracing::error!(account_id = %account.id, %error, "could not open Git account credential");
                None
            }
        }
    })
    .collect()
}
