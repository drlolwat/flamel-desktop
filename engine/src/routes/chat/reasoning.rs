//! Auto reasoning handoffs belong to the durable server queue, not a browser
//! heuristic or an instruction asking the user to change a dropdown.
use crate::AppState;
use crate::db::{Db, plans};
use crate::model_routing::{automatic_reasoning, effort_rank, resolve_model_route};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

fn should_handoff(request: &Value, current: Option<&str>, desired: &str) -> bool {
    automatic_reasoning(request.get("reasoningEffort").and_then(Value::as_str))
        && effort_rank(desired) > effort_rank(current.unwrap_or("medium"))
}

fn continuation_request(original: &Value, id: &str, sid: &str, plan_id: &str) -> Value {
    let mut body = original.clone();
    if let Some(object) = body.as_object_mut() {
        for key in ["interruptActive", "inputRecovery", "images", "deliveryMode"] {
            object.remove(key);
        }
        object.insert("requestId".into(), json!(id));
        object.insert("sessionId".into(), json!(sid));
        object.insert("activePlanId".into(), json!(plan_id));
        object.insert("reasoningEffort".into(), json!("auto"));
        object.insert("hiddenContinuation".into(), json!(true));
        object.insert("message".into(),json!("Continue the approved plan from the saved work. Flamel has automatically adjusted reasoning for the active item. Do not ask the user to change reasoning or resend their request; verify the current item and continue without repeating completed work."));
    }
    body
}

fn enqueue(
    db: &Db,
    run_id: &str,
    sid: &str,
    uid: &str,
    plan_id: &str,
    request: &Value,
) -> rusqlite::Result<bool> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let eligible: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM chat_runs WHERE id=?1 AND session_id=?2 AND user_id=?3 AND state='running' AND COALESCE(json_extract(request_json,'$.continuationCancelled'),0)=0)
        AND EXISTS(SELECT 1 FROM plans WHERE id=?4 AND user_id=?3 AND execution_status='running' AND approval_status='approved' AND finished=0)
        AND NOT EXISTS(SELECT 1 FROM chat_runs WHERE session_id=?2 AND state='queued')
        AND NOT EXISTS(SELECT 1 FROM input_requests WHERE session_id=?2 AND
            (status='pending' OR (status='answered' AND
                (run_id=?1 OR id=json_extract((SELECT request_json FROM chat_runs WHERE id=?1),'$.inputRecovery.requestId')))))
        AND NOT EXISTS(SELECT 1 FROM chat_inputs WHERE session_id=?2 AND delivery='waiting')",
        params![run_id,sid,uid,plan_id],|row|row.get(0))?;
    if !eligible {
        return Ok(false);
    }
    let now = chrono::Utc::now().to_rfc3339();
    let changed = tx.execute("INSERT OR IGNORE INTO chat_runs(id,session_id,user_id,state,request_json,created_at,updated_at)
        VALUES (?1,?2,?3,'queued',?4,?5,?5)",
        params![request["requestId"].as_str(),sid,uid,request.to_string(),now])?;
    tx.commit()?;
    Ok(changed == 1)
}

