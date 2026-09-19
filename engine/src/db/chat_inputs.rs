//! Durable receipts for guidance to an existing run, separate from queued runs.
use super::Db;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS chat_inputs (
        id TEXT PRIMARY KEY, session_id TEXT NOT NULL, user_id TEXT NOT NULL,
        request_json TEXT NOT NULL, delivery TEXT NOT NULL, note TEXT,
        created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS idx_chat_inputs_session ON chat_inputs(user_id, session_id, created_at);")
}

pub enum InputAdmission {
    New,
    Existing(Value),
    Conflict,
}

pub fn admit(
    db: &Db,
    id: &str,
    sid: &str,
    uid: &str,
    request: &Value,
) -> rusqlite::Result<InputAdmission> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let existing = tx.query_row("SELECT session_id, user_id, request_json, delivery, note, updated_at FROM chat_inputs WHERE id=?1",
        [id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,String>(5)?))).optional()?;
    if let Some((session, user, stored, delivery, note, updated)) = existing {
        if session != sid
            || user != uid
            || serde_json::from_str::<Value>(&stored).ok().as_ref() != Some(request)
        {
            return Ok(InputAdmission::Conflict);
        }
        let mut existing = receipt(id, sid, request, &delivery, note.as_deref());
        existing["updatedAt"] = json!(updated);
        return Ok(InputAdmission::Existing(existing));
    }
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM chat_runs WHERE id=?1)",
        [id],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(InputAdmission::Conflict);
    }
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute(
        "INSERT INTO chat_inputs VALUES (?1,?2,?3,?4,'waiting',NULL,?5,?5)",
        params![id, sid, uid, request.to_string(), now],
    )?;
    tx.commit()?;
    Ok(InputAdmission::New)
}

pub fn receipt(id: &str, sid: &str, request: &Value, delivery: &str, note: Option<&str>) -> Value {
    json!({"type":"input_delivery", "accepted":true, "requestId":id, "sessionId":sid,
        "message":request.get("message"), "delivery":delivery, "note":note,
        "updatedAt":chrono::Utc::now().to_rfc3339()})
}

