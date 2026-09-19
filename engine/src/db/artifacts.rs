use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Db;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedArtifact {
    pub id: String,
    pub user_id: String,
    pub session_id: String,
    pub tool_call_id: Option<String>,
    pub kind: String,
    pub name: String,
    pub media_type: String,
    pub byte_size: i64,
    pub sha256: String,
    #[serde(skip_serializing)]
    pub storage_path: String,
    pub metadata: Value,
    pub created_at: String,
}

impl GeneratedArtifact {
    pub fn public_ref(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "kind": self.kind,
            "name": self.name,
            "mediaType": self.media_type,
            "byteSize": self.byte_size,
            "sha256": self.sha256,
            "createdAt": self.created_at,
            "contentUrl": format!("/api/artifacts/{}/content", self.id),
            "downloadUrl": format!("/api/artifacts/{}/content?download=1", self.id),
            "toolCallId": self.tool_call_id,
            "metadata": self.metadata,
        })
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GeneratedArtifact> {
    let metadata: String = row.get(10)?;
    Ok(GeneratedArtifact {
        id: row.get(0)?,
        user_id: row.get(1)?,
        session_id: row.get(2)?,
        tool_call_id: row.get(3)?,
        kind: row.get(4)?,
        name: row.get(5)?,
        media_type: row.get(6)?,
        byte_size: row.get(7)?,
        sha256: row.get(8)?,
        storage_path: row.get(9)?,
        metadata: serde_json::from_str(&metadata).unwrap_or(Value::Object(Default::default())),
        created_at: row.get(11)?,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn insert(
    db: &Db,
    id: &str,
    user_id: &str,
    session_id: &str,
    tool_call_id: Option<&str>,
    kind: &str,
    name: &str,
    media_type: &str,
    byte_size: i64,
    sha256: &str,
    storage_path: &str,
    metadata: &Value,
) -> Result<GeneratedArtifact, String> {
    let created_at = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO generated_artifacts
             (id,user_id,session_id,tool_call_id,kind,name,media_type,byte_size,sha256,storage_path,metadata,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                id,
                user_id,
                session_id,
                tool_call_id,
                kind,
                name,
                media_type,
                byte_size,
                sha256,
                storage_path,
                metadata.to_string(),
                created_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    get_for_user(db, id, user_id)?.ok_or_else(|| "artifact was not persisted".to_string())
}

pub fn get_for_user(db: &Db, id: &str, user_id: &str) -> Result<Option<GeneratedArtifact>, String> {
    db.conn()
        .query_row(
            "SELECT id,user_id,session_id,tool_call_id,kind,name,media_type,byte_size,sha256,storage_path,metadata,created_at
             FROM generated_artifacts WHERE id=?1 AND user_id=?2",
            params![id, user_id],
            map_row,
        )
        .optional()
        .map_err(|error| error.to_string())
}

pub fn list_for_session(
    db: &Db,
    user_id: &str,
    session_id: &str,
) -> Result<Vec<GeneratedArtifact>, String> {
    let conn = db.conn();
    let mut statement = conn
        .prepare(
            "SELECT id,user_id,session_id,tool_call_id,kind,name,media_type,byte_size,sha256,storage_path,metadata,created_at
             FROM generated_artifacts WHERE user_id=?1 AND session_id=?2 ORDER BY created_at",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![user_id, session_id], map_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}
