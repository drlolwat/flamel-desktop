//! Continue approved work through the durable queue, including without a browser.
use crate::db::Db;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) const STALLED: &str = "The approved plan is still unfinished, but repeated continuation attempts made no tool or plan progress. Work is preserved; execution has stopped to avoid a loop.";

#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    None,
    Queued,
    Stalled,
}

/// Serialize Stop against automatic enqueue without changing the active run's
/// lease: its normal finalizer still owns cleanup and queued user messages.
pub(crate) fn cancel_for_run(db: &Db, run_id: &str) -> rusqlite::Result<()> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("UPDATE chat_runs SET request_json=json_set(request_json,'$.continuationCancelled',json('true')) WHERE id=?1 AND state IN ('preparing','running','recovering','waiting_for_input')", [run_id])?;
    tx.execute("UPDATE chat_runs SET state='cancelled',finished_at=?2,updated_at=?2 WHERE state='queued' AND json_extract(request_json,'$.continuationOf')=?1", params![run_id,chrono::Utc::now().to_rfc3339()])?;
    tx.execute(
        "UPDATE chat_runs SET state='cancelled',finished_at=?2,updated_at=?2,
        lease_owner=NULL,lease_expires_at=NULL,retry_at=NULL WHERE id=?1
        AND state IN ('waiting_for_input','recovering') AND lease_owner IS NULL",
        params![run_id, chrono::Utc::now().to_rfc3339()],
    )?;
    tx.execute(
        "UPDATE input_requests SET status='cancelled',cancelled_at=?2,updated_at=?2
        WHERE run_id=?1 AND status='pending'",
        params![run_id, chrono::Utc::now().to_rfc3339()],
    )?;
    tx.execute("UPDATE sessions SET continuity_status='cancelled',active_request_id=NULL WHERE active_request_id=?1 AND EXISTS(SELECT 1 FROM chat_runs WHERE id=?1 AND state='cancelled')",[run_id])?;
    tx.commit()
}

// Ignore prose, timestamps and provider-generated IDs: another apology is not
// progress, nor is replaying the same tool call with a different call ID.
fn tool_evidence(value: &Value, evidence: &mut Vec<Value>) {
    if let Some(blocks) = value.as_array() {
        for block in blocks {
            tool_evidence(block, evidence);
        }
    } else if value["type"] == "tool_use" {
        evidence.push(json!({"name":value["name"],"input":value["input"]}));
    } else if value["type"] == "tool_result" {
        evidence.push(json!({"content":value["content"],"is_error":value["is_error"]}));
    } else if let Some(content) = value.get("content") {
        tool_evidence(content, evidence);
    } else if let Some(message) = value.get("message") {
        tool_evidence(message, evidence);
    }
}

