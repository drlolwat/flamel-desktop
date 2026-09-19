//! Durable execution ownership and delayed recovery, independent of browsers.
use super::{Db, realtime::DurableChatRun};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    for (name, definition) in [
        ("retry_at", "TEXT"),
        ("failure_kind", "TEXT"),
        ("consecutive_failures", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('chat_runs') WHERE name=?1)",
            [name],
            |r| r.get(0),
        )?;
        if !exists {
            conn.execute_batch(&format!(
                "ALTER TABLE chat_runs ADD COLUMN {name} {definition}"
            ))?;
        }
    }
    // One-time reconciliation runs before any providers are admitted. Never
    // replay a historical waiting row that has been superseded by newer work.
    let installed: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='idx_chat_runs_one_executor')",
        [],
        |r| r.get(0),
    )?;
    if !installed {
        conn.execute_batch("SAVEPOINT execution_ownership;
            UPDATE chat_runs SET state='cancelled',error='Superseded historical execution; transcript and answers preserved',
                finished_at=COALESCE(finished_at,updated_at),lease_owner=NULL,lease_expires_at=NULL
            WHERE state IN ('preparing','running','recovering','waiting_for_input') AND EXISTS(
                SELECT 1 FROM chat_runs newer WHERE newer.session_id=chat_runs.session_id
                AND (newer.created_at>chat_runs.created_at OR
                    (newer.created_at=chat_runs.created_at AND newer.rowid>chat_runs.rowid))
                AND newer.started_at IS NOT NULL);
            UPDATE chat_runs SET state='failed',error='Historical input wait ended without an executing owner; work preserved',
                finished_at=COALESCE(finished_at,updated_at)
            WHERE state='waiting_for_input' AND NOT EXISTS(SELECT 1 FROM input_requests i
                WHERE i.run_id=chat_runs.id AND i.status IN ('pending','answered'));
            UPDATE chat_runs SET state='queued',lease_owner=NULL,lease_expires_at=NULL
            WHERE state IN ('preparing','running','recovering') AND rowid NOT IN (
                SELECT MAX(rowid) FROM chat_runs WHERE state IN ('preparing','running','recovering') GROUP BY session_id);
            UPDATE input_requests SET status='cancelled',cancelled_at=COALESCE(cancelled_at,updated_at)
            WHERE status='pending' AND run_id IN (SELECT id FROM chat_runs WHERE state='cancelled' AND error='Superseded historical execution; transcript and answers preserved');
            CREATE UNIQUE INDEX idx_chat_runs_one_executor ON chat_runs(session_id)
                WHERE state IN ('preparing','running','recovering');
            RELEASE execution_ownership;")?;
    }
    Ok(())
}

pub fn activate(db: &Db, run: &str, session: &str) -> rusqlite::Result<Option<String>> {
    let owner = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let changed = db.conn().execute(
        "UPDATE chat_runs SET state='running',lease_owner=?3,
        lease_expires_at=?4,started_at=COALESCE(started_at,?5),updated_at=?5,retry_at=NULL
        WHERE id=?1 AND session_id=?2 AND state IN ('preparing','running') AND lease_owner IS NULL",
        params![
            run,
            session,
            owner,
            (now + chrono::Duration::seconds(90)).to_rfc3339(),
            now.to_rfc3339()
        ],
    )?;
    Ok((changed == 1).then_some(owner))
}

pub fn owns(db: &Db, run: &str, owner: &str) -> bool {
    db.conn()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM chat_runs WHERE id=?1 AND lease_owner=?2
        AND state IN ('preparing','running','recovering'))",
            params![run, owner],
            |r| r.get(0),
        )
        .unwrap_or(false)
}

pub fn heartbeat(db: &Db, run: &str, owner: &str) -> rusqlite::Result<bool> {
    Ok(db.conn().execute("UPDATE chat_runs SET lease_expires_at=?3 WHERE id=?1 AND lease_owner=?2 AND state='running'",
        params![run,owner,(chrono::Utc::now()+chrono::Duration::seconds(90)).to_rfc3339()])? == 1)
}

pub fn retry_delay(failures: u32) -> i64 {
    match failures {
        0 | 1 => 10,
        2 => 30,
        3 => 120,
        _ => 300,
    }
}

