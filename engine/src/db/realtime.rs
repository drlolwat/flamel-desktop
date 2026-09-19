use crate::db::Db;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub sequence: u64,
    pub event_id: String,
    pub session_id: String,
    pub user_id: String,
    pub run_id: Option<String>,
    pub kind: String,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct DurableChatRun {
    pub id: String,
    pub session_id: String,
    pub user_id: String,
    pub state: String,
    pub request: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedChatRun {
    pub id: String,
    pub session_id: String,
    pub message: String,
    pub hidden_continuation: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAdmission {
    Start,
    Queued,
    Duplicate,
    Conflict,
    StorageUnavailable,
}

pub const RECOVERY_MESSAGE: &str = "Continue the interrupted response from the durable session context. Complete the user's pending request without repeating finished work.";

fn matches_saved_request(
    state: &str,
    stored: &str,
    incoming: &Value,
    run_id: &str,
    session_id: &str,
) -> bool {
    let Ok(mut expected) = serde_json::from_str::<Value>(stored) else {
        return false;
    };
    if &expected == incoming {
        return true;
    }
    let Some(object) = expected.as_object_mut() else {
        return false;
    };
    // Older rows can predate canonical request/session IDs in the body. The
    // dispatcher fills these from the immutable owned run record.
    object.insert("requestId".into(), serde_json::json!(run_id));
    object.insert("sessionId".into(), serde_json::json!(session_id));
    if &expected == incoming {
        return true;
    }
    if state != "recovering" || expected.get("inputRecovery").is_some() {
        return false;
    }
    expected["message"] = serde_json::json!(RECOVERY_MESSAGE);
    expected["hiddenContinuation"] = serde_json::json!(true);
    &expected == incoming
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuedRunUpdate {
    Updated { session_id: String },
    NotFound,
    NotQueued,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RealtimeMaintenance {
    pub superseded_messages: usize,
    pub expired_events: usize,
    pub expired_runs: usize,
}

/// Incrementally trim the reconnect journal without touching the durable chat
/// transcript or lossless context ledger. Small batches keep the single SQLite
/// writer available to active provider turns even when an older installation
/// accumulated millions of cumulative streaming snapshots.
pub fn maintain_realtime_journal(db: &Db, batch_size: usize) -> RealtimeMaintenance {
    let batch_size = batch_size.clamp(1, 10_000) as i64;
    let terminal_event_cutoff = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();
    let terminal_run_cutoff = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
    let mut conn = db.conn();
    let Ok(tx) = conn.transaction_with_behavior(TransactionBehavior::Immediate) else {
        return RealtimeMaintenance::default();
    };

    let superseded_messages = tx
        .execute(
            "DELETE FROM session_events WHERE sequence IN (
                SELECT older.sequence
                FROM session_events older
                WHERE older.kind='message' AND older.run_id IS NOT NULL
                  AND EXISTS (
                    SELECT 1 FROM session_events newer
                    WHERE newer.run_id=older.run_id
                      AND newer.kind='message'
                      AND newer.sequence>older.sequence
                  )
                LIMIT ?1
             )",
            [batch_size],
        )
        .unwrap_or(0);

    let expired_events = tx
        .execute(
            "DELETE FROM session_events WHERE sequence IN (
                SELECT event.sequence
                FROM session_events event
                JOIN chat_runs run ON run.id=event.run_id
                WHERE run.state IN ('completed', 'failed', 'cancelled')
                  AND run.finished_at IS NOT NULL
                  AND run.finished_at<?1
                ORDER BY event.sequence ASC
                LIMIT ?2
             )",
            params![terminal_event_cutoff, batch_size],
        )
        .unwrap_or(0);

    let expired_runs = tx
        .execute(
            "DELETE FROM chat_runs
             WHERE id IN (
                SELECT run.id FROM chat_runs run
                WHERE run.state IN ('completed', 'failed', 'cancelled')
                  AND run.finished_at IS NOT NULL
                  AND run.finished_at<?1
                  AND NOT EXISTS (
                    SELECT 1 FROM session_events event WHERE event.run_id=run.id
                  )
                  AND NOT EXISTS (
                    SELECT 1 FROM chat_inputs input WHERE input.id=run.id
                      AND input.session_id=run.session_id AND input.user_id=run.user_id
                  )
                ORDER BY run.finished_at ASC
                LIMIT ?2
             )",
            params![terminal_run_cutoff, batch_size],
        )
        .unwrap_or(0);

    if tx.commit().is_err() {
        return RealtimeMaintenance::default();
    }
    RealtimeMaintenance {
        superseded_messages,
        expired_events,
        expired_runs,
    }
}

pub fn checkpoint_realtime_wal(db: &Db) {
    let conn = db.conn();
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE); PRAGMA optimize;");
}

pub fn append_event(
    db: &Db,
    session_id: &str,
    run_id: Option<&str>,
    kind: &str,
    payload: &Value,
) -> Option<SessionEvent> {
    let mut conn = db.conn();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .ok()?;
    let user_id: String = tx
        .query_row(
            "SELECT COALESCE(user_id, '') FROM sessions WHERE id=?1",
            [session_id],
            |row| row.get(0),
        )
        .ok()?;
    if user_id.is_empty() {
        return None;
    }
    let event_id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let payload_json = serde_json::to_string(payload).ok()?;
    tx.execute(
        "INSERT INTO session_events
         (event_id, session_id, user_id, run_id, kind, payload, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            &event_id,
            session_id,
            &user_id,
            run_id,
            kind,
            payload_json,
            &created_at
        ],
    )
    .ok()?;
    let sequence = tx.last_insert_rowid() as u64;

    // Chat `message` events are cumulative snapshots, not independent messages.
    // Retaining every intermediate copy makes reconnect replay quadratic and can
    // grow the database by gigabytes during a single long provider turn. Keep
    // only the newest snapshot for each durable run while preserving every
    // status, tool, question, error, and terminal event.
    if kind == "message"
        && let Some(run_id) = run_id
    {
        tx.execute(
            "DELETE FROM session_events
             WHERE session_id=?1 AND run_id=?2 AND kind='message' AND sequence<>?3",
            params![session_id, run_id, sequence as i64],
        )
        .ok()?;
    }
    if let Some(run_id) = run_id {
        tx.execute(
            "UPDATE chat_runs SET last_event_sequence=?2, updated_at=?3 WHERE id=?1",
            params![run_id, sequence as i64, &created_at],
        )
        .ok()?;
    }
    tx.commit().ok()?;
    Some(SessionEvent {
        sequence,
        event_id,
        session_id: session_id.to_string(),
        user_id,
        run_id: run_id.map(str::to_string),
        kind: kind.to_string(),
        payload: payload.clone(),
        created_at,
    })
}

pub fn max_sequence(db: &Db, user_id: &str, session_id: &str) -> u64 {
    db.conn()
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM session_events
             WHERE user_id=?1 AND session_id=?2",
            params![user_id, session_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as u64
}

pub fn first_sequence_for_run(
    db: &Db,
    user_id: &str,
    session_id: &str,
    run_id: &str,
) -> Option<u64> {
    db.conn()
        .query_row(
            "SELECT MIN(sequence) FROM session_events
             WHERE user_id=?1 AND session_id=?2 AND run_id=?3",
            params![user_id, session_id, run_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten()
        .map(|sequence| sequence as u64)
}

pub fn replay(
    db: &Db,
    user_id: &str,
    session_id: &str,
    after_sequence: u64,
    through_sequence: u64,
) -> Vec<SessionEvent> {
    let conn = db.conn();
    let mut statement = match conn.prepare(
        "SELECT sequence, event_id, session_id, user_id, run_id, kind, payload, created_at
         FROM session_events
         WHERE user_id=?1 AND session_id=?2 AND sequence>?3 AND sequence<=?4
         ORDER BY sequence ASC",
    ) {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };
    statement
        .query_map(
            params![
                user_id,
                session_id,
                after_sequence as i64,
                through_sequence as i64
            ],
            |row| {
                let payload: String = row.get(6)?;
                Ok(SessionEvent {
                    sequence: row.get::<_, i64>(0)? as u64,
                    event_id: row.get(1)?,
                    session_id: row.get(2)?,
                    user_id: row.get(3)?,
                    run_id: row.get(4)?,
                    kind: row.get(5)?,
                    payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
                    created_at: row.get(7)?,
                })
            },
        )
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

pub fn create_run(
    db: &Db,
    run_id: &str,
    session_id: &str,
    user_id: &str,
    request: &Value,
    state: &str,
) -> bool {
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT OR IGNORE INTO chat_runs
             (id, session_id, user_id, state, request_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![run_id, session_id, user_id, state, request.to_string(), now],
        )
        .unwrap_or(0)
        > 0
}

/// Atomically decides whether a newly submitted turn owns the session or must
/// wait behind existing durable work.
pub fn admit_run(
    db: &Db,
    run_id: &str,
    session_id: &str,
    user_id: &str,
    request: &Value,
) -> RunAdmission {
    admit_run_with_predecessor(db, run_id, session_id, user_id, request).0
}

/// Admits a durable run and, when it must wait, returns the exact run that owned
/// the session during the same SQLite transaction. Interrupting clients use the
/// captured ID so a concurrent handoff can never abort the replacement run.
pub fn admit_run_with_predecessor(
    db: &Db,
    run_id: &str,
    session_id: &str,
    user_id: &str,
    request: &Value,
) -> (RunAdmission, Option<String>) {
    match try_admit_run(db, run_id, session_id, user_id, request) {
        Ok(admission) => admission,
        Err(error) => {
            tracing::error!(run_id, %error, "chat input could not be saved");
            (RunAdmission::StorageUnavailable, None)
        }
    }
}

fn try_admit_run(
    db: &Db,
    run_id: &str,
    session_id: &str,
    user_id: &str,
    request: &Value,
) -> rusqlite::Result<(RunAdmission, Option<String>)> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = tx
        .query_row(
            "SELECT state, session_id, user_id, request_json FROM chat_runs WHERE id=?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    if let Some((state, owner_session, owner_user, stored)) = &existing {
        if owner_session != session_id
            || owner_user != user_id
            || !matches_saved_request(state, stored, request, run_id, session_id)
        {
            return Ok((RunAdmission::Conflict, None));
        }
    } else if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM chat_inputs WHERE id=?1)",
        [run_id],
        |row| row.get::<_, bool>(0),
    )? {
        // A guidance receipt is not permission to run the input again. Only
        // queue_unsent_input can turn a definitely-unsent receipt into a run.
        return Ok((RunAdmission::Conflict, None));
    }
    let predecessor_id = tx
        .query_row(
            "SELECT id FROM chat_runs
             WHERE session_id=?1 AND id<>?2
               AND state IN ('preparing', 'running', 'recovering', 'waiting_for_input')
             ORDER BY updated_at ASC, created_at ASC
             LIMIT 1",
            params![session_id, run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let other_inflight = predecessor_id.is_some();

    let admission = match existing.as_ref().map(|(state, _, _, _)| state.as_str()) {
        // Only the dispatcher promotes a claimed queued run to `recovering`.
        Some("recovering") if !other_inflight => {
            let now = chrono::Utc::now().to_rfc3339();
            let changed = tx.execute(
                "UPDATE chat_runs SET state='preparing', updated_at=?2
                     WHERE id=?1 AND state='recovering'",
                params![run_id, now],
            )?;
            if changed == 1 {
                RunAdmission::Start
            } else {
                RunAdmission::Duplicate
            }
        }
        Some(_) => RunAdmission::Duplicate,
        None => {
            let state = if other_inflight {
                "queued"
            } else {
                "preparing"
            };
            let now = chrono::Utc::now().to_rfc3339();
            let inserted = tx.execute(
                "INSERT INTO chat_runs
                     (id, session_id, user_id, state, request_json, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![run_id, session_id, user_id, state, request.to_string(), now],
            )?;
            if inserted != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            } else if state == "queued" {
                RunAdmission::Queued
            } else {
                RunAdmission::Start
            }
        }
    };

    tx.commit()?;
    Ok((admission, predecessor_id))
}

#[cfg(test)]
mod admission_failure_tests {
    use super::*;

    fn database() -> Db {
        let db = Db::open(":memory:");
        db.conn().execute("INSERT INTO sessions(id,project_path,created_at,updated_at,user_id) VALUES ('session','/repo','now','now','user')",[]).unwrap();
        db
    }

    #[test]
    fn storage_failure_is_never_an_accepted_duplicate() {
        let db = database();
        db.conn()
            .execute_batch(
                "CREATE TRIGGER reject_chat_input BEFORE INSERT ON chat_runs
            BEGIN SELECT RAISE(FAIL, 'storage unavailable'); END;",
            )
            .unwrap();
        assert_eq!(
            admit_run(
                &db,
                "new",
                "session",
                "user",
                &serde_json::json!({"message":"keep me"})
            ),
            RunAdmission::StorageUnavailable
        );
        assert_eq!(
            db.conn()
                .query_row("SELECT count(*) FROM chat_runs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn duplicate_receipt_requires_the_same_owner_and_payload() {
        let db = database();
        let request = serde_json::json!({"message":"original"});
        assert!(create_run(
            &db, "run", "session", "user", &request, "running"
        ));
        assert_eq!(
            admit_run(&db, "run", "session", "user", &request),
            RunAdmission::Duplicate
        );
        assert_eq!(
            admit_run(&db, "run", "session", "other", &request),
            RunAdmission::Conflict
        );
        assert_eq!(
            admit_run(&db, "run", "other", "user", &request),
            RunAdmission::Conflict
        );
        assert_eq!(
            admit_run(
                &db,
                "run",
                "session",
                "user",
                &serde_json::json!({"message":"changed"})
            ),
            RunAdmission::Conflict
        );
    }

    #[test]
    fn exact_dispatcher_recovery_survives_strict_receipt_checks() {
        let db = database();
        let original =
            serde_json::json!({"message":"build it","workingDirectory":"/repo","model":"test"});
        assert!(create_run(
            &db,
            "run",
            "session",
            "user",
            &original,
            "recovering"
        ));
        let mut recovery = original.clone();
        recovery["requestId"] = serde_json::json!("run");
        recovery["sessionId"] = serde_json::json!("session");
        recovery["message"] = serde_json::json!(RECOVERY_MESSAGE);
        recovery["hiddenContinuation"] = serde_json::json!(true);
        let mut wrong = recovery.clone();
        wrong["workingDirectory"] = serde_json::json!("/another-repo");
        assert_eq!(
            admit_run(&db, "run", "session", "user", &wrong),
            RunAdmission::Conflict
        );
        assert_eq!(
            admit_run(&db, "run", "session", "other", &recovery),
            RunAdmission::Conflict
        );
        assert_eq!(
            admit_run(&db, "run", "session", "user", &recovery),
            RunAdmission::Start
        );
        // The original saved user intent remains intact after recovery.
        assert_eq!(
            admit_run(&db, "run", "session", "user", &original),
            RunAdmission::Duplicate
        );
    }

    #[test]
    fn guidance_ids_cannot_be_replayed_as_ordinary_runs() {
        for delivery in ["waiting", "sent", "uncertain", "failed"] {
            let db = database();
            let request =
                serde_json::json!({"requestId":"input","sessionId":"session","message":"guidance"});
            assert!(matches!(
                super::super::chat_inputs::admit(&db, "input", "session", "user", &request)
                    .unwrap(),
                super::super::chat_inputs::InputAdmission::New
            ));
            super::super::chat_inputs::complete(&db, "input", delivery, None).unwrap();
            assert_eq!(
                admit_run(&db, "input", "session", "user", &request),
                RunAdmission::Conflict
            );
            if delivery != "waiting" {
                assert!(queue_unsent_input(&db, "input", "session", "user").is_err());
            }
        }
    }

    #[test]
    fn definitely_unsent_receipt_and_queue_are_saved_together_once() {
        let db = database();
        let request = serde_json::json!({"requestId":"input","sessionId":"session","message":"guidance",
            "workingDirectory":"/repo","interruptActive":true,"deliveryMode":"steer"});
        super::super::chat_inputs::admit(&db, "input", "session", "user", &request).unwrap();
        assert!(queue_unsent_input(&db, "input", "session", "other").is_err());
        let run = queue_unsent_input(&db, "input", "session", "user")
            .unwrap()
            .unwrap();
        assert_eq!(run.state, "queued");
        assert_eq!(run.request["message"], "guidance");
        assert!(run.request.get("interruptActive").is_none());
        assert!(run.request.get("deliveryMode").is_none());
        assert_eq!(
            super::super::chat_inputs::list(&db, "session", "user").unwrap()[0]["delivery"],
            "queued"
        );
        assert!(queue_unsent_input(&db, "input", "session", "user").is_err());
        assert_eq!(
            admit_run(&db, "input", "session", "user", &run.request),
            RunAdmission::Start
        );
        assert_eq!(
            admit_run(&db, "input", "session", "user", &run.request),
            RunAdmission::Duplicate
        );
    }

    #[test]
    fn unsent_guidance_waits_behind_current_and_previously_queued_work() {
        let db = database();
        let request = serde_json::json!({"message":"current"});
        assert_eq!(
            admit_run(&db, "current", "session", "user", &request),
            RunAdmission::Start
        );
        assert_eq!(
            admit_run(&db, "previous", "session", "user", &request),
            RunAdmission::Queued
        );
        super::super::chat_inputs::admit(&db, "input", "session", "user", &request).unwrap();
        assert!(
            queue_unsent_input(&db, "input", "session", "user")
                .unwrap()
                .is_none()
        );
        let first = finish_run_and_claim_next(&db, "current", "session", "completed", None)
            .unwrap()
            .unwrap();
        assert_eq!(first.id, "previous");
        assert!(claim_next_queued_run(&db, "session").is_none());
        assert!(
            finish_run_and_claim_next(&db, "current", "session", "completed", None)
                .unwrap()
                .is_none()
        );
        let second = finish_run_and_claim_next(&db, "previous", "session", "completed", None)
            .unwrap()
            .unwrap();
        assert_eq!(second.id, "input");
    }

    #[test]
    fn restart_claims_queued_only_and_not_yet_started_work_without_changing_the_prompt() {
        for initial in ["queued", "preparing", "recovering"] {
            let db = database();
            let request = serde_json::json!({"message":"must run this exact original instruction"});
            assert!(create_run(
                &db, "first", "session", "user", &request, initial
            ));
            assert!(create_run(
                &db, "second", "session", "user", &request, "queued"
            ));
            let ready = reconcile_interrupted_runs(&db);
            assert_eq!(ready.len(), 1);
            assert_eq!(ready[0].id, "first");
            assert_eq!(ready[0].state, "queued");
            assert_eq!(ready[0].request, request);
            assert_eq!(
                admit_run(&db, "first", "session", "user", &ready[0].request),
                RunAdmission::Start
            );
            assert!(claim_next_queued_run(&db, "session").is_none());
        }
    }

    #[test]
    fn queue_claim_failure_rolls_back_completion_and_unsent_receipt() {
        let db = database();
        let request = serde_json::json!({"message":"keep me"});
        assert_eq!(
            admit_run(&db, "current", "session", "user", &request),
            RunAdmission::Start
        );
        assert_eq!(
            admit_run(&db, "next", "session", "user", &request),
            RunAdmission::Queued
        );
        db.conn()
            .execute_batch(
                "CREATE TRIGGER reject_claim BEFORE UPDATE ON chat_runs
            WHEN NEW.state='recovering' BEGIN SELECT RAISE(FAIL,'no space'); END;",
            )
            .unwrap();
        assert!(finish_run_and_claim_next(&db, "current", "session", "completed", None).is_err());
        assert_eq!(
            db.conn()
                .query_row(
                    "SELECT state FROM chat_runs WHERE id='current'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "preparing"
        );
        assert_eq!(
            admit_run(&db, "third", "session", "user", &request),
            RunAdmission::Queued
        );
        db.conn()
            .execute_batch("DROP TRIGGER reject_claim")
            .unwrap();
        assert_eq!(
            finish_run_and_claim_next(&db, "current", "session", "completed", None)
                .unwrap()
                .unwrap()
                .id,
            "next"
        );

        let db = database();
        super::super::chat_inputs::admit(&db, "input", "session", "user", &request).unwrap();
        db.conn()
            .execute_batch(
                "CREATE TRIGGER reject_receipt BEFORE UPDATE ON chat_inputs
            BEGIN SELECT RAISE(FAIL,'no space'); END;",
            )
            .unwrap();
        assert!(queue_unsent_input(&db, "input", "session", "user").is_err());
        assert_eq!(
            db.conn()
                .query_row("SELECT count(*) FROM chat_runs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            super::super::chat_inputs::list(&db, "session", "user").unwrap()[0]["delivery"],
            "waiting"
        );
    }

    #[test]
    fn lost_dispatch_response_never_regresses_an_admitted_or_finished_run() {
        for state in [
            "queued",
            "recovering",
            "preparing",
            "running",
            "completed",
            "failed",
            "cancelled",
        ] {
            let db = database();
            let request = serde_json::json!({"message":"original intent"});
            assert!(create_run(&db, "run", "session", "user", &request, state));
            record_dispatch_error(&db, "run", "session", "other", "wrong owner").unwrap();
            record_dispatch_error(&db, "run", "session", "user", "HTTP response lost").unwrap();
            let (saved_state, saved_request, error, started): (
                String,
                String,
                Option<String>,
                Option<String>,
            ) = db
                .conn()
                .query_row(
                    "SELECT state,request_json,error,started_at FROM chat_runs WHERE id='run'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();
            assert_eq!(saved_state, state);
            assert_eq!(
                serde_json::from_str::<Value>(&saved_request).unwrap(),
                request
            );
            assert_eq!(
                error.as_deref(),
                (state == "recovering").then_some("HTTP response lost")
            );
            assert!(started.is_none());
        }
    }
}

/// Completes the current lease and claims the oldest queued successor in one
/// transaction. A concurrent admission therefore either lands in this claim or
/// observes the claimed successor before deciding whether it can start.
pub fn finish_run_and_claim_next(
    db: &Db,
    run_id: &str,
    session_id: &str,
    final_state: &str,
    error: Option<&str>,
) -> rusqlite::Result<Option<DurableChatRun>> {
    finish_owned_run_and_claim_next(db, run_id, session_id, final_state, error, None)
}

pub fn finish_owned_run_and_claim_next(
    db: &Db,
    run_id: &str,
    session_id: &str,
    final_state: &str,
    error: Option<&str>,
    owner: Option<&str>,
) -> rusqlite::Result<Option<DurableChatRun>> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = chrono::Utc::now().to_rfc3339();
    let changed = tx.execute(
        "UPDATE chat_runs SET state=CASE WHEN ?2='completed' AND EXISTS(
            SELECT 1 FROM input_requests WHERE run_id=?1 AND status IN ('pending','answered'))
            THEN 'waiting_for_input' ELSE ?2 END,
            error=?3, updated_at=?4, finished_at=CASE WHEN ?2='waiting_for_input' OR
            (?2='completed' AND EXISTS(SELECT 1 FROM input_requests WHERE run_id=?1 AND status IN ('pending','answered')))
            THEN NULL ELSE ?4 END,lease_owner=NULL,lease_expires_at=NULL
         WHERE id=?1 AND session_id=?5
           AND state IN ('preparing', 'running', 'recovering') AND (?6 IS NULL OR lease_owner=?6)",
        params![run_id, final_state, error, now, session_id, owner],
    )?;
    let claimed = if changed == 1 {
        claim_queued_in_transaction(&tx, session_id)?
    } else {
        None
    };
    if changed == 1 {
        tx.execute("UPDATE sessions SET continuity_status=CASE WHEN ?2 IS NOT NULL THEN 'queued'
            WHEN EXISTS(SELECT 1 FROM chat_runs WHERE id=?3 AND state='waiting_for_input') THEN 'waiting_for_input'
            WHEN ?4='failed' THEN 'error' ELSE 'idle' END,
            active_request_id=?2,last_error=?5,updated_at=?6 WHERE id=?1",
            params![session_id,claimed.as_ref().map(|r|r.id.as_str()),run_id,final_state,error,now])?;
    }
    tx.commit()?;
    Ok(claimed)
}

/// Called only after the provider has definitely rejected (or never received)
/// live guidance. There is no HTTP flag that can grant this permission.
pub fn queue_unsent_input(
    db: &Db,
    id: &str,
    session_id: &str,
    user_id: &str,
) -> rusqlite::Result<Option<DurableChatRun>> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let raw: String = tx.query_row(
        "SELECT request_json FROM chat_inputs
         WHERE id=?1 AND session_id=?2 AND user_id=?3 AND delivery='waiting'",
        params![id, session_id, user_id],
        |row| row.get(0),
    )?;
    let mut request: Value =
        serde_json::from_str(&raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let body = request
        .as_object_mut()
        .ok_or(rusqlite::Error::InvalidQuery)?;
    body.remove("interruptActive");
    body.remove("deliveryMode");
    body.insert("requestId".into(), serde_json::json!(id));
    body.insert("sessionId".into(), serde_json::json!(session_id));
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute(
        "INSERT INTO chat_runs(id,session_id,user_id,state,request_json,created_at,updated_at)
         VALUES (?1,?2,?3,'queued',?4,?5,?5)",
        params![id, session_id, user_id, request.to_string(), now],
    )?;
    tx.execute(
        "UPDATE chat_inputs SET delivery='queued',
         note='Saved for the next turn; current work was not stopped.',updated_at=?2 WHERE id=?1",
        params![id, now],
    )?;
    let claimed = claim_queued_in_transaction(&tx, session_id)?;
    tx.commit()?;
    Ok(claimed)
}

pub fn update_queued_run_message(
    db: &Db,
    run_id: &str,
    user_id: &str,
    message: &str,
) -> QueuedRunUpdate {
    let mut conn = db.conn();
    let Ok(tx) = conn.transaction_with_behavior(TransactionBehavior::Immediate) else {
        return QueuedRunUpdate::NotQueued;
    };
    let existing = tx
        .query_row(
            "SELECT session_id, state, request_json FROM chat_runs WHERE id=?1 AND user_id=?2",
            params![run_id, user_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .unwrap_or(None);
    let Some((session_id, state, request_json)) = existing else {
        return QueuedRunUpdate::NotFound;
    };
    if state != "queued" {
        return QueuedRunUpdate::NotQueued;
    }
    let Ok(mut request) = serde_json::from_str::<Value>(&request_json) else {
        return QueuedRunUpdate::NotQueued;
    };
    let Some(request) = request.as_object_mut() else {
        return QueuedRunUpdate::NotQueued;
    };
    request.insert("message".to_string(), Value::String(message.to_string()));
    let now = chrono::Utc::now().to_rfc3339();
    let changed = tx
        .execute(
            "UPDATE chat_runs SET request_json=?3, updated_at=?4
             WHERE id=?1 AND user_id=?2 AND state='queued'",
            params![
                run_id,
                user_id,
                Value::Object(request.clone()).to_string(),
                now
            ],
        )
        .unwrap_or(0);
    if changed != 1 || tx.commit().is_err() {
        QueuedRunUpdate::NotQueued
    } else {
        QueuedRunUpdate::Updated { session_id }
    }
}

pub fn list_queued_runs(db: &Db, user_id: &str, session_id: &str) -> Vec<QueuedChatRun> {
    let conn = db.conn();
    let mut statement = match conn.prepare(
        "SELECT id, session_id, request_json, created_at
         FROM chat_runs
         WHERE user_id=?1 AND session_id=?2 AND state='queued'
         ORDER BY created_at ASC",
    ) {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };
    statement
        .query_map(params![user_id, session_id], |row| {
            let request_json: String = row.get(2)?;
            let request = serde_json::from_str::<Value>(&request_json).unwrap_or(Value::Null);
            let created_at = row
                .get::<_, String>(3)
                .ok()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                .map(|value| value.timestamp_millis())
                .unwrap_or_default();
            Ok(QueuedChatRun {
                id: row.get(0)?,
                session_id: row.get(1)?,
                message: request
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                hidden_continuation: request
                    .get("hiddenContinuation")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                created_at,
            })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

pub fn update_run_state(db: &Db, run_id: &str, state: &str, error: Option<&str>) {
    let now = chrono::Utc::now().to_rfc3339();
    let started = state == "running";
    let finished = matches!(state, "completed" | "failed" | "cancelled");
    let _ = db.conn().execute(
        "UPDATE chat_runs SET state=?2, error=?3, updated_at=?4,
             started_at=CASE WHEN ?5=1 THEN COALESCE(started_at, ?4) ELSE started_at END,
             finished_at=CASE WHEN ?6=1 THEN ?4 ELSE finished_at END
         WHERE id=?1",
        params![
            run_id,
            state,
            error,
            now,
            if started { 1 } else { 0 },
            if finished { 1 } else { 0 }
        ],
    );
}

/// Dispatch can lose its HTTP response after admission has succeeded. Only
/// annotate the still-unclaimed recovery lease; never regress a live/final run.
pub fn record_dispatch_error(
    db: &Db,
    run_id: &str,
    session_id: &str,
    user_id: &str,
    error: &str,
) -> rusqlite::Result<()> {
    db.conn().execute(
        "UPDATE chat_runs SET error=?4,updated_at=?5,lease_expires_at=NULL,
            retry_at=?6,consecutive_failures=consecutive_failures+1
         WHERE id=?1 AND session_id=?2 AND user_id=?3 AND state='recovering'",
        params![
            run_id,
            session_id,
            user_id,
            error,
            chrono::Utc::now().to_rfc3339(),
            (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339()
        ],
    )?;
    Ok(())
}

pub fn prepare_existing_run(db: &Db, run_id: &str, session_id: &str, user_id: &str) -> bool {
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "UPDATE chat_runs SET state='preparing', updated_at=?4
             WHERE id=?1 AND session_id=?2 AND user_id=?3
               AND state IN ('queued', 'recovering')",
            params![run_id, session_id, user_id, now],
        )
        .unwrap_or(0)
        == 1
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<DurableChatRun> {
    let request: String = row.get(4)?;
    Ok(DurableChatRun {
        id: row.get(0)?,
        session_id: row.get(1)?,
        user_id: row.get(2)?,
        state: row.get(3)?,
        request: serde_json::from_str(&request).unwrap_or(Value::Null),
    })
}

/// Marks work interrupted by a process exit as recoverable and returns at most
/// one run per session. Remaining queued turns are claimed after each completion.
pub fn reconcile_interrupted_runs(db: &Db) -> Vec<DurableChatRun> {
    match try_reconcile_interrupted_runs(db) {
        Ok(runs) => runs,
        Err(error) => {
            tracing::error!(%error, "durable chat recovery could not be saved");
            Vec::new()
        }
    }
}

fn try_reconcile_interrupted_runs(db: &Db) -> rusqlite::Result<Vec<DurableChatRun>> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute("UPDATE chat_inputs SET delivery='uncertain',note='Server restarted before delivery was confirmed. Check before resending.',updated_at=?1 WHERE delivery='waiting'", [&now])?;
    tx.execute(
        "UPDATE chat_runs SET state='waiting_for_input',lease_owner=NULL,lease_expires_at=NULL
        WHERE state IN ('running','preparing','recovering') AND EXISTS(
            SELECT 1 FROM input_requests WHERE run_id=chat_runs.id AND status='pending')",
        [],
    )?;
    tx.execute(
        "UPDATE chat_runs SET state=CASE
             WHEN state IN ('preparing','recovering') AND started_at IS NULL THEN 'queued'
             ELSE 'recovering' END, lease_owner=NULL,
             lease_expires_at=NULL, updated_at=?1
         WHERE state IN ('running', 'preparing', 'recovering')",
        [&now],
    )?;
    tx.execute(
        "UPDATE sessions SET continuity_status='recovering', recovery_strategy='server_restart',
             updated_at=?1
         WHERE id IN (SELECT session_id FROM chat_runs WHERE state='recovering')",
        [&now],
    )?;

    let mut statement = tx.prepare(
        "SELECT id, session_id, user_id, state, request_json
         FROM chat_runs
         WHERE state IN ('recovering', 'queued')
           AND NOT EXISTS (
               SELECT 1 FROM input_requests
               WHERE input_requests.run_id = chat_runs.id
                 AND input_requests.status = 'answered'
           )
         ORDER BY CASE state WHEN 'recovering' THEN 0 ELSE 1 END, created_at ASC",
    )?;
    let candidates = statement
        .query_map([], row_to_run)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    let mut sessions = std::collections::HashSet::new();
    let mut ready = Vec::new();
    for run in candidates {
        if !sessions.insert(run.session_id.clone()) {
            continue;
        }
        if run.state == "queued" {
            // Claim in SQLite, but retain the returned pre-claim queued state:
            // this prompt has never run, so dispatch must keep its exact text.
            if let Some(claimed) = claim_queued_in_transaction(&tx, &run.session_id)? {
                ready.push(claimed);
            }
        } else {
            ready.push(run);
        }
    }
    tx.commit()?;
    Ok(ready)
}

fn claim_queued_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    session_id: &str,
) -> rusqlite::Result<Option<DurableChatRun>> {
    let run = tx
        .query_row(
            "SELECT id, session_id, user_id, state, request_json
             FROM chat_runs WHERE session_id=?1 AND state='queued'
               AND NOT EXISTS (SELECT 1 FROM chat_runs active
                   WHERE active.session_id=?1
                     AND active.state IN ('preparing', 'running', 'recovering', 'waiting_for_input'))
               AND NOT EXISTS (SELECT 1 FROM input_requests
                   WHERE input_requests.run_id=chat_runs.id AND input_requests.status='answered')
             ORDER BY created_at ASC LIMIT 1",
            [session_id],
            row_to_run,
        )
        .optional()?;
    let Some(run) = run else {
        return Ok(None);
    };
    let now = chrono::Utc::now().to_rfc3339();
    let changed = tx.execute(
        "UPDATE chat_runs SET state='recovering', updated_at=?2
             WHERE id=?1 AND state='queued'",
        params![&run.id, now],
    )?;
    Ok((changed == 1).then_some(run))
}

pub fn claim_next_queued_run(db: &Db, session_id: &str) -> Option<DurableChatRun> {
    let mut conn = db.conn();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .ok()?;
    let claimed = claim_queued_in_transaction(&tx, session_id).ok()?;
    tx.commit().ok()?;
    claimed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sessions::{UpsertSession, upsert_session};

    #[test]
    fn event_sequences_are_replayable_and_ordered() {
        let db = Db::open(":memory:");
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/tmp/project".into(),
                title: Some("Session".into()),
                message_count: Some(0),
                pinned: None,
                model: None,
                input_tokens: None,
                output_tokens: None,
                provider: None,
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );
        let first = append_event(
            &db,
            "session-1",
            None,
            "status",
            &serde_json::json!({"n": 1}),
        )
        .unwrap();
        let second = append_event(
            &db,
            "session-1",
            None,
            "status",
            &serde_json::json!({"n": 2}),
        )
        .unwrap();
        assert!(second.sequence > first.sequence);
        let replayed = replay(&db, "user-1", "session-1", first.sequence, second.sequence);
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].payload["n"], 2);

        let run_event = append_event(
            &db,
            "session-1",
            Some("run-1"),
            "status",
            &serde_json::json!({"n": 3}),
        )
        .unwrap();
        assert_eq!(
            first_sequence_for_run(&db, "user-1", "session-1", "run-1"),
            Some(run_event.sequence)
        );
    }

    #[test]
    fn cumulative_run_messages_are_coalesced() {
        let db = Db::open(":memory:");
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/tmp/project".into(),
                title: Some("Session".into()),
                message_count: Some(0),
                pinned: None,
                model: None,
                input_tokens: None,
                output_tokens: None,
                provider: None,
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );
        assert!(create_run(
            &db,
            "run-1",
            "session-1",
            "user-1",
            &serde_json::json!({"message": "hello"}),
            "running",
        ));

        let first = append_event(
            &db,
            "session-1",
            Some("run-1"),
            "message",
            &serde_json::json!({"text": "partial"}),
        )
        .unwrap();
        let status = append_event(
            &db,
            "session-1",
            Some("run-1"),
            "status",
            &serde_json::json!({"text": "working"}),
        )
        .unwrap();
        let final_snapshot = append_event(
            &db,
            "session-1",
            Some("run-1"),
            "message",
            &serde_json::json!({"text": "complete"}),
        )
        .unwrap();

        let replayed = replay(&db, "user-1", "session-1", 0, final_snapshot.sequence);
        assert_eq!(replayed.len(), 2);
        assert!(
            replayed
                .iter()
                .any(|event| event.sequence == status.sequence)
        );
        assert!(
            !replayed
                .iter()
                .any(|event| event.sequence == first.sequence)
        );
        assert_eq!(
            replayed
                .iter()
                .find(|event| event.kind == "message")
                .unwrap()
                .payload["text"],
            "complete"
        );
    }

    #[test]
    fn interrupted_and_queued_runs_resume_one_at_a_time() {
        let db = Db::open(":memory:");
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/tmp/project".into(),
                title: Some("Session".into()),
                message_count: Some(1),
                pinned: None,
                model: None,
                input_tokens: None,
                output_tokens: None,
                provider: None,
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );
        assert!(create_run(
            &db,
            "run-1",
            "session-1",
            "user-1",
            &serde_json::json!({"message": "first"}),
            "running",
        ));
        assert!(create_run(
            &db,
            "run-2",
            "session-1",
            "user-1",
            &serde_json::json!({"message": "second"}),
            "queued",
        ));

        let recoverable = reconcile_interrupted_runs(&db);
        assert_eq!(recoverable.len(), 1);
        assert_eq!(recoverable[0].id, "run-1");
        assert_eq!(recoverable[0].state, "recovering");

        update_run_state(&db, "run-1", "completed", None);
        let next = claim_next_queued_run(&db, "session-1").unwrap();
        assert_eq!(next.id, "run-2");
        assert!(claim_next_queued_run(&db, "session-1").is_none());
        assert!(prepare_existing_run(&db, "run-2", "session-1", "user-1"));
    }

    #[test]
    fn admission_queues_behind_preparing_work_without_in_memory_state() {
        let db = Db::open(":memory:");
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/tmp/project".into(),
                title: Some("Session".into()),
                message_count: Some(0),
                pinned: None,
                model: None,
                input_tokens: None,
                output_tokens: None,
                provider: None,
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );

        assert_eq!(
            admit_run(
                &db,
                "run-1",
                "session-1",
                "user-1",
                &serde_json::json!({"message": "first"}),
            ),
            RunAdmission::Start
        );
        assert_eq!(
            admit_run_with_predecessor(
                &db,
                "run-2",
                "session-1",
                "user-1",
                &serde_json::json!({"message": "second"}),
            ),
            (RunAdmission::Queued, Some("run-1".to_string()))
        );
        assert_eq!(
            admit_run(
                &db,
                "run-2",
                "session-1",
                "user-1",
                &serde_json::json!({"message": "second"}),
            ),
            RunAdmission::Duplicate
        );
    }

    #[test]
    fn claimed_queue_item_can_start_after_predecessor_finishes() {
        let db = Db::open(":memory:");
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/tmp/project".into(),
                title: Some("Session".into()),
                message_count: Some(0),
                pinned: None,
                model: None,
                input_tokens: None,
                output_tokens: None,
                provider: None,
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );
        assert_eq!(
            admit_run(
                &db,
                "run-1",
                "session-1",
                "user-1",
                &serde_json::json!({"message": "first"}),
            ),
            RunAdmission::Start
        );
        assert_eq!(
            admit_run(
                &db,
                "run-2",
                "session-1",
                "user-1",
                &serde_json::json!({"message": "second"}),
            ),
            RunAdmission::Queued
        );
        let claimed = finish_run_and_claim_next(&db, "run-1", "session-1", "completed", None)
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id, "run-2");
        assert_eq!(
            admit_run(&db, "run-2", "session-1", "user-1", &claimed.request,),
            RunAdmission::Start
        );
        assert_eq!(
            admit_run(
                &db,
                "run-3",
                "session-1",
                "user-1",
                &serde_json::json!({"message": "third"}),
            ),
            RunAdmission::Queued
        );
    }

    #[test]
    fn admission_starts_cleanly_after_an_empty_atomic_handoff() {
        let db = Db::open(":memory:");
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/tmp/project".into(),
                title: Some("Session".into()),
                message_count: Some(0),
                pinned: None,
                model: None,
                input_tokens: None,
                output_tokens: None,
                provider: None,
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );
        assert_eq!(
            admit_run(
                &db,
                "run-1",
                "session-1",
                "user-1",
                &serde_json::json!({"message": "first"}),
            ),
            RunAdmission::Start
        );
        assert!(
            finish_run_and_claim_next(&db, "run-1", "session-1", "completed", None,)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            admit_run(
                &db,
                "run-2",
                "session-1",
                "user-1",
                &serde_json::json!({"message": "second"}),
            ),
            RunAdmission::Start
        );
    }

    #[test]
    fn queued_message_edits_are_owned_ordered_and_stop_at_claim() {
        let db = Db::open(":memory:");
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/tmp/project".into(),
                title: Some("Session".into()),
                message_count: Some(0),
                pinned: None,
                model: None,
                input_tokens: None,
                output_tokens: None,
                provider: None,
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );
        assert_eq!(
            admit_run(
                &db,
                "run-1",
                "session-1",
                "user-1",
                &serde_json::json!({"message": "first"}),
            ),
            RunAdmission::Start
        );
        assert_eq!(
            admit_run(
                &db,
                "run-2",
                "session-1",
                "user-1",
                &serde_json::json!({"message": "old second"}),
            ),
            RunAdmission::Queued
        );
        assert_eq!(
            admit_run(
                &db,
                "run-3",
                "session-1",
                "user-1",
                &serde_json::json!({"message": "third"}),
            ),
            RunAdmission::Queued
        );

        assert_eq!(
            update_queued_run_message(&db, "run-2", "other-user", "stolen"),
            QueuedRunUpdate::NotFound
        );
        assert_eq!(
            update_queued_run_message(&db, "run-2", "user-1", "edited second"),
            QueuedRunUpdate::Updated {
                session_id: "session-1".into()
            }
        );

        let claimed = finish_run_and_claim_next(&db, "run-1", "session-1", "completed", None)
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id, "run-2");
        assert_eq!(claimed.request["message"], "edited second");
        assert_eq!(
            update_queued_run_message(&db, "run-2", "user-1", "too late"),
            QueuedRunUpdate::NotQueued
        );
        assert!(claim_next_queued_run(&db, "session-1").is_none());
        let next = finish_run_and_claim_next(&db, "run-2", "session-1", "completed", None)
            .unwrap()
            .unwrap();
        assert_eq!(next.id, "run-3");
    }

    #[test]
    fn queued_run_projection_preserves_hidden_continuation_metadata() {
        let db = Db::open(":memory:");
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/tmp/project".into(),
                title: Some("Session".into()),
                message_count: Some(0),
                pinned: None,
                model: None,
                input_tokens: None,
                output_tokens: None,
                provider: None,
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );
        assert!(create_run(
            &db,
            "run-1",
            "session-1",
            "user-1",
            &serde_json::json!({
                "message": "Continue from where you left off.",
                "hiddenContinuation": true,
            }),
            "queued",
        ));

        let runs = list_queued_runs(&db, "user-1", "session-1");
        assert_eq!(runs.len(), 1);
        assert!(runs[0].hidden_continuation);
    }

    #[test]
    fn maintenance_only_removes_transient_history_from_terminal_runs() {
        let db = Db::open(":memory:");
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/tmp/project".into(),
                title: Some("Session".into()),
                message_count: Some(0),
                pinned: None,
                model: None,
                input_tokens: None,
                output_tokens: None,
                provider: None,
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );
        assert!(create_run(
            &db,
            "old-run",
            "session-1",
            "user-1",
            &serde_json::json!({"message":"old"}),
            "completed",
        ));
        assert!(create_run(
            &db,
            "active-run",
            "session-1",
            "user-1",
            &serde_json::json!({"message":"active"}),
            "running",
        ));
        let old = "2026-01-01T00:00:00+00:00";
        db.conn()
            .execute(
                "UPDATE chat_runs SET finished_at=?1, updated_at=?1 WHERE id='old-run'",
                [old],
            )
            .unwrap();
        let old_event = append_event(
            &db,
            "session-1",
            Some("old-run"),
            "done",
            &serde_json::json!({"type":"done"}),
        )
        .unwrap();
        db.conn()
            .execute(
                "UPDATE chat_runs SET finished_at=?1, updated_at=?1 WHERE id='old-run'",
                [old],
            )
            .unwrap();
        let active_event = append_event(
            &db,
            "session-1",
            Some("active-run"),
            "status",
            &serde_json::json!({"type":"status"}),
        )
        .unwrap();

        let result = maintain_realtime_journal(&db, 100);
        assert_eq!(result.expired_events, 1);
        assert_eq!(
            max_sequence(&db, "user-1", "session-1"),
            active_event.sequence
        );
        assert!(
            replay(
                &db,
                "user-1",
                "session-1",
                old_event.sequence.saturating_sub(1),
                active_event.sequence,
            )
            .iter()
            .all(|event| event.run_id.as_deref() != Some("old-run"))
        );
        assert_eq!(
            db.conn()
                .query_row(
                    "SELECT COUNT(*) FROM chat_runs WHERE id='active-run'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
    }
}
