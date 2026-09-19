use axum::{
    Extension, Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;

fn context_matches(headers: &HeaderMap, session_id: &str) -> bool {
    headers
        .get("x-flamel-session-id")
        .is_none_or(|value| value.to_str().ok() == Some(session_id))
}

fn owns_context(
    state: &AppState,
    headers: &HeaderMap,
    user: &str,
    session_id: &str,
    authenticated: Option<&crate::mcp_auth::AuthenticatedMcpContext>,
) -> bool {
    if !context_matches(headers, session_id) {
        return false;
    }
    if let Some(session) = crate::db::sessions::get_session(&state.db, session_id) {
        return session.user_id.as_deref() == Some(user);
    }
    // Native agents and assistants have authenticated runtime IDs, not rows in
    // the interactive chat table. Never accept a bare forwarded session header.
    authenticated.is_some_and(|context| {
        context.user_id == user && context.session_id.as_deref() == Some(session_id)
    })
}

/// POST /api/internal/nested-event
/// Called by flamel_mcp during sub-agent execution. Wraps the child event in
/// an NDJSON envelope and broadcasts it to the parent session's live stream so
/// the frontend can render nested tool activity inside the parent AgentRow.
///
/// Body: `{"sessionId": "...", "runId": "...", "event": {...}}`
/// The `runId` correlates events to a specific sub-agent invocation — the
/// parent tool_result output begins with `AGENT_RUN:{runId}` so the frontend
/// can attach buffered events to the right AgentRow.
pub async fn handle_nested_event(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    authenticated: Option<Extension<crate::mcp_auth::AuthenticatedMcpContext>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let session_id = match body.get("sessionId").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "sessionId required"})),
            );
        }
    };
    let run_id = match body.get("runId").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "runId required"})),
            );
        }
    };
    let event = body.get("event").cloned().unwrap_or(Value::Null);
    let Some(user) = super::admin::get_current_user_id(&state, &headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Authentication required"})),
        );
    };
    if !owns_context(
        &state,
        &headers,
        &user,
        &session_id,
        authenticated.as_ref().map(|value| &value.0),
    ) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Session not found"})),
        );
    }
    if session_id.is_empty()
        || run_id.is_empty()
        || run_id.len() > 200
        || !event.is_object()
        || event.to_string().len() > 1_048_576
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Invalid agent event"})),
        );
    }
    let parent_run_id = body
        .get("parentRunId")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty());

    if let Err(error) = crate::db::agent_tracking::record(
        &state.db,
        &user,
        &session_id,
        &run_id,
        parent_run_id.unwrap_or(""),
        &event,
    ) {
        tracing::error!(%error, "Failed to save agent receipt");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"Agent receipt could not be saved"})),
        );
    }

    let wrapped = json!({
        "type": "nested_agent_event",
        "runId": run_id,
        "parentRunId": parent_run_id,
        "event": event,
    });

    let streams = state.live_streams.lock().await;
    if let Some(tx) = streams.get(&session_id) {
        let _ = tx.send(format!("{}\n", wrapped));
        (StatusCode::OK, Json(json!({"ok": true})))
    } else {
        // The active stream's relay journals live events. Late results still
        // need a durable event when that relay has already closed.
        state.realtime.publish_value(&session_id, None, &wrapped);
        (
            StatusCode::OK,
            Json(json!({"ok": true, "note": "saved for reconnect"})),
        )
    }
}

/// The status belongs to the authenticated session, not an MCP process.
pub async fn handle_batch_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    authenticated: Option<Extension<crate::mcp_auth::AuthenticatedMcpContext>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let Some(user) = super::admin::get_current_user_id(&state, &headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Authentication required"})),
        );
    };
    let session = body["sessionId"].as_str().unwrap_or("");
    if !owns_context(
        &state,
        &headers,
        &user,
        session,
        authenticated.as_ref().map(|value| &value.0),
    ) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Session not found"})),
        );
    }
    match crate::db::agent_tracking::batch_status(
        &state.db,
        &user,
        session,
        body["parentRunId"].as_str().unwrap_or(""),
        body["batch_id"].as_str(),
    ) {
        Ok(status) => (StatusCode::OK, Json(status)),
        Err(error) => {
            tracing::error!(%error, "Failed to read agent receipt");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error":"Agent status is unavailable; completion is unconfirmed"})),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn late_results_are_saved_and_another_account_cannot_write_or_read_them() {
        let state = AppState::new(":memory:", 0);
        state.db.conn().execute("INSERT INTO sessions(id,project_path,created_at,updated_at,user_id) VALUES('s','/tmp','now','now','alice')",[]).unwrap();
        let mut owner = HeaderMap::new();
        owner.insert("x-flamel-user-id", "alice".parse().unwrap());
        let mut other = HeaderMap::new();
        other.insert("x-flamel-user-id", "bob".parse().unwrap());
        let body = json!({"sessionId":"s","runId":"child","event":{"type":"nested_start","description":"Inspect logs"}});
        assert_eq!(
            handle_nested_event(
                State(state.clone()),
                other.clone(),
                None,
                Json(body.clone())
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            handle_nested_event(State(state.clone()), owner.clone(), None, Json(body))
                .await
                .0,
            StatusCode::OK
        );
        let mut events = state.realtime.subscribe();
        assert_eq!(handle_nested_event(State(state.clone()),owner.clone(),None,Json(json!({"sessionId":"s","runId":"child","event":{"type":"nested_done","ok":true,"output":"Cause found"}}))).await.0,StatusCode::OK);
        let received = events
            .try_recv()
            .expect("late event was journaled and published without a live stream");
        assert_eq!(received.session_id, "s");
        let lookup = json!({"sessionId":"s"});
        assert_eq!(
            handle_batch_status(State(state.clone()), other, None, Json(lookup.clone()))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        let (code, Json(receipt)) =
            handle_batch_status(State(state), owner, None, Json(lookup)).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(receipt["agents"][0]["output"], "Cause found");
    }

    #[tokio::test]
    async fn signed_session_context_cannot_be_retargeted_to_another_owned_session() {
        let state = AppState::new(":memory:", 0);
        state.db.conn().execute("INSERT INTO sessions(id,project_path,created_at,updated_at,user_id) VALUES('other-session','/tmp','now','now','alice')",[]).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-flamel-user-id", "alice".parse().unwrap());
        headers.insert("x-flamel-session-id", "signed-session".parse().unwrap());
        let event =
            json!({"sessionId":"other-session","runId":"child","event":{"type":"nested_start"}});
        assert_eq!(
            handle_nested_event(State(state.clone()), headers.clone(), None, Json(event))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            handle_batch_status(
                State(state.clone()),
                headers,
                None,
                Json(json!({"sessionId":"other-session"}))
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            crate::db::agent_tracking::batch_status(&state.db, "alice", "other-session", "", None)
                .unwrap()["status"],
            "not_found"
        );
    }
}