pub fn complete(db: &Db, id: &str, delivery: &str, note: Option<&str>) -> rusqlite::Result<()> {
    let changed = db.conn().execute(
        "UPDATE chat_inputs SET delivery=?2,note=?3,updated_at=?4 WHERE id=?1",
        params![id, delivery, note, chrono::Utc::now().to_rfc3339()],
    )?;
    if changed != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

pub fn list(db: &Db, sid: &str, uid: &str) -> rusqlite::Result<Vec<Value>> {
    let conn = db.conn();
    let mut query = conn.prepare(
        "SELECT i.id,i.request_json,i.delivery,i.note,
                MAX(i.updated_at, COALESCE(r.updated_at,i.updated_at)),r.started_at,r.state
         FROM chat_inputs i LEFT JOIN chat_runs r
           ON r.id=i.id AND r.session_id=i.session_id AND r.user_id=i.user_id
         WHERE i.session_id=?1 AND i.user_id=?2 ORDER BY i.created_at DESC LIMIT 100",
    )?;
    query
        .query_map(params![sid, uid], |r| {
            let request =
                serde_json::from_str::<Value>(&r.get::<_, String>(1)?).unwrap_or(Value::Null);
            let mut value = receipt(
                &r.get::<_, String>(0)?,
                sid,
                &request,
                &r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?.as_deref(),
            );
            value["updatedAt"] = json!(r.get::<_, String>(4)?);
            // A queued receipt records admission, not the lifetime of its run.
            // Project durable run progress, including pre-upgrade receipts, without
            // rewriting history or dispatching the saved input again.
            let started = r.get::<_, Option<String>>(5)?;
            value["runStartedAt"] = json!(started);
            if value["delivery"] == "queued"
                && started.is_none()
                && matches!(
                    r.get::<_, Option<String>>(6)?.as_deref(),
                    Some("cancelled" | "failed")
                )
            {
                value["delivery"] = json!("failed");
                value["note"] = json!("The queued run ended before starting.");
            }
            Ok(value)
        })?
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_receipts_follow_owned_run_progress_without_rewriting_admission() {
        let db = Db::open(":memory:");
        db.conn()
            .execute_batch(
                "INSERT INTO sessions (id,project_path,created_at,updated_at,user_id)
            VALUES ('session','/repo','2026-09-08','2026-09-08','user')",
            )
            .unwrap();
        let body = json!({"message":"queued guidance"});
        for (id, state, started, owner) in [
            ("pending", "queued", None, "user"),
            ("running", "running", Some("2026-09-08T02:00:00Z"), "user"),
            (
                "completed",
                "completed",
                Some("2026-09-08T02:00:00Z"),
                "user",
            ),
            ("cancelled", "cancelled", None, "user"),
            (
                "foreign",
                "completed",
                Some("2026-09-08T02:00:00Z"),
                "other",
            ),
        ] {
            admit(&db, id, "session", "user", &body).unwrap();
            complete(&db, id, "queued", None).unwrap();
            db.conn()
                .execute(
                    "INSERT INTO chat_runs
                (id,session_id,user_id,state,request_json,created_at,updated_at,started_at)
                VALUES (?1,'session',?2,?3,'{}','2026-09-08','2099-01-01',?4)",
                    params![id, owner, state, started],
                )
                .unwrap();
        }
        let inputs = list(&db, "session", "user").unwrap();
        let get = |id: &str| inputs.iter().find(|v| v["requestId"] == id).unwrap();
        for id in ["running", "completed"] {
            assert_eq!(get(id)["runStartedAt"], "2026-09-08T02:00:00Z");
            assert_eq!(get(id)["updatedAt"], "2099-01-01");
        }
        for id in ["pending", "foreign"] {
            assert!(get(id)["runStartedAt"].is_null());
            assert_eq!(get(id)["delivery"], "queued");
        }
        assert_eq!(get("cancelled")["delivery"], "failed");
        assert!(list(&db, "session", "other").unwrap().is_empty());
        let unchanged: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM chat_inputs WHERE delivery='queued'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unchanged, 5);
        db.conn()
            .execute(
                "UPDATE chat_runs SET finished_at='2000-01-01' WHERE state='completed'",
                [],
            )
            .unwrap();
        super::super::realtime::maintain_realtime_journal(&db, 100);
        assert!(
            list(&db, "session", "user")
                .unwrap()
                .iter()
                .any(|v| v["requestId"] == "completed" && v["runStartedAt"].is_string())
        );
    }

    #[test]
    fn reopening_keeps_receipts_and_restart_never_blindly_resends() {
        let dir = std::env::temp_dir().join(format!("flamel-inputs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("flamel.db");
        let body = json!({"message":"keep going"});
        {
            let db = Db::open(path.to_str().unwrap());
            assert!(matches!(
                admit(&db, "waiting", "session", "user", &body).unwrap(),
                InputAdmission::New
            ));
            assert!(matches!(
                admit(&db, "sent", "session", "user", &body).unwrap(),
                InputAdmission::New
            ));
            complete(&db, "sent", "sent", None).unwrap();
        }
        {
            let db = Db::open(path.to_str().unwrap());
            super::super::realtime::reconcile_interrupted_runs(&db);
            assert!(
                matches!(admit(&db,"waiting","session","user",&body).unwrap(),InputAdmission::Existing(value) if value["delivery"] == "uncertain")
            );
            assert!(
                matches!(admit(&db,"sent","session","user",&body).unwrap(),InputAdmission::Existing(value) if value["delivery"] == "sent")
            );
            assert_eq!(list(&db, "session", "user").unwrap().len(), 2);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn receipt_is_durable_owned_and_not_resubmitted() {
        let db = Db::open(":memory:");
        let body = json!({"message":"keep going"});
        assert!(matches!(
            admit(&db, "id", "session", "user", &body).unwrap(),
            InputAdmission::New
        ));
        complete(
            &db,
            "id",
            "uncertain",
            Some("Connection lost; check before retrying."),
        )
        .unwrap();
        assert!(
            matches!(admit(&db,"id","session","user",&body).unwrap(),InputAdmission::Existing(value) if value["delivery"] == "uncertain")
        );
        assert!(matches!(
            admit(&db, "id", "session", "other", &body).unwrap(),
            InputAdmission::Conflict
        ));
        assert!(matches!(
            admit(
                &db,
                "id",
                "session",
                "user",
                &json!({"message":"different"})
            )
            .unwrap(),
            InputAdmission::Conflict
        ));
        assert!(list(&db, "session", "other").unwrap().is_empty());
        assert_eq!(list(&db, "session", "user").unwrap().len(), 1);
    }
}
