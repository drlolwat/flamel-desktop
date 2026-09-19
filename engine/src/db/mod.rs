pub mod agent_tracking;
pub mod agents;
pub mod artifacts;
pub mod brain_store;
pub mod builtin_skills;
pub mod capabilities;
pub mod capability_packs;
pub mod chat_inputs;
pub mod claude_auth;
pub mod context;
pub mod credentials;
pub mod flamel_users;
pub mod git_accounts;
pub mod input_requests;
pub mod intelligence;
pub mod maintenance;
pub mod mcp;
pub mod memory_store;
pub mod ownership;
pub mod plan_quality;
pub mod plans;
pub mod prompts;
pub mod provider_auth;
pub mod realtime;
pub mod run_lifecycle;
pub mod runtime_catalog;
pub mod schema;
pub mod session_titles;
pub mod sessions;
pub mod settings;
pub mod shares;
pub mod skill_catalog;
pub mod todos;
pub mod work;

use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// Thread-safe database handle. rusqlite::Connection is !Send, so we wrap in Mutex.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &str) -> Self {
        if let Some(parent) = PathBuf::from(path).parent() {
            fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path).expect("Failed to open database");
        conn.busy_timeout(std::time::Duration::from_secs(10))
            .expect("Failed to configure database busy timeout");
        let is_new_database = conn
            .query_row(
                "SELECT NOT EXISTS(
                    SELECT 1 FROM sqlite_schema
                    WHERE type='table' AND name NOT LIKE 'sqlite_%'
                 )",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false);
        if is_new_database {
            // This must be selected before the first table is created. Existing
            // databases are converted by the supervised offline maintenance
            // path because changing auto_vacuum requires a full VACUUM.
            conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")
                .expect("Failed to configure incremental database vacuuming");
        }
        schema::run_migrations(&conn);
        conn.execute_batch("CREATE TABLE IF NOT EXISTS native_profiles(user_id TEXT PRIMARY KEY,label TEXT NOT NULL,created_at TEXT NOT NULL);").expect("initialize native profiles");
        plan_quality::migrate(&conn).expect("Failed to initialize website acceptance");
        run_lifecycle::migrate(&conn).expect("Failed to initialize execution ownership");
        // WAL keeps readers responsive while provider streams persist output.
        // These settings are intentionally conservative for both Desktop and
        // small hosted servers; SQLite remains the single source of truth.
        conn.execute_batch(
            "PRAGMA synchronous=NORMAL;
             PRAGMA temp_store=MEMORY;
             PRAGMA wal_autocheckpoint=1000;
             PRAGMA cache_size=-65536;
             PRAGMA mmap_size=268435456;",
        )
        .expect("Failed to configure database runtime pragmas");
        Db {
            conn: Mutex::new(conn),
        }
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::Db;

    #[test]
    fn new_databases_enable_incremental_auto_vacuum() {
        let path =
            std::env::temp_dir().join(format!("flamel-auto-vacuum-{}.db", uuid::Uuid::new_v4()));
        let db = Db::open(path.to_str().expect("temporary database path"));
        let mode = db
            .conn()
            .query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, i64>(0))
            .expect("read auto_vacuum mode");
        assert_eq!(mode, 2);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

pub fn default_db_path() -> String {
    if let Ok(p) = std::env::var("DB_PATH") {
        return p;
    }
    let data_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("@flamel");
    fs::create_dir_all(&data_dir).ok();

    let new_path = data_dir.join("flamel.db");

    // Migrate from reclaude on first run (project rename)
    if !new_path.exists() {
        let reclaude_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("@reclaude");
        let reclaude_path = reclaude_dir.join("reclaude.db");
        if reclaude_path.exists() {
            tracing::info!(
                "migrating DB from reclaude → flamel: {}",
                reclaude_path.display()
            );
            if let Err(e) = fs::copy(&reclaude_path, &new_path) {
                tracing::error!("failed to migrate DB: {e}");
            }
        }
    }

    // Migrate from old Electron location on first run
    if !new_path.exists() {
        let old_path = data_dir.join("electron").join("flamel.db");
        if old_path.exists() {
            tracing::info!(
                "migrating DB from {} to {}",
                old_path.display(),
                new_path.display()
            );
            if let Err(e) = fs::copy(&old_path, &new_path) {
                tracing::error!("failed to migrate DB: {e}");
            }
        }
    }

    new_path.to_string_lossy().into_owned()
}
