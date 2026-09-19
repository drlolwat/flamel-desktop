use crate::db::Db;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPrompt {
    pub id: String,
    pub title: String,
    pub content: String,
    pub created_at: String,
    pub use_count: i64,
}

pub fn list_prompts(db: &Db, user_id: Option<&str>) -> Vec<SavedPrompt> {
    let conn = db.conn();
    let row_to_prompt = |row: &rusqlite::Row| {
        Ok(SavedPrompt {
            id: row.get("id")?,
            title: row.get("title")?,
            content: row.get("content")?,
            created_at: row.get("created_at")?,
            use_count: row.get::<_, Option<i64>>("use_count")?.unwrap_or(0),
        })
    };
    match user_id {
        Some(uid) => {
            let mut stmt = conn
                .prepare("SELECT * FROM prompts WHERE user_id = ?1 ORDER BY use_count DESC, created_at DESC")
                .unwrap();
            stmt.query_map(rusqlite::params![uid], row_to_prompt)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
        None => {
            let mut stmt = conn
                .prepare("SELECT * FROM prompts ORDER BY use_count DESC, created_at DESC")
                .unwrap();
            stmt.query_map([], row_to_prompt)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
    }
}

pub fn create_prompt(db: &Db, title: &str, content: &str, user_id: &str) -> SavedPrompt {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO prompts (id, title, content, created_at, user_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, title, content, now, user_id],
    )
    .unwrap();
    SavedPrompt {
        id,
        title: title.to_string(),
        content: content.to_string(),
        created_at: now,
        use_count: 0,
    }
}

pub fn get_prompt_user_id(db: &Db, id: &str) -> Option<String> {
    let conn = db.conn();
    conn.query_row("SELECT user_id FROM prompts WHERE id = ?1", [id], |row| {
        row.get::<_, Option<String>>(0)
    })
    .ok()
    .flatten()
}

pub fn delete_prompt(db: &Db, id: &str) {
    let conn = db.conn();
    conn.execute("DELETE FROM prompts WHERE id = ?1", [id]).ok();
}

pub fn increment_prompt_use(db: &Db, id: &str) {
    let conn = db.conn();
    conn.execute(
        "UPDATE prompts SET use_count = use_count + 1 WHERE id = ?1",
        [id],
    )
    .ok();
}