pub fn park(db: &Db, run: &str, owner: &str, reason: &str) -> rusqlite::Result<Option<String>> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row: Option<(String, u32, bool)> = tx
        .query_row(
            "SELECT session_id,consecutive_failures,
        COALESCE(json_extract(request_json,'$.continuationCancelled'),0) FROM chat_runs
        WHERE id=?1 AND lease_owner=?2 AND state IN ('preparing','running','recovering')",
            params![run, owner],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((session, failures, cancelled)) = row else {
        return Ok(None);
    };
    let now = chrono::Utc::now();
    let retry = (now + chrono::Duration::seconds(retry_delay(failures + 1))).to_rfc3339();
    tx.execute("UPDATE chat_runs SET state=?3,error=?4,failure_kind='provider_failure',retry_at=?5,
        consecutive_failures=consecutive_failures+1,lease_owner=NULL,lease_expires_at=NULL,updated_at=?6,
        finished_at=CASE WHEN ?3='cancelled' THEN ?6 ELSE NULL END WHERE id=?1 AND lease_owner=?2",
        params![run,owner,if cancelled {"cancelled"} else {"recovering"},reason,if cancelled {None} else {Some(&retry)},now.to_rfc3339()])?;
    tx.execute(
        "UPDATE sessions SET continuity_status=?3,active_request_id=?2,last_error=?4,
        recovery_strategy='durable_retry',updated_at=?5 WHERE id=?1 AND
        (active_request_id=?2 OR active_request_id IS NULL)",
        params![
            session,
            run,
            if cancelled { "cancelled" } else { "recovering" },
            reason,
            now.to_rfc3339()
        ],
    )?;
    tx.commit()?;
    Ok((!cancelled).then_some(retry))
}

pub fn reset_failures(db: &Db, run: &str, owner: &str) {
    let _ = db.conn().execute(
        "UPDATE chat_runs SET consecutive_failures=0,error=NULL,failure_kind=NULL
        WHERE id=?1 AND lease_owner=?2",
        params![run, owner],
    );
}

