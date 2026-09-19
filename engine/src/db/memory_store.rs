use rusqlite::Connection;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// Separate database handle for the memory store (memories.db).
pub struct MemoryDb {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    pub user_id: String,
    pub project_path: String,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub provenance: serde_json::Value,
    pub confidence: f64,
    pub source_kind: String,
    pub source_id: Option<String>,
    pub pinned: bool,
    pub superseded_by: Option<String>,
    pub retention: String,
    pub expires_at: Option<i64>,
}

impl MemoryDb {
    pub fn open(path: &str) -> Self {
        if let Some(parent) = PathBuf::from(path).parent() {
            fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path).expect("Failed to open memories database");
        run_migrations(&conn);
        MemoryDb {
            conn: Mutex::new(conn),
        }
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
}

fn run_migrations(conn: &Connection) {
    conn.execute_batch("PRAGMA journal_mode=WAL;").ok();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memories (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            project_path TEXT NOT NULL DEFAULT '',
            content TEXT NOT NULL,
            tags TEXT NOT NULL DEFAULT '[]',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
            content,
            tags,
            content='memories',
            content_rowid='rowid'
        );

        -- Sync triggers: keep FTS index in sync with memories table
        CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts(rowid, content, tags)
            VALUES (new.rowid, new.content, new.tags);
        END;

        CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, content, tags)
            VALUES ('delete', old.rowid, old.content, old.tags);
        END;

        CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, content, tags)
            VALUES ('delete', old.rowid, old.content, old.tags);
            INSERT INTO memories_fts(rowid, content, tags)
            VALUES (new.rowid, new.content, new.tags);
        END;

        CREATE INDEX IF NOT EXISTS idx_memories_user ON memories(user_id);
        CREATE INDEX IF NOT EXISTS idx_memories_user_project ON memories(user_id, project_path);",
    )
    .expect("Failed to run memory migrations");
    for statement in [
        "ALTER TABLE memories ADD COLUMN provenance_json TEXT NOT NULL DEFAULT '{}'",
        "ALTER TABLE memories ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0",
        "ALTER TABLE memories ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'user_statement'",
        "ALTER TABLE memories ADD COLUMN source_id TEXT",
        "ALTER TABLE memories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE memories ADD COLUMN superseded_by TEXT",
        "ALTER TABLE memories ADD COLUMN retention TEXT NOT NULL DEFAULT 'durable'",
        "ALTER TABLE memories ADD COLUMN expires_at INTEGER",
    ] {
        let _ = conn.execute(statement, []);
    }
}

fn parse_tags(tags_json: &str) -> Vec<String> {
    serde_json::from_str(tags_json).unwrap_or_default()
}

fn row_to_memory(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    let tags_json: String = row.get(4)?;
    Ok(Memory {
        id: row.get(0)?,
        user_id: row.get(1)?,
        project_path: row.get(2)?,
        content: row.get(3)?,
        tags: parse_tags(&tags_json),
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        provenance: serde_json::from_str(&row.get::<_, String>(7)?)
            .unwrap_or_else(|_| serde_json::json!({})),
        confidence: row.get(8)?,
        source_kind: row.get(9)?,
        source_id: row.get(10)?,
        pinned: row.get::<_, i64>(11)? == 1,
        superseded_by: row.get(12)?,
        retention: row.get(13)?,
        expires_at: row.get(14)?,
    })
}

pub fn add_memory(
    db: &MemoryDb,
    user_id: &str,
    project_path: &str,
    content: &str,
    tags: &[String],
) -> Result<Memory, String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string());

    conn.execute(
        "INSERT INTO memories (id, user_id, project_path, content, tags, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, user_id, project_path, content, tags_json, now, now],
    )
    .map_err(|e| e.to_string())?;

    Ok(Memory {
        id,
        user_id: user_id.to_string(),
        project_path: project_path.to_string(),
        content: content.to_string(),
        tags: tags.to_vec(),
        created_at: now,
        updated_at: now,
        provenance: serde_json::json!({"source":"explicit memory tool"}),
        confidence: 1.0,
        source_kind: "user_statement".into(),
        source_id: None,
        pinned: false,
        superseded_by: None,
        retention: "durable".into(),
        expires_at: None,
    })
}

