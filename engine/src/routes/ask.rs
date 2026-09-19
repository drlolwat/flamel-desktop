use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{Notify, broadcast};

use crate::{
    AppState,
    db::input_requests::{
        CreateInputRequestResult, DeliveryClaimResult, InputRequest, InputRequestTransition,
        NewInputRequest, answer_input_request, cancel_input_request, claim_answered_for_recovery,
        claim_delivery_until, create_input_request, get_input_request, get_input_request_for_user,
        list_answered_unconsumed, mark_consumed,
    },
    routes::admin::get_current_user_id,
};

#[derive(Debug, PartialEq)]
enum WaitOutcome {
    Answered(String),
    Cancelled,
    Consumed,
    Missing,
    Shutdown,
}

struct InputWaiterGuard {
    state: Arc<AppState>,
    request_id: String,
    notify: Arc<Notify>,
}

impl Drop for InputWaiterGuard {
    fn drop(&mut self) {
        let mut waiters = self.state.input_request_waiters.lock().unwrap();
        if waiters
            .get(&self.request_id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.notify))
            && Arc::strong_count(&self.notify) == 2
        {
            waiters.remove(&self.request_id);
        }
    }
}

fn error_response(status: StatusCode, code: &str, message: &str) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({"error": {"code": code, "message": message}})),
    )
}

fn validate_question(body: &Value) -> Result<(), &'static str> {
    let question = body
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|question| !question.is_empty())
        .ok_or("question required")?;
    if question.len() > 4_000 {
        return Err("question is too long");
    }

    let allow_free_text = body
        .get("allowFreeText")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if let Some(choices) = body.get("choices") {
        let choices = choices.as_array().ok_or("choices must be an array")?;
        if !(2..=3).contains(&choices.len()) {
            return Err("choices must contain two or three options");
        }
        if choices.iter().any(|choice| choice_label(choice).is_none()) {
            return Err("each choice requires a non-empty label");
        }
    } else if !allow_free_text {
        return Err("allowFreeText=false requires choices");
    }
    Ok(())
}

fn choice_label(choice: &Value) -> Option<&str> {
    choice
        .as_str()
        .or_else(|| choice.get("label").and_then(Value::as_str))
        .or_else(|| choice.get("value").and_then(Value::as_str))
        .map(str::trim)
        .filter(|label| !label.is_empty())
}

fn validate_answer(request: &Value, answer: &str) -> Result<(), &'static str> {
    let answer = answer.trim();
    if answer.is_empty() {
        return Err("answer required");
    }
    let matches_choice = request
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|choices| {
            choices
                .iter()
                .filter_map(choice_label)
                .any(|label| label == answer)
        });
    if matches_choice
        || request
            .get("allowFreeText")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    {
        Ok(())
    } else {
        Err("answer must match one of the allowed choices")
    }
}

fn register_waiter(state: &Arc<AppState>, request_id: &str) -> InputWaiterGuard {
    let notify = state
        .input_request_waiters
        .lock()
        .unwrap()
        .entry(request_id.to_string())
        .or_insert_with(|| Arc::new(Notify::new()))
        .clone();
    InputWaiterGuard {
        state: Arc::clone(state),
        request_id: request_id.to_string(),
        notify,
    }
}

fn wake_waiters(state: &AppState, request_id: &str) -> bool {
    let notify = state
        .input_request_waiters
        .lock()
        .unwrap()
        .get(request_id)
        .cloned();
    if let Some(notify) = notify {
        notify.notify_waiters();
        true
    } else {
        false
    }
}

pub(crate) const DELIVERY_LEASE_SECONDS: i64 = 30;

fn delivery_claim_window() -> (String, String) {
    let now = chrono::Utc::now();
    (
        now.to_rfc3339(),
        (now + chrono::Duration::seconds(DELIVERY_LEASE_SECONDS)).to_rfc3339(),
    )
}

fn live_answer_response(
    state: &AppState,
    request: &InputRequest,
    answer: String,
) -> (StatusCode, Json<Value>) {
    let owner = format!("live-{}", uuid::Uuid::new_v4());
    let (now, expires_at) = delivery_claim_window();
    match claim_delivery_until(
        &state.db,
        &request.id,
        &request.user_id,
        &owner,
        "live",
        &now,
        &expires_at,
    ) {
        Ok(DeliveryClaimResult::Claimed(_)) => (
            StatusCode::OK,
            Json(json!({
                "status": "answered",
                "requestId": request.id,
                "answer": answer,
                "deliveryOwner": owner,
            })),
        ),
        Ok(DeliveryClaimResult::Busy(_)) => error_response(
            StatusCode::CONFLICT,
            "answer_delivery_claimed",
            "This answer is already being delivered.",
        ),
        Ok(DeliveryClaimResult::NotReady(_)) => error_response(
            StatusCode::CONFLICT,
            "answer_not_deliverable",
            "This answer is no longer available for live delivery.",
        ),
        Ok(DeliveryClaimResult::NotFound) => error_response(
            StatusCode::NOT_FOUND,
            "question_not_found",
            "The pending question no longer exists.",
        ),
        Err(error) => {
            tracing::error!("ask: live delivery claim failed: {error}");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "delivery_claim_failed",
                "The accepted answer could not be claimed for delivery.",
            )
        }
    }
}

