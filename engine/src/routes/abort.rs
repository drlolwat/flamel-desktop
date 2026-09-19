use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;

pub async fn abort_session(state: &AppState, session_id: &str) -> bool {
    if let Some(run_id) = crate::db::sessions::get_session(&state.db, session_id)
        .and_then(|session| session.active_request_id)
        && let Err(error) = crate::routes::chat::continuation::cancel_for_run(&state.db, &run_id)
    {
        tracing::error!(%error, session_id, "could not persist automatic continuation cancellation");
    }
    let local_signaled = state.session_pool.sigint_session(session_id).await;
    let runner_signaled = false;
    local_signaled || runner_signaled
}

pub fn signal_session_abort_controller(state: &AppState, session_id: &str) -> bool {
    let request_ids = state
        .request_to_session
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, mapped_session_id)| mapped_session_id.as_str() == session_id)
        .map(|(request_id, _)| request_id.clone())
        .collect::<Vec<_>>();
    let controllers = state.abort_controllers.lock().unwrap();
    request_ids
        .iter()
        .filter_map(|request_id| controllers.get(request_id))
        .any(|sender| sender.send(true).is_ok())
}

pub fn signal_request_abort_controller(state: &AppState, request_id: &str) -> bool {
    if let Err(error) = crate::routes::chat::continuation::cancel_for_run(&state.db, request_id) {
        tracing::error!(%error, request_id, "could not persist automatic continuation cancellation");
    }
    state
        .abort_controllers
        .lock()
        .unwrap()
        .get(request_id)
        .is_some_and(|sender| sender.send(true).is_ok())
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbortSessionRequest {
    expected_request_id: Option<String>,
}

pub async fn handle_abort_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(request): Json<AbortSessionRequest>,
) -> Response {
    let Some(user_id) = crate::routes::admin::get_current_user_id(&state, &headers) else {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))).into_response();
    };
    let Some(session) = crate::db::sessions::get_session(&state.db, &session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Session not found"})),
        )
            .into_response();
    };
    if session.user_id.as_deref() != Some(user_id.as_str()) {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))).into_response();
    }
    if let Some(expected_request_id) = request.expected_request_id.as_deref() {
        if session.active_request_id.as_deref() != Some(expected_request_id) {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "The requested turn is no longer active",
                    "activeRequestId": session.active_request_id,
                })),
            )
                .into_response();
        }
    }

    let (controller_signaled, provider_signaled) =
        if let Some(run_id) = session.active_request_id.as_ref() {
            let Json(result) = handle_abort(
                State(Arc::clone(&state)),
                headers.clone(),
                Path(run_id.clone()),
            )
            .await;
            (
                result["aborted"].as_bool().unwrap_or(false),
                result["sigint"].as_bool().unwrap_or(false),
            )
        } else {
            (false, false)
        };
    state.realtime.publish_value(
        &session_id,
        session.active_request_id.as_deref(),
        &json!({
            "type": "interrupt_requested",
            "sessionId": session_id,
            "requestId": session.active_request_id,
        }),
    );
    (
        StatusCode::ACCEPTED,
        Json(json!({
            "ok": true,
            "aborted": controller_signaled || provider_signaled,
            "requestId": session.active_request_id,
        })),
    )
        .into_response()
}

pub async fn handle_abort(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Json<Value> {
    let Some(user_id) = crate::routes::admin::get_current_user_id(&state, &headers) else {
        return Json(json!({"ok":false,"error":"Forbidden"}));
    };
    let session_id: Option<String> = state
        .db
        .conn()
        .query_row(
            "SELECT session_id FROM chat_runs WHERE id=?1 AND user_id=?2",
            rusqlite::params![request_id, user_id],
            |r| r.get(0),
        )
        .ok();
    if session_id.is_none() {
        return Json(json!({"ok":false,"error":"Request not found"}));
    }
    let owner = if let Some(sid) = &session_id {
        state.session_pool.turn_owner(sid).await
    } else {
        None
    };
    let executing: bool=state.db.conn().query_row("SELECT EXISTS(SELECT 1 FROM chat_runs WHERE id=?1 AND state IN ('preparing','running') AND lease_owner IS NOT NULL)",[&request_id],|r|r.get(0)).unwrap_or(false);
    let signaled = signal_request_abort_controller(&state, &request_id);
    let provider_signaled = if executing {
        if let Some(sid) = &session_id {
            state.session_pool.sigint_owned_turn(sid, owner).await
        } else {
            false
        }
    } else {
        false
    };
    if !executing {
        if let Some(sid) = &session_id {
            state.realtime.publish_value(sid,Some(&request_id),&json!({"type":"continuity","status":"cancelled","completionManaged":"server","isSessionWarm":false,"requiresResume":false}));
        }
    }
    Json(json!({"ok":true,"aborted":signaled || session_id.is_some(),"sigint":provider_signaled}))
}