pub fn set_evidence(
    db: &MemoryDb,
    id: &str,
    user_id: &str,
    provenance: &serde_json::Value,
    confidence: f64,
    source_kind: &str,
    source_id: Option<&str>,
    pinned: bool,
    retention: &str,
    expires_at: Option<i64>,
) -> Result<(), String> {
    let rows = db
        .conn()
        .execute(
            "UPDATE memories SET provenance_json=?1,confidence=?2,source_kind=?3,
        source_id=?4,pinned=?5,retention=?6,expires_at=?7 WHERE id=?8 AND user_id=?9",
            rusqlite::params![
                provenance.to_string(),
                confidence.clamp(0.0, 1.0),
                source_kind,
                source_id,
                pinned,
                retention,
                expires_at,
                id,
                user_id
            ],
        )
        .map_err(|e| e.to_string())?;
    if rows == 0 {
        return Err("Memory not found or not owned by user".into());
    }
    Ok(())
}

pub fn supersede_memory(
    db: &MemoryDb,
    id: &str,
    user_id: &str,
    replacement_id: &str,
) -> Result<(), String> {
    let conn = db.conn();
    let replacement_exists = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM memories WHERE id=?1 AND user_id=?2)",
            rusqlite::params![replacement_id, user_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|e| e.to_string())?;
    if !replacement_exists {
        return Err("Replacement memory not found or not owned by user".into());
    }
    let rows = conn
        .execute(
            "UPDATE memories SET superseded_by=?1 WHERE id=?2 AND user_id=?3",
            rusqlite::params![replacement_id, id, user_id],
        )
        .map_err(|e| e.to_string())?;
    if rows == 0 {
        return Err("Memory not found or not owned by user".into());
    }
    Ok(())
}

pub fn pin_memory(db: &MemoryDb, id: &str, user_id: &str, pinned: bool) -> Result<(), String> {
    let rows = db
        .conn()
        .execute(
            "UPDATE memories SET pinned=?1,updated_at=unixepoch() WHERE id=?2 AND user_id=?3",
            rusqlite::params![pinned, id, user_id],
        )
        .map_err(|e| e.to_string())?;
    if rows == 0 {
        return Err("Memory not found or not owned by user".into());
    }
    Ok(())
}

fn search_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for raw in query.split(|c: char| !c.is_alphanumeric()) {
        let token = raw.trim().to_lowercase();
        if token.len() < 2 || is_search_stopword(&token) || tokens.contains(&token) {
            continue;
        }
        tokens.push(token);
    }
    tokens
}

fn is_search_stopword(token: &str) -> bool {
    matches!(
        token,
        "about"
            | "after"
            | "again"
            | "also"
            | "and"
            | "are"
            | "because"
            | "before"
            | "but"
            | "can"
            | "for"
            | "from"
            | "has"
            | "have"
            | "into"
            | "need"
            | "needs"
            | "not"
            | "our"
            | "should"
            | "that"
            | "the"
            | "then"
            | "this"
            | "use"
            | "user"
            | "when"
            | "with"
            | "you"
    )
}

fn memory_search_score(
    memory: &Memory,
    phrase: &str,
    tokens: &[String],
    project_path: Option<&str>,
) -> i64 {
    let content = memory.content.to_lowercase();
    let tags = memory.tags.join(" ").to_lowercase();
    let memory_project = memory.project_path.to_lowercase();
    let mut score = 0;

    if !phrase.is_empty() {
        if content.contains(phrase) {
            score += 80;
        }
        if tags.contains(phrase) {
            score += 100;
        }
    }

    for token in tokens {
        if content.contains(token) {
            score += 10;
        }
        if tags.contains(token) {
            score += 14;
        }
        if memory_project.contains(token) {
            score += 3;
        }
    }

    if score > 0 {
        score += (memory.confidence.clamp(0.0, 1.0) * 10.0) as i64;
        if memory.pinned {
            score += 25;
        }
        if let Some(pp) = project_path {
            if memory.project_path == pp {
                score += 5;
            } else if memory.project_path.is_empty() {
                score += 2;
            }
        }
    }

    score
}

