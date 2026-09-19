//! Read-only migration of legacy Desktop ownership labels.
use crate::db::Db;
pub struct FlamelUser {
    pub flamel_user_id: String,
    pub email: String,
}
pub fn list_flamel_users(db: &Db) -> Vec<FlamelUser> {
    let conn = db.conn();
    let Ok(mut statement) =
        conn.prepare("SELECT flamel_user_id,email FROM flamel_dev_users ORDER BY created_at")
    else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok(FlamelUser {
            flamel_user_id: row.get(0)?,
            email: row.get(1)?,
        })
    }) else {
        return Vec::new();
    };
    rows.filter_map(Result::ok).collect()
}