pub(crate) fn queue_if_needed(
    db: &Db,
    sid: &str,
    uid: &str,
    run_id: &str,
) -> rusqlite::Result<Outcome> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let saved: Option<(String, Option<String>)> = tx.query_row(
        "SELECT r.request_json,s.active_plan_id FROM chat_runs r JOIN sessions s ON s.id=r.session_id
         WHERE r.id=?1 AND r.session_id=?2 AND r.user_id=?3 AND s.user_id=?3 AND r.state='running'
         AND (s.session_owner_kind IS NULL OR s.session_owner_kind='interactive')
         AND COALESCE(json_extract(r.request_json,'$.continuationCancelled'),0)=0
         AND NOT EXISTS(SELECT 1 FROM chat_runs WHERE session_id=?2 AND state='queued')
         AND NOT EXISTS(SELECT 1 FROM input_requests WHERE session_id=?2 AND
             (status='pending' OR (status='answered' AND
                 (run_id=?1 OR id=json_extract(r.request_json,'$.inputRecovery.requestId')))))
         AND NOT EXISTS(SELECT 1 FROM chat_inputs WHERE session_id=?2 AND delivery='waiting')",
        params![run_id,sid,uid], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    let Some((saved, active_plan)) = saved else {
        return Ok(Outcome::None);
    };
    let Ok(mut request) = serde_json::from_str::<Value>(&saved) else {
        return Ok(Outcome::None);
    };
    let Some(plan_id) = active_plan.or_else(|| request["activePlanId"].as_str().map(str::to_owned))
    else {
        return Ok(Outcome::None);
    };
    let eligible: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM plans WHERE id=?1 AND user_id=?2 AND execution_status='running'
         AND approval_status='approved' AND finished=0 AND archived=0)
         AND (EXISTS(SELECT 1 FROM plan_items WHERE plan_id=?1 AND status IN ('todo','in_progress'))
         OR EXISTS(SELECT 1 FROM plan_browser_reviews WHERE plan_id=?1 AND evidence_json IS NULL))",
        params![plan_id,uid], |row| row.get(0),
    )?;
    if !eligible {
        return Ok(Outcome::None);
    }

    let mut hasher = Sha256::new();
    {
        let mut stmt = tx.prepare("SELECT id,status,COALESCE(notes,''),text FROM plan_items WHERE plan_id=?1 ORDER BY position,id")?;
        for row in stmt.query_map([&plan_id], |r| {
            Ok(json!([
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?
            ]))
        })? {
            hasher.update(row?.to_string().as_bytes());
        }
        // Bound transcript reads and exclude ordinary prose from evidence.
        let mut stmt = tx.prepare("SELECT message FROM (SELECT id,role,message FROM session_messages WHERE session_id=?1 ORDER BY id DESC LIMIT 64) WHERE role IN ('assistant','tool_result') AND (message LIKE '%\"type\":\"tool_%' OR message LIKE '%\"type\": \"tool_%') ORDER BY id DESC LIMIT 8")?;
        for row in stmt.query_map([sid], |r| r.get::<_, String>(0))? {
            if let Ok(value) = serde_json::from_str::<Value>(&row?) {
                let mut evidence = Vec::new();
                tool_evidence(&value, &mut evidence);
                hasher.update(json!(evidence).to_string().as_bytes());
            }
        }
    }
    let fingerprint = hex::encode(hasher.finalize());
    let unchanged = if request["planContinuation"]["fingerprint"].as_str() == Some(&fingerprint) {
        request["planContinuation"]["unchanged"]
            .as_u64()
            .unwrap_or(0)
            .saturating_add(1)
    } else {
        0
    };
    if unchanged >= 5 {
        return Ok(Outcome::Stalled);
    }

    let id = format!("plan-continuation-{}", uuid::Uuid::new_v4());
    let Some(body) = request.as_object_mut() else {
        return Ok(Outcome::None);
    };
    for key in ["interruptActive", "inputRecovery", "images", "deliveryMode"] {
        body.remove(key);
    }
    body.insert("requestId".into(), json!(id));
    body.insert("sessionId".into(), json!(sid));
    body.insert("activePlanId".into(), json!(plan_id));
    body.insert("hiddenContinuation".into(), json!(true));
    body.insert("continuationOf".into(), json!(run_id));
    body.insert(
        "planContinuation".into(),
        json!({"fingerprint":fingerprint,"unchanged":unchanged}),
    );
    body.insert(
        "message".into(),
        json!(if unchanged >= 3 {
            format!("No meaningful progress was recorded across three continuation turns. Diagnose the actual failure using saved tool results, change the approach, and execute the next feasible step. Repeated promises or identical calls are not progress. If an external dependency prevents work, record its exact evidence and request only the missing input.\n\n{}",super::prompt::PLAN_CONTINUATION_INSTRUCTION)
        } else { super::prompt::PLAN_CONTINUATION_INSTRUCTION.to_string() }),
    );
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute("INSERT INTO chat_runs(id,session_id,user_id,state,request_json,created_at,updated_at) VALUES (?1,?2,?3,'queued',?4,?5,?5)", params![id,sid,uid,request.to_string(),now])?;
    tx.commit()?;
    Ok(Outcome::Queued)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{plans, realtime};

    fn fixture() -> (Db, String) {
        let db = Db::open(":memory:");
        db.conn().execute("INSERT INTO sessions(id,project_path,created_at,updated_at,user_id) VALUES ('s','/repo','now','now','u')", []).unwrap();
        let plan = plans::create_plan(
            &db,
            "Ship",
            None,
            &[json!({"text":"Deploy and verify"})],
            Some("s"),
            Some("/repo"),
            "u",
        )
        .unwrap();
        plans::update_plan_execution(&db, &plan.meta.id, "start").unwrap();
        let request = json!({"message":"implement, test, then deploy","model":"gpt-6-astra","reasoningEffort":"high","workingDirectory":"/repo","activePlanId":plan.meta.id,"interruptActive":true,"images":[{}]});
        assert!(realtime::create_run(
            &db, "run", "s", "u", &request, "running"
        ));
        (db, plan.meta.id)
    }

    fn finish_and_start(db: &Db, run: &str) -> String {
        let next = realtime::finish_run_and_claim_next(db, run, "s", "completed", None)
            .unwrap()
            .unwrap();
        assert_eq!(
            realtime::admit_run(db, &next.id, "s", "u", &next.request),
            realtime::RunAdmission::Start
        );
        realtime::update_run_state(db, &next.id, "running", None);
        next.id
    }

    #[test]
    fn milestone_complete_tag_cannot_abandon_approved_work_and_queue_is_once_only() {
        let (db, _) = fixture();
        crate::db::sessions::persist_session_message(&db,"s","assistant", &json!({"content":[{"type":"text","text":"Tests passed. I'm continuing with deployment. <flamel:complete/>"}]}).to_string());
        assert_eq!(
            queue_if_needed(&db, "s", "other", "run").unwrap(),
            Outcome::None
        );
        assert_eq!(
            queue_if_needed(&db, "s", "u", "run").unwrap(),
            Outcome::Queued
        );
        assert_eq!(
            queue_if_needed(&db, "s", "u", "run").unwrap(),
            Outcome::None
        );
        let next = realtime::finish_run_and_claim_next(&db, "run", "s", "completed", None)
            .unwrap()
            .unwrap();
        assert_eq!(next.request["model"], "gpt-6-astra");
        assert_eq!(next.request["reasoningEffort"], "high");
        assert_eq!(next.request["workingDirectory"], "/repo");
        assert_eq!(next.request["hiddenContinuation"], true);
        assert!(next.request.get("interruptActive").is_none());
        assert!(next.request.get("images").is_none());
        assert!(
            next.request["message"]
                .as_str()
                .unwrap()
                .contains("deployment when already requested")
        );
    }

    #[test]
    fn active_plan_from_an_earlier_session_is_authoritative() {
        let (db, id) = fixture();
        db.conn()
            .execute("UPDATE plans SET session_id=NULL WHERE id=?1", [&id])
            .unwrap();
        db.conn()
            .execute("UPDATE sessions SET active_plan_id=?1 WHERE id='s'", [&id])
            .unwrap();
        db.conn()
            .execute("UPDATE chat_runs SET request_json='{}' WHERE id='run'", [])
            .unwrap();
        assert_eq!(
            queue_if_needed(&db, "s", "u", "run").unwrap(),
            Outcome::Queued
        );
    }

    #[test]
    fn execution_and_user_input_boundaries_are_respected() {
        for mutation in [
            "UPDATE plans SET execution_status='paused'",
            "UPDATE plans SET execution_status='blocked'",
            "UPDATE plans SET approval_status='pending'",
            "UPDATE plans SET finished=1",
            "UPDATE plans SET archived=1",
            "UPDATE plans SET user_id='other'",
            "UPDATE sessions SET session_owner_kind='goal'",
            "UPDATE plan_items SET status='done'",
            "UPDATE plan_items SET status='blocked'",
            "UPDATE chat_runs SET state='cancelled'",
            "UPDATE chat_runs SET state='waiting_for_input'",
            "INSERT INTO input_requests(id,session_id,user_id,request_json,status,created_at,updated_at) VALUES ('q','s','u','{}','pending','now','now')",
            "INSERT INTO chat_inputs VALUES ('steer','s','u','{}','waiting',NULL,'now','now')",
        ] {
            let (db, _) = fixture();
            db.conn().execute(mutation, []).unwrap();
            assert_eq!(
                queue_if_needed(&db, "s", "u", "run").unwrap(),
                Outcome::None,
                "{mutation}"
            );
        }
        let (db, _) = fixture();
        assert!(realtime::create_run(
            &db,
            "user-message",
            "s",
            "u",
            &json!({"message":"stop"}),
            "queued"
        ));
        assert_eq!(
            queue_if_needed(&db, "s", "u", "run").unwrap(),
            Outcome::None
        );
    }

    #[test]
    fn historical_answer_does_not_block_a_later_run_but_current_delivery_does() {
        // Production failure: an August 30 answer remained `answered` while a
        // September 7 approved run ended with remaining implementation work.
        for (question_run, recovery, expected) in [
            ("old-run", false, Outcome::Queued),
            ("run", false, Outcome::None),
            ("old-run", true, Outcome::None),
        ] {
            let (db, _) = fixture();
            assert!(realtime::create_run(
                &db,
                "old-run",
                "s",
                "u",
                &json!({}),
                "completed"
            ));
            db.conn().execute("INSERT INTO input_requests(id,session_id,run_id,user_id,request_json,status,created_at,updated_at) VALUES ('old-answer','s',?1,'u','{}','answered','2026-08-30','2026-08-30')",[question_run]).unwrap();
            if recovery {
                db.conn().execute("UPDATE chat_runs SET request_json=json_set(request_json,'$.inputRecovery.requestId','old-answer') WHERE id='run'",[]).unwrap();
            }
            let outcome = queue_if_needed(&db, "s", "u", "run").unwrap();
            assert_eq!(
                outcome, expected,
                "question run={question_run}, recovery={recovery}"
            );
            if outcome == Outcome::Queued {
                // Exercise the browser-independent finish/claim/admit path.
                let next = finish_and_start(&db, "run");
                assert_ne!(next, "run");
            }
            let status: String = db
                .conn()
                .query_row(
                    "SELECT status FROM input_requests WHERE id='old-answer'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(status, "answered", "historical answers must remain intact");
        }
    }

    #[test]
    fn repeated_promises_stall_but_plan_progress_resets_the_guard() {
        let (db, _) = fixture();
        let mut run = "run".to_string();
        for _ in 0..5 {
            crate::db::sessions::persist_session_message(
                &db,
                "s",
                "assistant",
                &json!({"content":[{"type":"text","text":"I'll continue now."}]}).to_string(),
            );
            assert_eq!(
                queue_if_needed(&db, "s", "u", &run).unwrap(),
                Outcome::Queued
            );
            run = finish_and_start(&db, &run);
        }
        assert_eq!(
            queue_if_needed(&db, "s", "u", &run).unwrap(),
            Outcome::Stalled
        );
        db.conn()
            .execute(
                "UPDATE plan_items SET status='in_progress',notes='Build verified; deploying'",
                [],
            )
            .unwrap();
        assert_eq!(
            queue_if_needed(&db, "s", "u", &run).unwrap(),
            Outcome::Queued
        );
    }

    #[test]
    fn changing_tool_inputs_count_but_fresh_call_ids_do_not() {
        let evidence = |id: &str, command: &str| {
            let mut result = Vec::new();
            tool_evidence(
                &json!({"content":[{"type":"tool_use","id":id,"name":"shell","input":{"command":command}}]}),
                &mut result,
            );
            result
        };
        assert_eq!(evidence("first", "test"), evidence("second", "test"));
        assert_ne!(evidence("first", "test"), evidence("second", "deploy"));
    }

    #[test]
    fn stop_serializes_with_enqueue_and_preserves_queued_user_messages() {
        let (db, _) = fixture();
        cancel_for_run(&db, "run").unwrap();
        assert_eq!(
            queue_if_needed(&db, "s", "u", "run").unwrap(),
            Outcome::None
        );
        let (db, _) = fixture();
        assert_eq!(
            queue_if_needed(&db, "s", "u", "run").unwrap(),
            Outcome::Queued
        );
        assert!(realtime::create_run(
            &db,
            "user-message",
            "s",
            "u",
            &json!({"message":"different task"}),
            "queued"
        ));
        cancel_for_run(&db, "run").unwrap();
        let next = realtime::finish_run_and_claim_next(&db, "run", "s", "cancelled", None)
            .unwrap()
            .unwrap();
        assert_eq!(next.id, "user-message");
    }
}