pub fn search_memories(
    db: &MemoryDb,
    query: &str,
    user_id: &str,
    project_path: Option<&str>,
    limit: i64,
) -> Vec<Memory> {
    let tokens = search_tokens(query);
    if tokens.is_empty() {
        return vec![];
    }

    let conn = db.conn();
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(pp) = project_path
    {
        (
            "SELECT id, user_id, project_path, content, tags, created_at, updated_at,
             provenance_json,confidence,source_kind,source_id,pinned,superseded_by,retention,expires_at
             FROM memories
             WHERE user_id = ?1 AND superseded_by IS NULL AND (expires_at IS NULL OR expires_at > unixepoch())
             AND (project_path = ?2 OR project_path = '')",
            vec![
                Box::new(user_id.to_string()) as Box<dyn rusqlite::types::ToSql>,
                Box::new(pp.to_string()),
            ],
        )
    } else {
        (
            "SELECT id, user_id, project_path, content, tags, created_at, updated_at,
             provenance_json,confidence,source_kind,source_id,pinned,superseded_by,retention,expires_at
             FROM memories WHERE user_id = ?1 AND superseded_by IS NULL
             AND (expires_at IS NULL OR expires_at > unixepoch())",
            vec![Box::new(user_id.to_string()) as Box<dyn rusqlite::types::ToSql>],
        )
    };

    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let memories: Vec<Memory> = match stmt.query_map(params_refs.as_slice(), row_to_memory) {
        Ok(r) => r.filter_map(|r| r.ok()).collect(),
        Err(_) => return vec![],
    };

    let phrase = query.trim().to_lowercase();
    let mut scored: Vec<(i64, Memory)> = memories
        .into_iter()
        .filter_map(|memory| {
            let score = memory_search_score(&memory, &phrase, &tokens, project_path);
            (score > 0).then_some((score, memory))
        })
        .collect();

    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| right.created_at.cmp(&left.created_at))
    });

    scored
        .into_iter()
        .take(limit.max(0) as usize)
        .map(|(_, memory)| memory)
        .collect()
}

pub fn list_memories(
    db: &MemoryDb,
    user_id: &str,
    project_path: Option<&str>,
    limit: i64,
) -> Vec<Memory> {
    let conn = db.conn();

    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(pp) = project_path
    {
        (
            "SELECT id, user_id, project_path, content, tags, created_at, updated_at,
             provenance_json,confidence,source_kind,source_id,pinned,superseded_by,retention,expires_at
             FROM memories WHERE user_id = ?1 AND project_path = ?2
             AND superseded_by IS NULL AND (expires_at IS NULL OR expires_at > unixepoch())
             ORDER BY pinned DESC, updated_at DESC LIMIT ?3",
            vec![
                Box::new(user_id.to_string()) as Box<dyn rusqlite::types::ToSql>,
                Box::new(pp.to_string()),
                Box::new(limit),
            ],
        )
    } else {
        (
            "SELECT id, user_id, project_path, content, tags, created_at, updated_at,
             provenance_json,confidence,source_kind,source_id,pinned,superseded_by,retention,expires_at
             FROM memories WHERE user_id = ?1 AND superseded_by IS NULL
             AND (expires_at IS NULL OR expires_at > unixepoch())
             ORDER BY pinned DESC, updated_at DESC LIMIT ?2",
            vec![
                Box::new(user_id.to_string()) as Box<dyn rusqlite::types::ToSql>,
                Box::new(limit),
            ],
        )
    };

    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    match stmt.query_map(params_refs.as_slice(), row_to_memory) {
        Ok(r) => r.filter_map(|r| r.ok()).collect(),
        Err(_) => vec![],
    }
}

pub fn update_memory(
    db: &MemoryDb,
    id: &str,
    user_id: &str,
    content: &str,
    tags: &[String],
) -> Result<(), String> {
    let conn = db.conn();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string());

    let rows = conn
        .execute(
            "UPDATE memories SET content = ?1, tags = ?2, updated_at = ?3
             WHERE id = ?4 AND user_id = ?5",
            rusqlite::params![content, tags_json, now, id, user_id],
        )
        .map_err(|e| e.to_string())?;

    if rows == 0 {
        return Err("Memory not found or not owned by user".to_string());
    }
    Ok(())
}

pub fn delete_memory(db: &MemoryDb, id: &str, user_id: &str) -> Result<(), String> {
    let conn = db.conn();
    let rows = conn
        .execute(
            "DELETE FROM memories WHERE id = ?1 AND user_id = ?2",
            rusqlite::params![id, user_id],
        )
        .map_err(|e| e.to_string())?;

    if rows == 0 {
        return Err("Memory not found or not owned by user".to_string());
    }
    Ok(())
}

pub fn clear_memories(
    db: &MemoryDb,
    user_id: &str,
    project_path: Option<&str>,
) -> Result<i64, String> {
    let conn = db.conn();
    let rows = if let Some(pp) = project_path {
        conn.execute(
            "DELETE FROM memories WHERE user_id = ?1 AND project_path = ?2",
            rusqlite::params![user_id, pp],
        )
    } else {
        conn.execute(
            "DELETE FROM memories WHERE user_id = ?1",
            rusqlite::params![user_id],
        )
    }
    .map_err(|e| e.to_string())?;

    Ok(rows as i64)
}