pub async fn recover_answered_input_request(
    state: Arc<AppState>,
    request_id: &str,
) -> Result<bool, String> {
    if state.is_shutting_down() {
        return Ok(false);
    }
    let owner = format!("recovery-{}", uuid::Uuid::new_v4());
    let (now, expires_at) = delivery_claim_window();
    let Some(delivery) =
        claim_answered_for_recovery(&state.db, request_id, &owner, &now, &expires_at)?
    else {
        return Ok(false);
    };
    crate::routes::chat::submit_durable_chat_run(state, delivery.run).await;
    Ok(true)
}

pub async fn recover_answered_input_requests(state: Arc<AppState>) {
    let now = chrono::Utc::now().to_rfc3339();
    let requests = match list_answered_unconsumed(&state.db, &now) {
        Ok(requests) => requests,
        Err(error) => {
            tracing::error!("input recovery: failed to list accepted answers: {error}");
            return;
        }
    };
    for request in requests {
        if let Err(error) = recover_answered_input_request(Arc::clone(&state), &request.id).await {
            tracing::error!(
                request_id = request.id,
                "input recovery: failed to claim accepted answer: {error}"
            );
        }
    }
}

async fn wait_for_input_request(
    db: Arc<crate::db::Db>,
    request_id: &str,
    user_id: &str,
    notify: &Notify,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<WaitOutcome, String> {
    loop {
        // Register the notification future before checking storage so an answer
        // committed between the read and await cannot be lost.
        let notified = notify.notified();
        tokio::pin!(notified);

        let Some(request) = get_input_request_for_user(&db, request_id, user_id)? else {
            return Ok(WaitOutcome::Missing);
        };
        match request.status.as_str() {
            "answered" => {
                return Ok(WaitOutcome::Answered(request.answer.unwrap_or_default()));
            }
            "cancelled" => return Ok(WaitOutcome::Cancelled),
            "consumed" => return Ok(WaitOutcome::Consumed),
            "pending" => {}
            _ => return Err(format!("invalid input request status: {}", request.status)),
        }
        if *shutdown.borrow() {
            return Ok(WaitOutcome::Shutdown);
        }

        tokio::select! {
            _ = &mut notified => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(WaitOutcome::Shutdown);
                }
            }
        }
    }
}

async fn publish_event(state: &AppState, session_id: &str, run_id: Option<&str>, event: &Value) {
    let live_tx = {
        let mut streams = state.live_streams.lock().await;
        if let Some(tx) = streams.get(session_id) {
            tx.clone()
        } else {
            let (tx, _) = broadcast::channel::<String>(2048);
            streams.insert(session_id.to_string(), tx.clone());
            tx
        }
    };
    if live_tx.send(format!("{}\n", event)).is_err() {
        state.realtime.publish_value(session_id, run_id, event);
    }
}

fn publish_question_lifecycle(state: &AppState, request: &InputRequest, kind: &str, status: &str) {
    let mut payload = json!({
        "requestId": request.id,
        "sessionId": request.session_id,
        "runId": request.run_id,
        "status": status,
    });
    if kind == "question.requested" {
        payload["question"] = request
            .request
            .get("question")
            .cloned()
            .unwrap_or(Value::Null);
        payload["choices"] = request
            .request
            .get("choices")
            .cloned()
            .unwrap_or(Value::Null);
        payload["allowFreeText"] = json!(
            request
                .request
                .get("allowFreeText")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        );
        payload["multiSelect"] = Value::Bool(false);
        payload["previewMarkdown"] = request
            .request
            .get("previewMarkdown")
            .cloned()
            .unwrap_or(Value::Null);
    }
    state.realtime.publish_value(
        &request.session_id,
        request.run_id.as_deref(),
        &json!({
            "type": "workflow_event",
            "kind": kind,
            "source": "backend",
            "payload": payload,
        }),
    );
}

