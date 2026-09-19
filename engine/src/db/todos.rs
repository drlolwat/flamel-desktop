use crate::db::Db;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub id: String,
    pub project_path: String,
    pub text: String,
    pub done: bool,
    pub position: i64,
    pub created_at: String,
    pub updated_at: String,
}

fn row_to_todo(row: &rusqlite::Row) -> rusqlite::Result<TodoItem> {
    Ok(TodoItem {
        id: row.get("id")?,
        project_path: row.get("project_path")?,
        text: row.get("text")?,
        done: row.get::<_, i64>("done")? != 0,
        position: row.get("position")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn list_todos(db: &Db, project_path: &str, user_id: Option<&str>) -> Vec<TodoItem> {
    let conn = db.conn();
    match user_id {
        Some(uid) => {
            let mut stmt = conn
                .prepare("SELECT * FROM todos WHERE project_path = ?1 AND user_id = ?2 ORDER BY position ASC, created_at ASC")
                .unwrap();
            stmt.query_map(rusqlite::params![project_path, uid], row_to_todo)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
        None => {
            let mut stmt = conn
                .prepare("SELECT * FROM todos WHERE project_path = ?1 ORDER BY position ASC, created_at ASC")
                .unwrap();
            stmt.query_map([project_path], row_to_todo)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
    }
}

pub fn create_todo(db: &Db, project_path: &str, text: &str, user_id: &str) -> TodoItem {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let max_pos: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), -1) FROM todos WHERE project_path = ?1",
            [project_path],
            |row| row.get(0),
        )
        .unwrap_or(-1);
    let position = max_pos + 1;
    conn.execute(
        "INSERT INTO todos (id, project_path, text, done, position, created_at, updated_at, user_id) VALUES (?1, ?2, ?3, 0, ?4, ?5, ?5, ?6)",
        rusqlite::params![id, project_path, text, position, now, user_id],
    )
    .unwrap();
    TodoItem {
        id,
        project_path: project_path.to_string(),
        text: text.to_string(),
        done: false,
        position,
        created_at: now.clone(),
        updated_at: now,
    }
}

pub fn get_todo_user_id(db: &Db, id: &str) -> Option<String> {
    let conn = db.conn();
    conn.query_row("SELECT user_id FROM todos WHERE id = ?1", [id], |row| {
        row.get::<_, Option<String>>(0)
    })
    .ok()
    .flatten()
}

pub fn update_todo(db: &Db, id: &str, done: Option<bool>, text: Option<&str>) -> Option<TodoItem> {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(d) = done {
        conn.execute(
            "UPDATE todos SET done = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![d as i64, now, id],
        )
        .ok();
    }
    if let Some(t) = text {
        conn.execute(
            "UPDATE todos SET text = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![t, now, id],
        )
        .ok();
    }
    conn.query_row("SELECT * FROM todos WHERE id = ?1", [id], row_to_todo)
        .ok()
}

pub fn delete_todo(db: &Db, id: &str) {
    let conn = db.conn();
    conn.execute("DELETE FROM todos WHERE id = ?1", [id]).ok();
}

pub fn clear_done_todos(db: &Db, project_path: &str, user_id: Option<&str>) {
    let conn = db.conn();
    match user_id {
        Some(uid) => {
            conn.execute(
                "DELETE FROM todos WHERE project_path = ?1 AND done = 1 AND user_id = ?2",
                rusqlite::params![project_path, uid],
            )
            .ok();
        }
        None => {
            conn.execute(
                "DELETE FROM todos WHERE project_path = ?1 AND done = 1",
                [project_path],
            )
            .ok();
        }
    }
}