/// A dropped preparation or expired execution must not strand durable work.
/// Live executors are excluded; their heartbeat and guarded finalizer own them.
pub fn recover_orphans(db: &Db, active: &[String]) -> rusqlite::Result<()> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = chrono::Utc::now();
    let mut stmt=tx.prepare("SELECT id,session_id FROM chat_runs WHERE state IN ('preparing','running') AND COALESCE(lease_expires_at,updated_at)<?1")?;
    let rows = stmt
        .query_map([(now - chrono::Duration::seconds(180)).to_rfc3339()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for (id, session) in rows {
        if active.contains(&session) {
            continue;
        }
        tx.execute("UPDATE chat_runs SET state='recovering',lease_owner=NULL,lease_expires_at=NULL,retry_at=?2,error='Execution owner disappeared; resuming saved work',updated_at=?2 WHERE id=?1",params![id,now.to_rfc3339()])?;
        tx.execute("UPDATE sessions SET continuity_status='recovering',active_request_id=?2,recovery_strategy='durable_retry' WHERE id=?1",params![session,id])?;
    }
    tx.commit()
}

/// Claim a due dispatch for sixty seconds. HTTP admission consumes that claim;
/// a lost response cannot demote a run that has already begun executing.
pub fn due(db: &Db, excluded_sessions: &[String]) -> rusqlite::Result<Vec<DurableChatRun>> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut stmt = tx.prepare("SELECT id,session_id,user_id,state,request_json FROM chat_runs
        WHERE state='recovering' AND lease_owner IS NULL AND (retry_at IS NULL OR retry_at<=?1)
        AND (lease_expires_at IS NULL OR lease_expires_at<=?1)
        AND COALESCE(json_extract(request_json,'$.continuationCancelled'),0)=0
        AND NOT EXISTS(SELECT 1 FROM input_requests i WHERE i.run_id=chat_runs.id AND i.status IN ('pending','answered'))
        ORDER BY updated_at LIMIT 32")?;
    let rows = stmt
        .query_map([&now], |r| {
            let raw: String = r.get(4)?;
            Ok(DurableChatRun {
                id: r.get(0)?,
                session_id: r.get(1)?,
                user_id: r.get(2)?,
                state: r.get(3)?,
                request: serde_json::from_str(&raw).unwrap_or(Value::Null),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    let mut result = Vec::new();
    for row in rows {
        if excluded_sessions.contains(&row.session_id) {
            continue;
        }
        tx.execute(
            "UPDATE chat_runs SET lease_expires_at=?2 WHERE id=?1",
            params![
                row.id,
                (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339()
            ],
        )?;
        result.push(row);
    }
    tx.commit()?;
    Ok(result)
}

pub fn recovery_metadata(db: &Db, run: Option<&str>) -> Value {
    run.and_then(|run| db.conn().query_row("SELECT retry_at,error FROM chat_runs WHERE id=?1", [run], |r| {
        Ok(json!({"retryAt":r.get::<_,Option<String>>(0)?,"recoveryReason":r.get::<_,Option<String>>(1)?,"completionManaged":"server"}))
    }).optional().ok().flatten()).unwrap_or_else(||json!({"completionManaged":"server"}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::realtime::{self, RunAdmission};
    fn seeded() -> Db {
        let db = Db::open(":memory:");
        db.conn().execute("INSERT INTO sessions(id,project_path,created_at,updated_at,user_id) VALUES('s','/tmp','now','now','u')", []).unwrap();
        assert_eq!(
            realtime::admit_run(&db, "r", "s", "u", &json!({"message":"finish this"})),
            RunAdmission::Start
        );
        db
    }
    #[test]
    fn one_owner_survives_questions_and_queued_messages() {
        let db = seeded();
        let owner = activate(&db, "r", "s").unwrap().unwrap();
        assert!(activate(&db, "r", "s").unwrap().is_none());
        crate::db::input_requests::create_input_request(
            &db,
            crate::db::input_requests::NewInputRequest {
                id: "q",
                run_id: Some("r"),
                session_id: "s",
                user_id: "u",
                request: &json!({"question":"Region?"}),
            },
        )
        .unwrap();
        assert_eq!(
            realtime::admit_run(&db, "next", "s", "u", &json!({})),
            RunAdmission::Queued
        );
        assert!(owns(&db, "r", &owner));
        assert!(
            realtime::finish_owned_run_and_claim_next(
                &db,
                "r",
                "s",
                "completed",
                None,
                Some("stale")
            )
            .unwrap()
            .is_none()
        );
        assert!(owns(&db, "r", &owner));
        assert!(
            realtime::finish_owned_run_and_claim_next(
                &db,
                "r",
                "s",
                "completed",
                None,
                Some(&owner)
            )
            .unwrap()
            .is_none()
        );
        assert!(realtime::claim_next_queued_run(&db, "s").is_none());
        assert!(!owns(&db, "r", &owner));
    }
    #[test]
    fn failures_remain_recoverable_beyond_immediate_retry_limit() {
        let db = seeded();
        for failure in 1..=9 {
            let owner = activate(&db, "r", "s").unwrap().unwrap();
            assert!(
                park(&db, "r", &owner, "provider disconnected")
                    .unwrap()
                    .is_some()
            );
            assert!(!owns(&db, "r", &owner));
            assert!(due(&db, &[]).unwrap().is_empty());
            db.conn()
                .execute(
                    "UPDATE chat_runs SET retry_at='2000-01-01' WHERE id='r'",
                    [],
                )
                .unwrap();
            assert!(due(&db, &["s".into()]).unwrap().is_empty());
            let runs = due(&db, &[]).unwrap();
            assert_eq!(runs.len(), 1);
            assert!(
                due(&db, &[]).unwrap().is_empty(),
                "dispatcher lease prevents duplicate dispatch"
            );
            assert_eq!(
                realtime::admit_run(&db, "r", "s", "u", &json!({"message":"finish this"})),
                RunAdmission::Start
            );
            let count: u32 = db
                .conn()
                .query_row(
                    "SELECT consecutive_failures FROM chat_runs WHERE id='r'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, failure);
        }
        assert_eq!(
            [
                retry_delay(1),
                retry_delay(2),
                retry_delay(3),
                retry_delay(9)
            ],
            [10, 30, 120, 300]
        );
    }
    #[test]
    fn stop_cancels_a_parked_run_and_new_work_can_start() {
        let db = seeded();
        let owner = activate(&db, "r", "s").unwrap().unwrap();
        park(&db, "r", &owner, "offline").unwrap();
        db.conn().execute("UPDATE chat_runs SET state='cancelled',retry_at=NULL,request_json=json_set(request_json,'$.continuationCancelled',json('true')) WHERE id='r'",[]).unwrap();
        assert!(due(&db, &[]).unwrap().is_empty());
        assert_eq!(
            realtime::admit_run(&db, "next", "s", "u", &json!({})),
            RunAdmission::Start
        );
    }
}