fn terminal_response(request: &InputRequest) -> Option<(StatusCode, Json<Value>)> {
    match request.status.as_str() {
        "cancelled" => Some(error_response(
            StatusCode::GONE,
            "question_cancelled",
            "The user cancelled this question.",
        )),
        "consumed" => Some(error_response(
            StatusCode::CONFLICT,
            "answer_already_consumed",
            "This answer was already delivered.",
        )),
        _ => None,
    }
}

/// Called by flamel_mcp. The response remains open until durable state records
/// an answer/cancellation or the server begins a managed shutdown.
pub async fn handle_ask(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "Authenticated user identity required.",
        );
    };
    let Some(request_id) = body
        .get("requestId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "requestId required",
        );
    };
    let Some(session_id) = body
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "sessionId required",
        );
    };
    if let Err(message) = validate_question(&body) {
        return error_response(StatusCode::BAD_REQUEST, "invalid_question", message);
    }

    let Some(session) = crate::db::sessions::get_session(&state.db, session_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            "session_not_found",
            "Session not found.",
        );
    };
    if session.user_id.as_deref() != Some(user_id.as_str()) {
        return error_response(
            StatusCode::FORBIDDEN,
            "session_owner_mismatch",
            "The question does not belong to this user.",
        );
    }

    let run_id = body
        .get("runId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let request = match create_input_request(
        &state.db,
        NewInputRequest {
            id: request_id,
            session_id,
            run_id,
            user_id: &user_id,
            request: &body,
        },
    ) {
        Ok(CreateInputRequestResult::Created(request)) => request,
        Ok(CreateInputRequestResult::Existing(request)) => request,
        Ok(CreateInputRequestResult::Conflict) => {
            return error_response(
                StatusCode::CONFLICT,
                "input_request_conflict",
                "A different or already-active question owns this request.",
            );
        }
        Err(error) => {
            tracing::error!("ask: failed to persist input request: {error}");
            return error_response(
                StatusCode::CONFLICT,
                "input_request_invalid",
                "The question could not be linked to its session run.",
            );
        }
    };
    if request.status == "answered" {
        return live_answer_response(&state, &request, request.answer.clone().unwrap_or_default());
    }
    if let Some(response) = terminal_response(&request) {
        return response;
    }

    publish_question_lifecycle(&state, &request, "question.requested", "pending");
    let event = json!({
        "type": "ask_question",
        "kind": "question.requested",
        "requestId": request.id,
        "sessionId": request.session_id,
        "runId": request.run_id,
        "status": "pending",
        "question": request.request.get("question"),
        "choices": request.request.get("choices"),
        "allowFreeText": request.request.get("allowFreeText").and_then(Value::as_bool).unwrap_or(true),
        "multiSelect": false,
        "previewMarkdown": request.request.get("previewMarkdown"),
    });
    publish_event(&state, session_id, request.run_id.as_deref(), &event).await;

    let waiter = register_waiter(&state, &request.id);
    let mut shutdown = state.shutdown_receiver();
    let outcome = wait_for_input_request(
        Arc::clone(&state.db),
        &request.id,
        &user_id,
        &waiter.notify,
        &mut shutdown,
    )
    .await;
    match outcome {
        Ok(WaitOutcome::Answered(answer)) => live_answer_response(&state, &request, answer),
        Ok(WaitOutcome::Cancelled) => error_response(
            StatusCode::GONE,
            "question_cancelled",
            "The user cancelled this question.",
        ),
        Ok(WaitOutcome::Consumed) => error_response(
            StatusCode::CONFLICT,
            "answer_already_consumed",
            "This answer was already delivered.",
        ),
        Ok(WaitOutcome::Missing) => error_response(
            StatusCode::NOT_FOUND,
            "question_not_found",
            "The pending question no longer exists.",
        ),
        Ok(WaitOutcome::Shutdown) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_shutdown",
            "The question remains pending and will be recovered after restart.",
        ),
        Err(error) => {
            tracing::error!("ask: durable wait failed: {error}");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "input_wait_failed",
                "The question could not be monitored.",
            )
        }
    }
}

