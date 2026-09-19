//! Preserve migration evidence; selecting a local profile never transfers ownership.
use rusqlite::Connection;
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS ownership_migrations(name TEXT PRIMARY KEY,source_user_id TEXT,target_user_id TEXT NOT NULL,moved_rows INTEGER NOT NULL,applied_at TEXT NOT NULL);")
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_migrations_preserve_private_records_and_owner_ids() {
        let db = super::super::Db::open(":memory:");
        db.conn().execute_batch("CREATE TABLE flamel_dev_users(flamel_user_id TEXT,email TEXT,role TEXT,created_at TEXT); INSERT INTO flamel_dev_users VALUES('a','a@example.test','member','now'),('b','b@example.test','admin','now');CREATE TABLE runners(id TEXT,secret TEXT);INSERT INTO runners VALUES('worker','legacy-value');INSERT INTO sessions(id,project_path,created_at,updated_at,user_id) VALUES('work','/repo','now','now','a');").unwrap();
        super::super::schema::run_migrations(&db.conn());
        assert!(crate::identity::select(&db, None).is_err());
        assert_eq!(crate::identity::select(&db, Some("a")).unwrap(), "a");
        assert_eq!(
            db.conn()
                .query_row("SELECT user_id FROM sessions WHERE id='work'", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap(),
            "a"
        );
        assert_eq!(
            db.conn()
                .query_row(
                    "SELECT role FROM flamel_dev_users WHERE flamel_user_id='a'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "member"
        );
        assert_eq!(
            db.conn()
                .query_row("SELECT secret FROM runners", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "legacy-value"
        );
    }
    #[test]
    fn fresh_local_database_has_no_private_service_tables() {
        let db = super::super::Db::open(":memory:");
        for table in [
            "flamel_dev_users",
            "runners",
            "project_runners",
            "local_accounts",
            "local_grants",
            "flamel_assistant_idempotency",
            "flamel_assistant_chattle_keys",
            "shares",
        ] {
            assert_eq!(
                db.conn()
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_schema WHERE name=?1",
                        [table],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                0,
                "{table}"
            );
        }
    }
}
