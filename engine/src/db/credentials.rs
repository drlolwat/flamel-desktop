use crate::db::Db;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct StoredCredential {
    pub kind: String,
    pub encrypted_value: String,
    pub metadata: Value,
    pub expires_at: Option<String>,
}

pub fn store(
    db: &Db,
    user_id: &str,
    capability_id: &str,
    kind: &str,
    encrypted: &str,
    metadata: &serde_json::Value,
    expires_at: Option<&str>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let conn = db.conn();
    conn.execute(
        "DELETE FROM capability_credentials WHERE user_id=?1 AND capability_id=?2",
        rusqlite::params![user_id, capability_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO capability_credentials (id,user_id,capability_id,kind,encrypted_value,metadata_json,expires_at,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",rusqlite::params![id,user_id,capability_id,kind,encrypted,metadata.to_string(),expires_at,now]).map_err(|e|e.to_string())?;
    conn.execute("UPDATE capability_connections SET credential_ref=?1,status='configured',last_error=NULL,updated_at=?2 WHERE user_id=?3 AND capability_id=?4",rusqlite::params![id,now,user_id,capability_id]).map_err(|e|e.to_string())?;
    Ok(id)
}
pub fn encrypted(db: &Db, user_id: &str, capability_id: &str) -> Option<String> {
    db.conn().query_row("SELECT encrypted_value FROM capability_credentials WHERE user_id=?1 AND capability_id=?2 AND (expires_at IS NULL OR julianday(expires_at)>julianday('now'))",rusqlite::params![user_id,capability_id],|r|r.get(0)).ok()
}

pub fn record(db: &Db, user_id: &str, capability_id: &str) -> Option<StoredCredential> {
    db.conn()
        .query_row(
            "SELECT kind,encrypted_value,metadata_json,expires_at
             FROM capability_credentials WHERE user_id=?1 AND capability_id=?2",
            rusqlite::params![user_id, capability_id],
            |row| {
                let metadata: String = row.get(2)?;
                Ok(StoredCredential {
                    kind: row.get(0)?,
                    encrypted_value: row.get(1)?,
                    metadata: serde_json::from_str(&metadata)
                        .unwrap_or_else(|_| serde_json::json!({})),
                    expires_at: row.get(3)?,
                })
            },
        )
        .ok()
}

pub fn delete(db: &Db, user_id: &str, capability_id: &str) -> bool {
    let conn = db.conn();
    let rows = conn
        .execute(
            "DELETE FROM capability_credentials WHERE user_id=?1 AND capability_id=?2",
            rusqlite::params![user_id, capability_id],
        )
        .unwrap_or(0);
    let _=conn.execute("UPDATE capability_connections SET credential_ref=NULL,status='needs_auth' WHERE user_id=?1 AND capability_id=?2",rusqlite::params![user_id,capability_id]);
    rows > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn credentials_are_tenant_scoped_and_revocation_is_owner_local() {
        let db = Db::open(":memory:");
        crate::db::capabilities::seed_builtin_catalog(&db);
        store(
            &db,
            "tenant-a",
            "builtin-flamel",
            "token",
            "encrypted-a",
            &json!({"owner":"a"}),
            None,
        )
        .unwrap();
        store(
            &db,
            "tenant-b",
            "builtin-flamel",
            "token",
            "encrypted-b",
            &json!({"owner":"b"}),
            None,
        )
        .unwrap();

        assert_eq!(
            record(&db, "tenant-a", "builtin-flamel")
                .unwrap()
                .encrypted_value,
            "encrypted-a"
        );
        assert_eq!(
            record(&db, "tenant-b", "builtin-flamel")
                .unwrap()
                .encrypted_value,
            "encrypted-b"
        );
        assert!(record(&db, "tenant-c", "builtin-flamel").is_none());

        assert!(delete(&db, "tenant-a", "builtin-flamel"));
        assert!(record(&db, "tenant-a", "builtin-flamel").is_none());
        assert_eq!(
            record(&db, "tenant-b", "builtin-flamel")
                .unwrap()
                .encrypted_value,
            "encrypted-b"
        );
    }
}