/// Called by the frontend. The first durable terminal transition wins; an
/// identical retry is accepted without delivering the answer twice.
pub async fn handle_ask_answer(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "Authenticated user identity required.",
        );
    };
    let answer = body.get("answer").and_then(Value::as_str).unwrap_or("");

    let durable = match get_input_request(&state.db, &request_id) {
        Ok(request) => request,
        Err(error) => {
            tracing::error!("ask answer: durable lookup failed: {error}");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "input_lookup_failed",
                "The question could not be loaded.",
            );
        }
    };
    let Some(request) = durable else {
        // Runner approvals still use the legacy in-memory resolver and share
        // this endpoint until their protocol is migrated.
        let sender = state.ask_resolvers.lock().unwrap().remove(&request_id);
        return if let Some(sender) = sender {
            let _ = sender.send(answer.to_string());
            (
                StatusCode::OK,
                Json(json!({"status": "answered", "requestId": request_id})),
            )
        } else {
            error_response(
                StatusCode::NOT_FOUND,
                "question_not_found",
                "No pending question exists for this request.",
            )
        };
    };
    if request.user_id != user_id {
        return error_response(
            StatusCode::FORBIDDEN,
            "question_owner_mismatch",
            "The question does not belong to this user.",
        );
    }
    if let Err(message) = validate_answer(&request.request, answer) {
        return error_response(StatusCode::BAD_REQUEST, "invalid_answer", message);
    }

    let transition = match answer_input_request(&state.db, &request_id, &user_id, answer.trim()) {
        Ok(transition) => transition,
        Err(error) => {
            tracing::error!("ask answer: transition failed: {error}");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "answer_failed",
                "The answer could not be saved.",
            );
        }
    };
    let (updated, idempotent) = match transition {
        InputRequestTransition::Updated(request) => (request, false),
        InputRequestTransition::Idempotent(request) => (request, true),
        InputRequestTransition::Conflict(_) => {
            return error_response(
                StatusCode::CONFLICT,
                "question_already_resolved",
                "The question was already resolved differently.",
            );
        }
        InputRequestTransition::NotFound => {
            return error_response(
                StatusCode::NOT_FOUND,
                "question_not_found",
                "No pending question exists for this request.",
            );
        }
    };

    let live_waiter = wake_waiters(&state, &request_id);
    if !idempotent {
        publish_question_lifecycle(&state, &updated, "question.resolved", "answered");
        publish_event(
            &state,
            &updated.session_id,
            updated.run_id.as_deref(),
            &json!({
                "type": "question_resolved",
                "kind": "question.resolved",
                "requestId": request_id,
                "sessionId": updated.session_id,
                "runId": updated.run_id,
                "status": "answered",
            }),
        )
        .await;
    }
    let recovery_state = Arc::clone(&state);
    let recovery_request_id = request_id.clone();
    tokio::spawn(async move {
        if live_waiter {
            tokio::time::sleep(std::time::Duration::from_secs(
                DELIVERY_LEASE_SECONDS as u64 + 1,
            ))
            .await;
        }
        if let Err(error) =
            recover_answered_input_request(recovery_state, &recovery_request_id).await
        {
            tracing::error!(
                request_id = recovery_request_id,
                "input recovery: delayed claim failed: {error}"
            );
        }
    });
    (
        StatusCode::OK,
        Json(json!({
            "status": "answered",
            "requestId": request_id,
            "idempotent": idempotent,
        })),
    )
}

pub async fn handle_ask_consume(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "Authenticated user identity required.",
        );
    };
    let owner = body
        .get("deliveryOwner")
        .and_then(Value::as_str)
        .unwrap_or("");
    if owner.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "delivery_owner_required",
            "deliveryOwner required",
        );
    }
    match mark_consumed(&state.db, &request_id, &user_id, owner) {
        Ok(InputRequestTransition::Updated(_)) | Ok(InputRequestTransition::Idempotent(_)) => (
            StatusCode::OK,
            Json(json!({"status": "consumed", "requestId": request_id})),
        ),
        Ok(InputRequestTransition::Conflict(_)) => error_response(
            StatusCode::CONFLICT,
            "delivery_owner_mismatch",
            "This answer is owned by a different delivery.",
        ),
        Ok(InputRequestTransition::NotFound) => error_response(
            StatusCode::NOT_FOUND,
            "question_not_found",
            "No accepted answer exists for this request.",
        ),
        Err(error) => {
            tracing::error!("ask consume: transition failed: {error}");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "consume_failed",
                "The answer delivery could not be acknowledged.",
            )
        }
    }
}

