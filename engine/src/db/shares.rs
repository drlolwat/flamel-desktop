use crate::db::Db;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareToken {
    pub token: String,
    pub session_id: String,
    pub created_at: String,
}

pub fn get_share_by_session_id(db: &Db, session_id: &str) -> Option<ShareToken> {
    let conn = db.conn();
    conn.query_row(
        "SELECT * FROM shares WHERE session_id = ?1 LIMIT 1",
        [session_id],
        |row| {
            Ok(ShareToken {
                token: row.get("token")?,
                session_id: row.get("session_id")?,
                created_at: row.get("created_at")?,
            })
        },
    )
    .ok()
}
