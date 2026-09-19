//! Validate an operator-supplied backup, never a live database.
use flamel::{credential_vault::CredentialVault, db::Db};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
fn owners(conn: &rusqlite::Connection) -> BTreeMap<String, String> {
    let mut snapshot = BTreeMap::new();
    for table in [
        "sessions",
        "projects",
        "user_settings",
        "git_accounts",
        "provider_credentials",
    ] {
        let columns = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        if !columns.iter().any(|name| name == "user_id") {
            continue;
        }
        let key = if columns.iter().any(|name| name == "id") {
            "id"
        } else {
            "rowid"
        };
        let mut query = conn
            .prepare(&format!(
                "SELECT CAST({key} AS TEXT),COALESCE(user_id,'') FROM {table} ORDER BY {key}"
            ))
            .unwrap();
        let rows = query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        snapshot.insert(
            table.into(),
            hex::encode(Sha256::digest(serde_json::to_vec(&rows).unwrap())),
        );
    }
    snapshot
}
fn main() {
    let Ok(path) = std::env::var("FLAMEL_TEST_RESTORE") else {
        println!("restore test requires FLAMEL_TEST_RESTORE pointing to an isolated backup");
        return;
    };
    assert!(
        path.contains("flamel-restore-test-"),
        "Refusing to migrate a path without the isolated test marker"
    );
    let before = owners(
        &rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap(),
    );
    let db = Db::open(&path);
    let profiles = flamel::identity::profiles(&db).unwrap();
    if profiles.len() > 1 {
        assert!(flamel::identity::select(&db, None).is_err());
    }
    let vault = CredentialVault::load().expect("load backup vault key");
    let migrated = vault
        .migrate_database_credentials(&db)
        .expect("authenticate and migrate backup credentials");
    let after = owners(&db.conn());
    assert_eq!(before, after, "migration changed persisted ownership");
    assert_eq!(
        db.conn()
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    println!(
        "restored profile: {} owner tables unchanged; {} identities preserved; {} credentials migrated; integrity_check passed",
        before.len(),
        profiles.len(),
        migrated
    );
}