pub async fn handle_ask_cancel(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "Authenticated user identity required.",
        );
    };
    let request = match get_input_request(&state.db, &request_id) {
        Ok(Some(request)) => request,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "question_not_found",
                "No pending question exists for this request.",
            );
        }
        Err(error) => {
            tracing::error!("ask cancel: durable lookup failed: {error}");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "input_lookup_failed",
                "The question could not be loaded.",
            );
        }
    };
    if request.user_id != user_id {
        return error_response(
            StatusCode::FORBIDDEN,
            "question_owner_mismatch",
            "The question does not belong to this user.",
        );
    }

    let transition = match cancel_input_request(&state.db, &request_id, &user_id) {
        Ok(transition) => transition,
        Err(error) => {
            tracing::error!("ask cancel: transition failed: {error}");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "cancel_failed",
                "The question could not be cancelled.",
            );
        }
    };
    let (updated, idempotent) = match transition {
        InputRequestTransition::Updated(request) => (request, false),
        InputRequestTransition::Idempotent(request) => (request, true),
        InputRequestTransition::Conflict(_) => {
            return error_response(
                StatusCode::CONFLICT,
                "question_already_resolved",
                "The question was already answered.",
            );
        }
        InputRequestTransition::NotFound => {
            return error_response(
                StatusCode::NOT_FOUND,
                "question_not_found",
                "No pending question exists for this request.",
            );
        }
    };

    let _ = wake_waiters(&state, &request_id);
    if !idempotent {
        publish_question_lifecycle(&state, &updated, "question.cancelled", "cancelled");
        publish_event(
            &state,
            &updated.session_id,
            updated.run_id.as_deref(),
            &json!({
                "type": "question_cancelled",
                "kind": "question.cancelled",
                "requestId": request_id,
                "sessionId": updated.session_id,
                "runId": updated.run_id,
                "status": "cancelled",
            }),
        )
        .await;
        if let Some(run_id) = &updated.run_id
            && let Some(abort) = state.abort_controllers.lock().unwrap().get(run_id)
        {
            let _ = abort.send(true);
        }
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "cancelled",
            "requestId": request_id,
            "idempotent": idempotent,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use std::time::Duration;

    fn seeded_request() -> (Arc<Db>, Arc<Notify>) {
        let db = Arc::new(Db::open(":memory:"));
        db.conn()
            .execute(
                "INSERT INTO sessions (
                    id, project_path, created_at, updated_at, user_id
                 ) VALUES ('session-1', '/tmp/project', '2026-01-01T00:00:00Z',
                           '2026-01-01T00:00:00Z', 'user-1')",
                [],
            )
            .unwrap();
        let request = json!({
            "question": "Which path?",
            "choices": [{"label": "A"}, {"label": "B"}],
            "allowFreeText": false,
        });
        create_input_request(
            &db,
            NewInputRequest {
                id: "request-1",
                session_id: "session-1",
                run_id: None,
                user_id: "user-1",
                request: &request,
            },
        )
        .unwrap();
        (db, Arc::new(Notify::new()))
    }

    #[test]
    fn question_and_answer_validation_enforces_the_choice_contract() {
        assert!(
            validate_question(&json!({
                "question": "Choose",
                "choices": [{"label": "A"}, {"label": "B"}],
                "allowFreeText": false,
            }))
            .is_ok()
        );
        assert!(
            validate_question(&json!({
                "question": "Choose",
                "choices": [{"label": "Only"}],
            }))
            .is_err()
        );

        let request = json!({
            "choices": [{"label": "A"}, {"label": "B"}],
            "allowFreeText": false,
        });
        assert!(validate_answer(&request, "A").is_ok());
        assert!(validate_answer(&request, "C").is_err());
        assert!(validate_answer(&json!({"allowFreeText": true}), "custom").is_ok());
    }

    #[tokio::test]
    async fn durable_wait_has_no_question_timeout_and_wakes_on_answer() {
        let (db, notify) = seeded_request();
        let (_shutdown_tx, mut shutdown) = tokio::sync::watch::channel(false);
        let mut waiting = Box::pin(wait_for_input_request(
            Arc::clone(&db),
            "request-1",
            "user-1",
            &notify,
            &mut shutdown,
        ));

        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut waiting)
                .await
                .is_err(),
            "the durable wait must remain blocked without an answer"
        );
        answer_input_request(&db, "request-1", "user-1", "A").unwrap();
        notify.notify_waiters();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .unwrap()
                .unwrap(),
            WaitOutcome::Answered("A".to_string())
        );
    }

    #[tokio::test]
    async fn durable_wait_exits_for_shutdown_without_resolving_the_question() {
        let (db, notify) = seeded_request();
        let (shutdown_tx, mut shutdown) = tokio::sync::watch::channel(false);
        let waiting = wait_for_input_request(
            Arc::clone(&db),
            "request-1",
            "user-1",
            &notify,
            &mut shutdown,
        );
        shutdown_tx.send_replace(true);
        assert_eq!(waiting.await.unwrap(), WaitOutcome::Shutdown);
        assert_eq!(
            get_input_request(&db, "request-1").unwrap().unwrap().status,
            "pending"
        );
    }
}