pub(crate) fn queue_if_needed(
    state: &AppState,
    sid: &str,
    uid: &str,
    run_id: &str,
    current_model: Option<&str>,
    current_effort: Option<&str>,
) -> Result<Option<Value>, String> {
    let request = state.db.conn().query_row("SELECT request_json FROM chat_runs WHERE id=?1 AND session_id=?2 AND user_id=?3 AND state='running'",
        params![run_id,sid,uid],|row|row.get::<_,String>(0)).optional().map_err(|error|error.to_string())?;
    let Some(request) = request else {
        return Ok(None);
    };
    let original: Value = serde_json::from_str(&request).map_err(|error| error.to_string())?;
    if !automatic_reasoning(original.get("reasoningEffort").and_then(Value::as_str)) {
        return Ok(None);
    }
    let session = crate::db::sessions::get_session(&state.db, sid);
    let plan_id = original
        .get("activePlanId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| session.and_then(|session| session.active_plan_id));
    let Some(plan_id) = plan_id else {
        return Ok(None);
    };
    if plans::get_plan_user_id(&state.db, &plan_id).as_deref() != Some(uid) {
        return Ok(None);
    }
    let Some(plan) = plans::get_plan(&state.db, &plan_id).filter(|plan| {
        !plan.meta.finished
            && plan.meta.execution_status == "running"
            && plan.meta.approval_status == "approved"
    }) else {
        return Ok(None);
    };
    let Some(floor) = plans::active_plan_reasoning_floor(&state.db, &plan.meta.id) else {
        return Ok(None);
    };
    let decision = resolve_model_route(
        original.get("model").and_then(Value::as_str),
        None,
        "continue the approved implementation",
        None,
        Some(&floor),
        &crate::db::settings::get_user_settings(&state.db, uid),
        crate::routes::providers::provider_availability(state, Some(uid)),
    )?;
    let desired = decision.reasoning_effort.as_deref().unwrap_or("medium");
    // Only raise at a boundary. A pinned model already at its ceiling must not
    // enter a restart loop, and the next ordinary turn may rebalance downward.
    if !should_handoff(&original, current_effort, desired) {
        return Ok(None);
    }
    let id = format!("auto-reasoning-{run_id}");
    let mut next = continuation_request(&original, &id, sid, &plan_id);
    next["continuationOf"] = json!(run_id);
    if !enqueue(&state.db, run_id, sid, uid, &plan_id, &next).map_err(|error| error.to_string())? {
        return Ok(None);
    }
    Ok(Some(
        json!({"type":"reasoning_reroute","sessionId":sid,"requestId":id,
        "fromModel":current_model,"fromReasoningEffort":current_effort,
        "model":decision.model,"reasoningEffort":desired,
        "requestedModel":decision.requested_model,"requestedReasoningEffort":"auto",
        "note":format!("Auto is switching reasoning from {} to {desired} and continuing the plan.",current_effort.unwrap_or("provider default"))}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_auto_can_raise_at_a_boundary_and_it_never_loops_at_a_ceiling() {
        assert!(!should_handoff(
            &json!({"reasoningEffort":"high"}),
            Some("high"),
            "xhigh"
        ));
        assert!(should_handoff(
            &json!({"reasoningEffort":"auto"}),
            Some("high"),
            "xhigh"
        ));
        assert!(!should_handoff(
            &json!({"reasoningEffort":"auto"}),
            Some("xhigh"),
            "xhigh"
        ));
        assert!(!should_handoff(
            &json!({"reasoningEffort":"auto"}),
            Some("xhigh"),
            "medium"
        ));
    }
    #[test]
    fn handoff_is_durable_once_and_does_not_jump_a_user_message() {
        let db = Db::open(":memory:");
        db.conn().execute("INSERT INTO sessions(id,project_path,created_at,updated_at,user_id) VALUES ('session','/repo','now','now','user')",[]).unwrap();
        let plan = plans::create_plan(
            &db,
            "Plan",
            None,
            &[json!({"text":"Verify","reasoningFloor":"xhigh"})],
            Some("session"),
            Some("/repo"),
            "user",
        )
        .unwrap();
        plans::update_plan_execution(&db, &plan.meta.id, "start").unwrap();
        let original = json!({"message":"finish it","model":"gpt-6-astra","reasoningEffort":"auto","workingDirectory":"/repo","interruptActive":true,"images":[{}]});
        assert!(crate::db::realtime::create_run(
            &db, "run", "session", "user", &original, "running"
        ));
        let next = continuation_request(&original, "next", "session", &plan.meta.id);
        assert!(crate::db::realtime::create_run(
            &db,
            "old-run",
            "session",
            "user",
            &json!({}),
            "completed"
        ));
        db.conn().execute("INSERT INTO input_requests(id,session_id,run_id,user_id,request_json,status,created_at,updated_at) VALUES ('historical-answer','session','old-run','user','{}','answered','2026-08-30','2026-08-30')",[]).unwrap();
        assert_eq!(next["reasoningEffort"], "auto");
        assert_eq!(next["model"], "gpt-6-astra");
        assert!(next.get("images").is_none() && next.get("interruptActive").is_none());
        assert!(!enqueue(&db, "run", "session", "other", &plan.meta.id, &next).unwrap());
        assert!(enqueue(&db, "run", "session", "user", &plan.meta.id, &next).unwrap());
        assert!(!enqueue(&db, "run", "session", "user", &plan.meta.id, &next).unwrap());
        let queued = crate::db::realtime::finish_run_and_claim_next(
            &db,
            "run",
            "session",
            "completed",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(queued.id, "next");
        assert_eq!(queued.request, next);
        assert_eq!(
            crate::db::realtime::admit_run(&db, "next", "session", "user", &queued.request),
            crate::db::realtime::RunAdmission::Start
        );
        assert!(crate::db::realtime::create_run(
            &db,
            "user-message",
            "session",
            "user",
            &json!({"message":"different task"}),
            "queued"
        ));
        crate::db::realtime::update_run_state(&db, "next", "running", None);
        let following = continuation_request(&original, "following", "session", &plan.meta.id);
        assert!(!enqueue(&db, "next", "session", "user", &plan.meta.id, &following).unwrap());
    }
}
