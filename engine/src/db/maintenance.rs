use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseOptimizationReport {
    pub database_path: String,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub archived_context_revisions_removed: usize,
    pub obsolete_fts_removed: bool,
    pub auto_vacuum: String,
    pub integrity_check: String,
    pub elapsed_seconds: u64,
}

fn database_bytes(path: &Path) -> u64 {
    [
        path.to_path_buf(),
        path.with_extension("db-wal"),
        path.with_extension("db-shm"),
    ]
    .iter()
    .filter_map(|candidate| std::fs::metadata(candidate).ok())
    .map(|metadata| metadata.len())
    .sum()
}

/// Rebuilds a stopped Flamel database after removing storage created by the
/// legacy streaming-snapshot ledger. The caller owns process supervision: no
/// server or Actions process may have the database open while this runs.
pub fn optimize_database(path: &Path) -> Result<DatabaseOptimizationReport, String> {
    if !path.is_file() {
        return Err(format!("database does not exist: {}", path.display()));
    }

    let started = Instant::now();
    let bytes_before = database_bytes(path);
    let mut conn = Connection::open(path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("failed to configure database timeout: {error}"))?;

    let transaction = conn
        .transaction()
        .map_err(|error| format!("database is busy; stop Flamel before optimizing: {error}"))?;
    let obsolete_fts_removed = transaction
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE name='context_events_fts' LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("failed to inspect legacy context index: {error}"))?
        .is_some();
    transaction
        .execute_batch(
            "DROP TRIGGER IF EXISTS context_events_fts_insert;
             DROP TRIGGER IF EXISTS context_events_fts_delete;
             DROP TABLE IF EXISTS context_events_fts;",
        )
        .map_err(|error| format!("failed to remove legacy context index: {error}"))?;
    let archived_context_revisions_removed = transaction
        .execute(
            "DELETE FROM context_events
             WHERE source_kind='session_message' AND active=0",
            [],
        )
        .map_err(|error| format!("failed to remove archived context snapshots: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("failed to commit database cleanup: {error}"))?;

    conn.execute_batch(
        "PRAGMA wal_checkpoint(TRUNCATE);
         PRAGMA journal_mode=DELETE;
         PRAGMA auto_vacuum=INCREMENTAL;
         VACUUM;
         PRAGMA journal_mode=WAL;
         PRAGMA optimize;",
    )
    .map_err(|error| format!("failed to rebuild database: {error}"))?;

    let auto_vacuum = conn
        .query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("failed to verify auto_vacuum mode: {error}"))?;
    if auto_vacuum != 2 {
        return Err(format!(
            "database rebuild did not enable incremental auto-vacuum (mode {auto_vacuum})"
        ));
    }
    let integrity_check = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(|error| format!("database integrity check failed to run: {error}"))?;
    if integrity_check != "ok" {
        return Err(format!(
            "database integrity check failed: {integrity_check}"
        ));
    }
    drop(conn);

    Ok(DatabaseOptimizationReport {
        database_path: path.display().to_string(),
        bytes_before,
        bytes_after: database_bytes(path),
        archived_context_revisions_removed,
        obsolete_fts_removed,
        auto_vacuum: "incremental".to_string(),
        integrity_check,
        elapsed_seconds: started.elapsed().as_secs(),
    })
}

#[cfg(test)]
mod tests {
    use super::optimize_database;
    use rusqlite::Connection;

    #[test]
    fn optimizer_removes_legacy_storage_and_preserves_active_context() {
        let path = std::env::temp_dir().join(format!(
            "flamel-database-optimization-{}.db",
            uuid::Uuid::new_v4()
        ));
        let conn = Connection::open(&path).expect("create legacy database");
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE context_events (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL UNIQUE,
                session_id TEXT NOT NULL,
                source_kind TEXT NOT NULL,
                source_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL
             );
             CREATE VIRTUAL TABLE context_events_fts USING fts5(content);
             CREATE TRIGGER context_events_fts_insert AFTER INSERT ON context_events BEGIN
                INSERT INTO context_events_fts(rowid, content) VALUES (new.sequence, new.content);
             END;",
        )
        .expect("create legacy context schema");
        for index in 0..8 {
            conn.execute(
                "INSERT INTO context_events
                 (event_id, session_id, source_kind, source_id, role, content, content_hash, active, created_at)
                 VALUES (?1, 'session', 'session_message', '1', 'assistant', ?2, ?1, ?3, 'now')",
                (format!("event-{index}"), "x".repeat(16_384), i64::from(index == 7)),
            )
            .expect("insert legacy context revision");
        }
        drop(conn);

        let report = optimize_database(&path).expect("optimize legacy database");
        assert_eq!(report.archived_context_revisions_removed, 7);
        assert!(report.obsolete_fts_removed);
        assert_eq!(report.auto_vacuum, "incremental");
        assert_eq!(report.integrity_check, "ok");

        let conn = Connection::open(&path).expect("reopen optimized database");
        let remaining = conn
            .query_row("SELECT COUNT(*) FROM context_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("count active context");
        let fts_exists = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name='context_events_fts'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("inspect obsolete FTS table");
        let auto_vacuum = conn
            .query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, i64>(0))
            .expect("inspect auto-vacuum mode");
        assert_eq!(remaining, 1);
        assert_eq!(fts_exists, 0);
        assert_eq!(auto_vacuum, 2);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}
