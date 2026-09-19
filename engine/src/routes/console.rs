use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::Deserialize;
use serde_json::json;
use std::{convert::Infallible, sync::Arc};
use tokio::sync::broadcast::error::RecvError;

use crate::{AppState, console::CommandSession, routes::admin::get_current_user_id};

#[derive(Debug, Deserialize, Default)]
pub struct CommandStreamQuery {
    #[serde(rename = "afterSequence")]
    after_sequence: Option<u64>,
    epoch: Option<String>,
}

fn owner_matches(session: &CommandSession, user_id: &str) -> bool {
    session
        .metadata
        .get("owner_user_id")
        .is_some_and(|owner| owner == user_id)
}

pub async fn handle_command_session_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<CommandStreamQuery>,
) -> Response {
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return json_error(StatusCode::UNAUTHORIZED, "Unauthorized");
    };
    let Some(session) = state.command_events.session(&session_id).await else {
        return json_error(StatusCode::NOT_FOUND, "Command session not found");
    };
    if !owner_matches(&session, &user_id) {
        return json_error(StatusCode::NOT_FOUND, "Command session not found");
    }
    command_stream(state, Some(session_id), None, query)
}

fn command_stream(
    state: Arc<AppState>,
    session_id: Option<String>,
    runner_id: Option<String>,
    query: CommandStreamQuery,
) -> Response {
    // Subscribe before the snapshot. Notifications only wake replay: sequence
    // ordering then comes from the store, not racing producer broadcasts.
    let mut rx = state.command_events.subscribe();
    let epoch = state.command_events.epoch().to_string();
    let reset = query
        .epoch
        .as_deref()
        .is_some_and(|previous| previous != epoch);
    let mut cursor = if reset {
        0
    } else {
        query.after_sequence.unwrap_or(0)
    };
    let stream = async_stream::stream! {
        yield Ok::<_, Infallible>(format!("{}\n", json!({"type": "command_stream_start", "epoch": epoch, "reset": reset})));
        let mut initial = true;
        loop {
            if state.command_events.replay_was_truncated(session_id.as_deref(), runner_id.as_deref(), cursor).await {
                yield Ok(format!("{}\n", json!({"type": "command_gap", "message": "Earlier output exceeded the replay limit; showing retained output."})));
            }
            let mut replay = if let Some(id) = session_id.as_deref() {
                state.command_events.replay_since(id, Some(cursor)).await
            } else {
                Vec::new()
            };
            replay.sort_by_key(|event| event.sequence);
            for event in replay {
                cursor = event.sequence;
                yield Ok(event.to_ndjson_line());
            }
            if initial {
                yield Ok(format!("{}\n", json!({"type": "command_replay_complete", "session_id": session_id, "runner_id": runner_id})));
                initial = false;
            }
            tokio::select! {
                event = rx.recv() => match event {
                    Err(RecvError::Closed) => break,
                    // Lag is repaired by the next replay, or explicitly reported if trimmed.
                    Ok(_) | Err(RecvError::Lagged(_)) => {},
                },
                _ = tokio::time::sleep(std::time::Duration::from_secs(25)) => {
                    yield Ok(format!("{}\n", json!({"type": "ping"})));
                }
            }
        }
    };
    Response::builder()
        .header("content-type", "application/x-ndjson")
        .header("cache-control", "no-cache")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(stream))
        .unwrap()
}

fn json_error(status: StatusCode, message: &str) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(json!({"error": message}).to_string()))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::{CommandEventStore, CommandOrigin, CommandSourceKind};
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn ownership_is_required_not_just_an_authenticated_identity() {
        let store = CommandEventStore::new();
        let mut session = store
            .start_session(
                CommandOrigin::new(CommandSourceKind::Pty),
                None,
                BTreeMap::new(),
            )
            .await;
        assert!(!owner_matches(&session, "alice"));
        session
            .metadata
            .insert("owner_user_id".into(), "alice".into());
        assert!(owner_matches(&session, "alice"));
        assert!(!owner_matches(&session, "bob"));
    }
}
