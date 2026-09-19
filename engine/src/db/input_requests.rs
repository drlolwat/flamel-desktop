use crate::db::Db;
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequest {
    pub id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub user_id: String,
    pub request: Value,
    pub status: String,
    pub answer: Option<String>,
    pub delivery_kind: Option<String>,
    pub delivery_claim_owner: Option<String>,
    pub delivery_claimed_at: Option<String>,
    pub delivery_claim_expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub answered_at: Option<String>,
    pub cancelled_at: Option<String>,
    pub consumed_at: Option<String>,
}

pub struct NewInputRequest<'a> {
    pub id: &'a str,
    pub session_id: &'a str,
    pub run_id: Option<&'a str>,
    pub user_id: &'a str,
    pub request: &'a Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CreateInputRequestResult {
    Created(InputRequest),
    Existing(InputRequest),
    Conflict,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputRequestTransition {
    Updated(InputRequest),
    Idempotent(InputRequest),
    Conflict(InputRequest),
    NotFound,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeliveryClaimResult {
    Claimed(InputRequest),
    Busy(InputRequest),
    NotReady(InputRequest),
    NotFound,
}

fn row_to_input_request(row: &Row<'_>) -> rusqlite::Result<InputRequest> {
    let request_json: String = row.get(4)?;
    let request = serde_json::from_str(&request_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            request_json.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    Ok(InputRequest {
        id: row.get(0)?,
        session_id: row.get(1)?,
        run_id: row.get(2)?,
        user_id: row.get(3)?,
        request,
        status: row.get(5)?,
        answer: row.get(6)?,
        delivery_kind: row.get(7)?,
        delivery_claim_owner: row.get(8)?,
        delivery_claimed_at: row.get(9)?,
        delivery_claim_expires_at: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        answered_at: row.get(13)?,
        cancelled_at: row.get(14)?,
        consumed_at: row.get(15)?,
    })
}

const SELECT_INPUT_REQUEST: &str =
    "SELECT id, session_id, run_id, user_id, request_json, status, answer,
            delivery_kind, delivery_claim_owner, delivery_claimed_at,
            delivery_claim_expires_at, created_at, updated_at, answered_at,
            cancelled_at, consumed_at
       FROM input_requests";

fn get_with_conn(conn: &Connection, id: &str) -> Result<Option<InputRequest>, String> {
    conn.query_row(
        &format!("{SELECT_INPUT_REQUEST} WHERE id = ?1"),
        [id],
        row_to_input_request,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn get_for_user_with_conn(
    conn: &Connection,
    id: &str,
    user_id: &str,
) -> Result<Option<InputRequest>, String> {
    conn.query_row(
        &format!("{SELECT_INPUT_REQUEST} WHERE id = ?1 AND user_id = ?2"),
        params![id, user_id],
        row_to_input_request,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub fn get_input_request(db: &Db, id: &str) -> Result<Option<InputRequest>, String> {
    get_with_conn(&db.conn(), id)
}

pub fn get_input_request_for_user(
    db: &Db,
    id: &str,
    user_id: &str,
) -> Result<Option<InputRequest>, String> {
    get_for_user_with_conn(&db.conn(), id, user_id)
}

pub fn create_input_request(
    db: &Db,
    new: NewInputRequest<'_>,
) -> Result<CreateInputRequestResult, String> {
    if new.id.trim().is_empty()
        || new.session_id.trim().is_empty()
        || new.user_id.trim().is_empty()
        || !new.request.is_object()
    {
        return Err("input request requires non-empty ids and an object payload".to_string());
    }

    let mut conn = db.conn();
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;

    if let Some(existing) = get_with_conn(&transaction, new.id)? {
        let matches = existing.session_id == new.session_id
            && existing.run_id.as_deref() == new.run_id
            && existing.user_id == new.user_id
            && existing.request == *new.request;
        return Ok(if matches {
            CreateInputRequestResult::Existing(existing)
        } else {
            CreateInputRequestResult::Conflict
        });
    }

    if let Some(run_id) = new.run_id {
        let pending_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM input_requests
                    WHERE run_id = ?1 AND status = 'pending'
                 )",
                [run_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if pending_exists {
            return Ok(CreateInputRequestResult::Conflict);
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    transaction
        .execute(
            "INSERT INTO input_requests (
                id, session_id, run_id, user_id, request_json, status,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?6)",
            params![
                new.id,
                new.session_id,
                new.run_id,
                new.user_id,
                new.request.to_string(),
                now,
            ],
        )
        .map_err(|error| error.to_string())?;

    if let Some(run_id) = new.run_id {
        let changed = transaction
            .execute(
                "UPDATE chat_runs
                    SET updated_at = ?1
                  WHERE id = ?2 AND session_id = ?3 AND user_id = ?4
                    AND state IN ('preparing', 'running', 'recovering')",
                params![now, run_id, new.session_id, new.user_id],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            let state: Option<String> = transaction
                .query_row(
                    "SELECT state FROM chat_runs
                     WHERE id = ?1 AND session_id = ?2 AND user_id = ?3",
                    params![run_id, new.session_id, new.user_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if state.as_deref() != Some("waiting_for_input") {
                return Ok(CreateInputRequestResult::Conflict);
            }
        }
    }

    let created = get_with_conn(&transaction, new.id)?
        .ok_or_else(|| "input request disappeared during creation".to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(CreateInputRequestResult::Created(created))
}

pub fn list_pending_for_session(
    db: &Db,
    user_id: &str,
    session_id: &str,
) -> Result<Vec<InputRequest>, String> {
    let conn = db.conn();
    let mut statement = conn
        .prepare(&format!(
            "{SELECT_INPUT_REQUEST}
             WHERE user_id = ?1 AND session_id = ?2 AND status = 'pending'
             ORDER BY created_at, id"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![user_id, session_id], row_to_input_request)
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

pub fn list_answered_unconsumed(db: &Db, now: &str) -> Result<Vec<InputRequest>, String> {
    let conn = db.conn();
    let mut statement = conn
        .prepare(&format!(
            "{SELECT_INPUT_REQUEST}
             WHERE status = 'answered'
               AND (
                    delivery_claim_owner IS NULL
                    OR delivery_claim_expires_at IS NULL
                    OR delivery_claim_expires_at <= ?1
               )
             ORDER BY answered_at, id"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([now], row_to_input_request)
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone)]
pub struct RecoveredInputDelivery {
    pub request: InputRequest,
    pub run: crate::db::realtime::DurableChatRun,
}

/// Atomically claims one accepted answer for provider-neutral recovery and
/// rewrites the original durable run with the deterministic hidden continuation.
/// This guarantees one Flamel continuation claimant; provider-side effects retain
/// the existing durable-run idempotency contract.
pub fn claim_answered_for_recovery(
    db: &Db,
    id: &str,
    owner: &str,
    now: &str,
    expires_at: &str,
) -> Result<Option<RecoveredInputDelivery>, String> {
    if owner.trim().is_empty() || expires_at <= now {
        return Err("recovery claim requires an owner and future expiry".to_string());
    }
    let mut conn = db.conn();
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let Some(existing) = get_with_conn(&transaction, id)? else {
        return Ok(None);
    };
    let Some(run_id) = existing.run_id.as_deref() else {
        return Ok(None);
    };
    if existing.status != "answered" {
        return Ok(None);
    }
    let claim_busy = existing
        .delivery_claim_owner
        .as_deref()
        .is_some_and(|current| {
            current != owner
                && existing
                    .delivery_claim_expires_at
                    .as_deref()
                    .is_some_and(|expiry| expiry > now)
        });
    if claim_busy {
        return Ok(None);
    }

    let run_row: Option<(String, String, String, String, String)> = transaction
        .query_row(
            "SELECT id, session_id, user_id, state, request_json
             FROM chat_runs WHERE id = ?1 AND session_id = ?2 AND user_id = ?3",
            params![run_id, &existing.session_id, &existing.user_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((run_id, session_id, user_id, state, request_json)) = run_row else {
        return Ok(None);
    };
    if !matches!(state.as_str(), "waiting_for_input" | "recovering") {
        return Ok(None);
    }
    // An answer never grants a second worker ownership of the same session.
    let another_owner: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM chat_runs WHERE session_id=?1 AND id<>?2
         AND state IN ('preparing','running','recovering'))",
            params![session_id, run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if another_owner {
        return Ok(None);
    }

    let mut run_request: Value =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    let Some(body) = run_request.as_object_mut() else {
        return Err("durable run request must be an object".to_string());
    };
    let question = existing
        .request
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or("Pending user question");
    let answer = existing.answer.as_deref().unwrap_or_default();
    body.insert(
        "message".to_string(),
        Value::String(format!(
            "Resume the suspended turn from its durable checkpoint. The user answered input request {id}.\n\nQuestion: {question}\nAnswer: {answer}\n\nApply this answer once, continue the unfinished plan or task, and do not ask the same question again."
        )),
    );
    body.insert("hiddenContinuation".to_string(), Value::Bool(true));
    body.insert(
        "inputRecovery".to_string(),
        serde_json::json!({
            "requestId": id,
            "deliveryOwner": owner,
        }),
    );

    let claimed = transaction
        .execute(
            "UPDATE input_requests
                SET delivery_kind = 'recovery', delivery_claim_owner = ?1,
                    delivery_claimed_at = ?2, delivery_claim_expires_at = ?3,
                    updated_at = ?2
              WHERE id = ?4 AND status = 'answered'
                AND (
                    delivery_claim_owner IS NULL
                    OR delivery_claim_owner = ?1
                    OR delivery_claim_expires_at IS NULL
                    OR delivery_claim_expires_at <= ?2
                )",
            params![owner, now, expires_at, id],
        )
        .map_err(|error| error.to_string())?;
    if claimed != 1 {
        return Ok(None);
    }
    let moved = transaction
        .execute(
            "UPDATE chat_runs
                SET state = 'recovering', request_json = ?1, updated_at = ?2,
                    error = NULL, finished_at = NULL
              WHERE id = ?3 AND state IN ('waiting_for_input', 'recovering')",
            params![run_request.to_string(), now, &run_id],
        )
        .map_err(|error| error.to_string())?;
    if moved != 1 {
        return Ok(None);
    }

    let request = get_with_conn(&transaction, id)?
        .ok_or_else(|| "input request disappeared while claiming recovery".to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(Some(RecoveredInputDelivery {
        request,
        run: crate::db::realtime::DurableChatRun {
            id: run_id,
            session_id,
            user_id,
            state: "recovering".to_string(),
            request: run_request,
        },
    }))
}

pub fn answer_input_request(
    db: &Db,
    id: &str,
    user_id: &str,
    answer: &str,
) -> Result<InputRequestTransition, String> {
    if answer.trim().is_empty() {
        return Err("answer must not be empty".to_string());
    }
    let mut conn = db.conn();
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let Some(existing) = get_for_user_with_conn(&transaction, id, user_id)? else {
        return Ok(InputRequestTransition::NotFound);
    };
    if existing.status != "pending" {
        return Ok(
            if existing.status == "answered" && existing.answer.as_deref() == Some(answer) {
                InputRequestTransition::Idempotent(existing)
            } else {
                InputRequestTransition::Conflict(existing)
            },
        );
    }

    let now = chrono::Utc::now().to_rfc3339();
    let changed = transaction
        .execute(
            "UPDATE input_requests
                SET status = 'answered', answer = ?1, answered_at = ?2, updated_at = ?2
              WHERE id = ?3 AND user_id = ?4 AND status = 'pending'",
            params![answer, now, id, user_id],
        )
        .map_err(|error| error.to_string())?;
    let updated = get_for_user_with_conn(&transaction, id, user_id)?
        .ok_or_else(|| "input request disappeared while answering".to_string())?;
    if changed == 0 {
        return Ok(
            if updated.status == "answered" && updated.answer.as_deref() == Some(answer) {
                InputRequestTransition::Idempotent(updated)
            } else {
                InputRequestTransition::Conflict(updated)
            },
        );
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(InputRequestTransition::Updated(updated))
}

pub fn cancel_input_request(
    db: &Db,
    id: &str,
    user_id: &str,
) -> Result<InputRequestTransition, String> {
    let mut conn = db.conn();
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let Some(existing) = get_for_user_with_conn(&transaction, id, user_id)? else {
        return Ok(InputRequestTransition::NotFound);
    };
    if existing.status != "pending" {
        return Ok(if existing.status == "cancelled" {
            InputRequestTransition::Idempotent(existing)
        } else {
            InputRequestTransition::Conflict(existing)
        });
    }

    let now = chrono::Utc::now().to_rfc3339();
    let changed = transaction
        .execute(
            "UPDATE input_requests
                SET status = 'cancelled', cancelled_at = ?1, updated_at = ?1
              WHERE id = ?2 AND user_id = ?3 AND status = 'pending'",
            params![now, id, user_id],
        )
        .map_err(|error| error.to_string())?;
    let updated = get_for_user_with_conn(&transaction, id, user_id)?
        .ok_or_else(|| "input request disappeared while cancelling".to_string())?;
    if changed == 0 {
        return Ok(if updated.status == "cancelled" {
            InputRequestTransition::Idempotent(updated)
        } else {
            InputRequestTransition::Conflict(updated)
        });
    }
    if let Some(run_id) = &updated.run_id {
        transaction
            .execute(
                "UPDATE chat_runs
                    SET state = 'cancelled', updated_at = ?1, finished_at = ?1
                  WHERE id = ?2 AND user_id = ?3 AND state = 'waiting_for_input'",
                params![now, run_id, user_id],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(InputRequestTransition::Updated(updated))
}

pub fn claim_delivery_until(
    db: &Db,
    id: &str,
    user_id: &str,
    owner: &str,
    kind: &str,
    now: &str,
    expires_at: &str,
) -> Result<DeliveryClaimResult, String> {
    if owner.trim().is_empty() || !matches!(kind, "live" | "recovery") || expires_at <= now {
        return Err("delivery claim requires an owner, valid kind, and future expiry".to_string());
    }
    let mut conn = db.conn();
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let Some(existing) = get_for_user_with_conn(&transaction, id, user_id)? else {
        return Ok(DeliveryClaimResult::NotFound);
    };
    if !matches!(existing.status.as_str(), "pending" | "answered") {
        return Ok(DeliveryClaimResult::NotReady(existing));
    }
    let busy = existing
        .delivery_claim_owner
        .as_deref()
        .is_some_and(|current| {
            current != owner
                && existing
                    .delivery_claim_expires_at
                    .as_deref()
                    .is_some_and(|expiry| expiry > now)
        });
    if busy {
        return Ok(DeliveryClaimResult::Busy(existing));
    }

    transaction
        .execute(
            "UPDATE input_requests
                SET delivery_kind = ?1, delivery_claim_owner = ?2,
                    delivery_claimed_at = ?3, delivery_claim_expires_at = ?4,
                    updated_at = ?3
              WHERE id = ?5 AND user_id = ?6
                AND status IN ('pending', 'answered')",
            params![kind, owner, now, expires_at, id, user_id],
        )
        .map_err(|error| error.to_string())?;
    let claimed = get_for_user_with_conn(&transaction, id, user_id)?
        .ok_or_else(|| "input request disappeared while claiming delivery".to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(DeliveryClaimResult::Claimed(claimed))
}

pub fn mark_consumed(
    db: &Db,
    id: &str,
    user_id: &str,
    owner: &str,
) -> Result<InputRequestTransition, String> {
    let mut conn = db.conn();
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let Some(existing) = get_for_user_with_conn(&transaction, id, user_id)? else {
        return Ok(InputRequestTransition::NotFound);
    };
    if existing.status == "consumed" && existing.delivery_claim_owner.as_deref() == Some(owner) {
        return Ok(InputRequestTransition::Idempotent(existing));
    }
    if existing.status != "answered" || existing.delivery_claim_owner.as_deref() != Some(owner) {
        return Ok(InputRequestTransition::Conflict(existing));
    }

    let now = chrono::Utc::now().to_rfc3339();
    let changed = transaction
        .execute(
            "UPDATE input_requests
                SET status = 'consumed', consumed_at = ?1, updated_at = ?1
              WHERE id = ?2 AND user_id = ?3 AND status = 'answered'
                AND delivery_claim_owner = ?4",
            params![now, id, user_id, owner],
        )
        .map_err(|error| error.to_string())?;
    let updated = get_for_user_with_conn(&transaction, id, user_id)?
        .ok_or_else(|| "input request disappeared while consuming".to_string())?;
    if changed == 0 {
        return Ok(InputRequestTransition::Conflict(updated));
    }
    // Legacy live questions changed the run to waiting_for_input before the
    // worker stopped. Consuming their answer must restore that same owner.
    if existing.delivery_kind.as_deref() == Some("live") {
        transaction
            .execute(
                "UPDATE chat_runs SET state='running',updated_at=?1 WHERE id=?2
             AND state='waiting_for_input' AND NOT EXISTS(SELECT 1 FROM chat_runs other
             WHERE other.session_id=?3 AND other.id<>?2
             AND other.state IN ('preparing','running','recovering'))",
                params![now, existing.run_id, existing.session_id],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(InputRequestTransition::Updated(updated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn seeded_db() -> Db {
        let db = Db::open(":memory:");
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sessions (
                id, project_path, created_at, updated_at, user_id
             ) VALUES ('session-1', '/tmp/project', '2026-01-01T00:00:00Z',
                       '2026-01-01T00:00:00Z', 'user-1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat_runs (
                id, session_id, user_id, state, request_json, created_at, updated_at
             ) VALUES ('run-1', 'session-1', 'user-1', 'running', '{}',
                       '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        drop(conn);
        db
    }

    fn new_request<'a>(id: &'a str, request: &'a Value) -> NewInputRequest<'a> {
        NewInputRequest {
            id,
            session_id: "session-1",
            run_id: Some("run-1"),
            user_id: "user-1",
            request,
        }
    }

    #[test]
    fn creation_is_idempotent_and_keeps_the_executing_lease() {
        let db = seeded_db();
        let payload = serde_json::json!({"question": "Which path?"});

        assert!(matches!(
            create_input_request(&db, new_request("request-1", &payload)).unwrap(),
            CreateInputRequestResult::Created(_)
        ));
        assert!(matches!(
            create_input_request(&db, new_request("request-1", &payload)).unwrap(),
            CreateInputRequestResult::Existing(_)
        ));
        assert!(matches!(
            create_input_request(
                &db,
                NewInputRequest {
                    id: "request-2",
                    ..new_request("request-2", &payload)
                }
            )
            .unwrap(),
            CreateInputRequestResult::Conflict
        ));
        let state: String = db
            .conn()
            .query_row(
                "SELECT state FROM chat_runs WHERE id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "running");
        assert!(
            crate::db::realtime::reconcile_interrupted_runs(&db).is_empty(),
            "restart recovery must not resume a run that is waiting for user input"
        );
    }

    #[test]
    fn rejects_invalid_payload_and_enforces_ownership() {
        let db = seeded_db();
        assert!(create_input_request(&db, new_request("request-1", &Value::Null)).is_err());

        let payload = serde_json::json!({"question": "Which path?"});
        create_input_request(&db, new_request("request-1", &payload)).unwrap();
        assert!(matches!(
            answer_input_request(&db, "request-1", "other-user", "A").unwrap(),
            InputRequestTransition::NotFound
        ));
        assert!(
            list_pending_for_session(&db, "other-user", "session-1")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn answer_cancel_race_has_one_terminal_winner() {
        let db = Arc::new(seeded_db());
        let payload = serde_json::json!({"question": "Which path?"});
        create_input_request(&db, new_request("request-1", &payload)).unwrap();
        let barrier = Arc::new(Barrier::new(3));

        let answer_db = Arc::clone(&db);
        let answer_barrier = Arc::clone(&barrier);
        let answer = std::thread::spawn(move || {
            answer_barrier.wait();
            answer_input_request(&answer_db, "request-1", "user-1", "A").unwrap()
        });
        let cancel_db = Arc::clone(&db);
        let cancel_barrier = Arc::clone(&barrier);
        let cancel = std::thread::spawn(move || {
            cancel_barrier.wait();
            cancel_input_request(&cancel_db, "request-1", "user-1").unwrap()
        });

        barrier.wait();
        let results = [answer.join().unwrap(), cancel.join().unwrap()];
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, InputRequestTransition::Updated(_)))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, InputRequestTransition::Conflict(_)))
                .count(),
            1
        );
    }

    #[test]
    fn accepted_answer_has_one_recovery_claim_and_deterministic_continuation() {
        let db = seeded_db();
        let payload = serde_json::json!({"question": "Which path?"});
        create_input_request(&db, new_request("request-1", &payload)).unwrap();
        answer_input_request(&db, "request-1", "user-1", "Use path A").unwrap();

        crate::db::realtime::reconcile_interrupted_runs(&db);
        let first = claim_answered_for_recovery(
            &db,
            "request-1",
            "recovery-a",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:01:00Z",
        )
        .unwrap()
        .expect("the accepted answer should be recoverable");
        assert_eq!(first.request.delivery_kind.as_deref(), Some("recovery"));
        assert_eq!(first.run.state, "recovering");
        assert_eq!(
            first.run.request.get("hiddenContinuation"),
            Some(&Value::Bool(true))
        );
        assert!(
            first.run.request["message"]
                .as_str()
                .unwrap()
                .contains("Answer: Use path A")
        );
        assert!(
            claim_answered_for_recovery(
                &db,
                "request-1",
                "recovery-b",
                "2026-01-01T00:00:30Z",
                "2026-01-01T00:02:00Z",
            )
            .unwrap()
            .is_none(),
            "an unexpired claim must prevent a second continuation"
        );
        assert!(
            crate::db::realtime::reconcile_interrupted_runs(&db).is_empty(),
            "generic restart replay must not race an accepted-answer recovery"
        );
    }

    #[test]
    fn delivery_claim_is_exclusive_expiring_and_consumable() {
        let db = seeded_db();
        let payload = serde_json::json!({"question": "Which path?"});
        create_input_request(&db, new_request("request-1", &payload)).unwrap();

        assert!(matches!(
            claim_delivery_until(
                &db,
                "request-1",
                "user-1",
                "live-owner",
                "live",
                "2026-01-01T00:00:00Z",
                "2026-01-01T00:01:00Z"
            )
            .unwrap(),
            DeliveryClaimResult::Claimed(_)
        ));
        assert!(matches!(
            claim_delivery_until(
                &db,
                "request-1",
                "user-1",
                "recovery-owner",
                "recovery",
                "2026-01-01T00:00:30Z",
                "2026-01-01T00:02:00Z"
            )
            .unwrap(),
            DeliveryClaimResult::Busy(_)
        ));
        assert!(matches!(
            claim_delivery_until(
                &db,
                "request-1",
                "user-1",
                "recovery-owner",
                "recovery",
                "2026-01-01T00:01:01Z",
                "2026-01-01T00:02:00Z"
            )
            .unwrap(),
            DeliveryClaimResult::Claimed(_)
        ));

        assert!(matches!(
            answer_input_request(&db, "request-1", "user-1", "A").unwrap(),
            InputRequestTransition::Updated(_)
        ));
        assert!(matches!(
            mark_consumed(&db, "request-1", "user-1", "live-owner").unwrap(),
            InputRequestTransition::Conflict(_)
        ));
        assert!(matches!(
            mark_consumed(&db, "request-1", "user-1", "recovery-owner").unwrap(),
            InputRequestTransition::Updated(_)
        ));
    }
}
