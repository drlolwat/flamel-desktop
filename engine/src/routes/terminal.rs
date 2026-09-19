//! Account-owned PTYs. Byte output is replayable; raw stdin is never journaled.
use axum::{
    Json,
    extract::{Path, Query, State, WebSocketUpgrade, ws::Message},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse,
        sse::{Event, Sse},
    },
};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use crate::routes::admin::{get_current_user_id, get_user_home};
use crate::shell::{kill_terminal, resize_terminal, spawn_terminal, write_terminal};
use crate::{
    AppState,
    console::{CommandLifecycleStatus, CommandOrigin, CommandSourceKind},
};

const MAX_TERMINALS_PER_USER: usize = 16;
const MAX_INPUT_BYTES: usize = 65_536;

#[derive(Deserialize)]
pub struct StartRequest {
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

#[derive(Serialize)]
pub struct StartResponse {
    pub session_id: String,
    pub command_session_id: String,
    pub command: String,
    pub cwd: String,
}

#[derive(Default, Deserialize)]
pub struct StreamQuery {
    #[serde(rename = "afterSequence")]
    after_sequence: Option<u64>,
    ticket: Option<String>,
}

fn current_user(state: &Arc<AppState>, headers: &HeaderMap) -> Result<String, StatusCode> {
    get_current_user_id(state, headers).ok_or(StatusCode::UNAUTHORIZED)
}

fn check_owner(owner: Option<&str>, user_id: &str) -> Result<(), StatusCode> {
    match owner {
        Some(owner) if owner == user_id => Ok(()),
        // Do not reveal another account's terminal IDs.
        _ => Err(StatusCode::NOT_FOUND),
    }
}

pub(crate) fn owns_terminal(
    state: &Arc<AppState>,
    id: &str,
    user_id: &str,
) -> Result<(), StatusCode> {
    let sessions = state.terminal_sessions.lock().unwrap();
    check_owner(
        sessions
            .get(id)
            .map(|session| session.owner_user_id.as_str()),
        user_id,
    )
}

fn valid_size(cols: u16, rows: u16) -> bool {
    (2..=1000).contains(&cols) && (2..=1000).contains(&rows)
}

pub async fn handle_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user_id = match current_user(&state, &headers) {
        Ok(id) => id,
        Err(status) => return status.into_response(),
    };
    let sessions = state.terminal_sessions.lock().unwrap();
    let rows: Vec<_> = sessions.iter().filter(|(_, session)| session.owner_user_id == user_id).map(|(id, session)| json!({
        "session_id": id, "command_session_id": session.command_session_id,
        "cwd": session.cwd, "command": session.command,
        "exited": session.exited.load(Ordering::Acquire), "exit_code": *session.exit_code.lock().unwrap(),
    })).collect();
    Json(json!({"terminals": rows})).into_response()
}

pub async fn handle_start(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<StartRequest>,
) -> impl IntoResponse {
    let user_id = match current_user(&state, &headers) {
        Ok(id) => id,
        Err(status) => return status.into_response(),
    };
    #[cfg(windows)]
    let default_command = "powershell.exe -NoLogo -NoProfile";
    #[cfg(not(windows))]
    let default_command = "exec \"${SHELL:-/bin/bash}\" -l";
    let command = req.command.unwrap_or_else(|| default_command.to_string());
    if command.is_empty() || command.len() > MAX_INPUT_BYTES {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let is_admin = false;
    let user_home = state.home_dir.clone();
    let requested_cwd = req
        .cwd
        .unwrap_or_else(|| user_home.to_string_lossy().into_owned());
    let access_root = super::path_guard::access_root(&user_home, is_admin);
    let cwd = match super::path_guard::guard_existing(&access_root, &requested_cwd) {
        Ok(path) if path.is_dir() => path.to_string_lossy().into_owned(),
        Ok(_) => {
            return (StatusCode::BAD_REQUEST, "Terminal cwd must be a directory").into_response();
        }
        Err((status, Json(body))) => return (status, Json(body)).into_response(),
    };
    let cols = req.cols.unwrap_or(120);
    let rows = req.rows.unwrap_or(30);
    if !valid_size(cols, rows) {
        return (StatusCode::BAD_REQUEST, "Invalid terminal size").into_response();
    }
    let sandbox_root =
        (!state.local_mode && !is_admin).then(|| user_home.to_string_lossy().into_owned());
    let session_id = uuid::Uuid::new_v4().to_string();
    let metadata = BTreeMap::from([
        ("terminal_session_id".to_string(), session_id.clone()),
        ("owner_user_id".to_string(), user_id.clone()),
    ]);
    let command_session = state
        .command_events
        .start_session(
            CommandOrigin::new(CommandSourceKind::Pty).with_label(format!("Terminal: {command}")),
            Some(cwd.clone()),
            metadata.clone(),
        )
        .await;
    let started = match state
        .command_events
        .start_command(
            &command_session.id,
            Some(command.clone()),
            Some(cwd.clone()),
            Some(command.clone()),
            metadata,
        )
        .await
    {
        Ok(started) => started,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    };
    // Quota check and spawn share one lock so concurrent starts cannot bypass it.
    let result = {
        let mut sessions = state.terminal_sessions.lock().unwrap();
        sessions.retain(|_, session| {
            !session.exited.load(Ordering::Acquire)
                || session.started_at.elapsed() < Duration::from_secs(3600)
        });
        let mut completed = sessions
            .iter()
            .filter(|(_, session)| {
                session.owner_user_id == user_id && session.exited.load(Ordering::Acquire)
            })
            .map(|(id, session)| (id.clone(), session.started_at))
            .collect::<Vec<_>>();
        completed.sort_by_key(|(_, started)| *started);
        for (id, _) in completed.iter().take(completed.len().saturating_sub(32)) {
            sessions.remove(id);
        }
        if sessions
            .values()
            .filter(|session| {
                session.owner_user_id == user_id && !session.exited.load(Ordering::Acquire)
            })
            .count()
            >= MAX_TERMINALS_PER_USER
        {
            Err("Too many running terminals; close one first".to_string())
        } else {
            spawn_terminal(
                user_id,
                &command,
                &cwd,
                cols,
                rows,
                sandbox_root.as_deref(),
                Arc::clone(&state.command_events),
                command_session.id.clone(),
                started.run.id.clone(),
                started.block.id.clone(),
            )
            .map(|session| {
                sessions.insert(session_id.clone(), session);
            })
        }
    };
    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(StartResponse {
                session_id,
                command_session_id: command_session.id,
                command,
                cwd,
            }),
        )
            .into_response(),
        Err(error) => {
            let _ = state
                .command_events
                .finish_command(&started.run.id, CommandLifecycleStatus::Failed, None)
                .await;
            let _ = state
                .command_events
                .close_session(&command_session.id, CommandLifecycleStatus::Failed)
                .await;
            (StatusCode::BAD_REQUEST, error).into_response()
        }
    }
}

pub async fn handle_delete(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> StatusCode {
    let user_id = match current_user(&state, &headers) {
        Ok(id) => id,
        Err(status) => return status,
    };
    if let Err(status) = owns_terminal(&state, &id, &user_id) {
        return status;
    }
    let removed = state.terminal_sessions.lock().unwrap().remove(&id);
    if let Some(mut session) = removed {
        let should_close = !session.event_closed.swap(true, Ordering::AcqRel);
        kill_terminal(&mut session);
        if should_close {
            let _ = state
                .command_events
                .finish_command(&session.run_id, CommandLifecycleStatus::Cancelled, None)
                .await;
            let _ = state
                .command_events
                .close_session(&session.command_session_id, CommandLifecycleStatus::Closed)
                .await;
        }
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

#[derive(Deserialize)]
pub struct InputRequest {
    pub text: Option<String>,
    pub data: Option<String>,
}

fn input_bytes(req: InputRequest) -> Result<Vec<u8>, StatusCode> {
    let bytes = match (req.text, req.data) {
        (Some(text), None) => text.into_bytes(),
        (None, Some(data)) if data.len() <= MAX_INPUT_BYTES * 2 => {
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| StatusCode::BAD_REQUEST)?
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    Ok(bytes)
}

fn send_input(state: &Arc<AppState>, id: &str, bytes: &[u8]) -> StatusCode {
    let sessions = state.terminal_sessions.lock().unwrap();
    match sessions.get(id) {
        Some(session) if !session.exited.load(Ordering::Acquire) => {
            // Never copy stdin into events, logs or replay, including echo-disabled passwords.
            match write_terminal(session, bytes) {
                Ok(()) => StatusCode::OK,
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
            }
        }
        Some(_) => StatusCode::CONFLICT,
        None => StatusCode::NOT_FOUND,
    }
}

pub async fn handle_input(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<InputRequest>,
) -> StatusCode {
    let user_id = match current_user(&state, &headers) {
        Ok(id) => id,
        Err(status) => return status,
    };
    if let Err(status) = owns_terminal(&state, &id, &user_id) {
        return status;
    }
    match input_bytes(req) {
        Ok(bytes) => send_input(&state, &id, &bytes),
        Err(status) => status,
    }
}

#[derive(Deserialize)]
pub struct ResizeRequest {
    pub cols: u16,
    pub rows: u16,
}

pub async fn handle_resize(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ResizeRequest>,
) -> StatusCode {
    let user_id = match current_user(&state, &headers) {
        Ok(id) => id,
        Err(status) => return status,
    };
    if let Err(status) = owns_terminal(&state, &id, &user_id) {
        return status;
    }
    if !valid_size(req.cols, req.rows) {
        return StatusCode::BAD_REQUEST;
    }
    let sessions = state.terminal_sessions.lock().unwrap();
    match sessions.get(&id) {
        Some(session) => {
            resize_terminal(session, req.cols, req.rows);
            StatusCode::OK
        }
        None => StatusCode::NOT_FOUND,
    }
}

pub async fn handle_output(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
) -> impl IntoResponse {
    let user_id = match current_user(&state, &headers) {
        Ok(id) => id,
        Err(status) => return status.into_response(),
    };
    if let Err(status) = owns_terminal(&state, &id, &user_id) {
        return status.into_response();
    }
    let sessions = state.terminal_sessions.lock().unwrap();
    let Some(session) = sessions.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut history = session.output_history.lock().unwrap();
    let (frames, truncated) = history.replay(query.after_sequence.unwrap_or(history.poll_cursor));
    if let Some(last) = frames.last() {
        history.poll_cursor = last.sequence;
    }
    let bytes: Vec<_> = frames
        .iter()
        .flat_map(|frame| frame.data.iter().copied())
        .collect();
    Json(json!({"output": String::from_utf8_lossy(&bytes), "exited": session.exited.load(Ordering::Acquire), "exit_code": *session.exit_code.lock().unwrap(), "after_sequence": history.poll_cursor, "truncated": truncated})).into_response()
}

pub async fn handle_status(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user_id = match current_user(&state, &headers) {
        Ok(id) => id,
        Err(status) => return status.into_response(),
    };
    if let Err(status) = owns_terminal(&state, &id, &user_id) {
        return status.into_response();
    }
    let sessions = state.terminal_sessions.lock().unwrap();
    let Some(session) = sessions.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let exited = session.exited.load(Ordering::Acquire);
    Json(json!({"running": !exited, "exited": exited, "exit_code": *session.exit_code.lock().unwrap(), "cwd": session.cwd, "command": session.command, "command_session_id": session.command_session_id})).into_response()
}

/// Both transports replay the same byte frames. Broadcast is only a wake-up:
/// reading history prevents subscribe/replay races and duplicate bytes.
fn output_stream(
    state: Arc<AppState>,
    id: String,
    after_sequence: u64,
) -> impl futures::Stream<Item = Value> + Send {
    async_stream::stream! {
        let receiver = state.terminal_sessions.lock().unwrap().get(&id).map(|session| session.output_tx.subscribe());
        let Some(mut receiver) = receiver else { yield json!({"type": "missing"}); return; };
        let mut cursor = after_sequence;
        loop {
            let snapshot = {
                let sessions = state.terminal_sessions.lock().unwrap();
                sessions.get(&id).map(|session| {
                    let (frames, truncated) = session.output_history.lock().unwrap().replay(cursor);
                    (frames, truncated, session.exited.load(Ordering::Acquire), *session.exit_code.lock().unwrap())
                })
            };
            let Some((frames, truncated, exited, exit_code)) = snapshot else { yield json!({"type": "missing"}); break; };
            if truncated { yield json!({"type": "gap", "message": "Earlier terminal output exceeded the replay limit; showing the retained tail."}); }
            for frame in frames {
                cursor = frame.sequence;
                if !frame.data.is_empty() {
                    yield json!({"type": "output", "sequence": cursor, "data": base64::engine::general_purpose::STANDARD.encode(&frame.data)});
                }
            }
            if exited { yield json!({"type": "exit", "exit_code": exit_code, "sequence": cursor}); break; }
            tokio::select! {
                result = receiver.recv() => if matches!(result, Err(tokio::sync::broadcast::error::RecvError::Closed)) { break; },
                _ = tokio::time::sleep(Duration::from_secs(15)) => { yield json!({"type": "ping"}); }
            }
        }
    }
}

pub async fn handle_stream(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
) -> impl IntoResponse {
    let user_id = match current_user(&state, &headers) {
        Ok(id) => id,
        Err(status) => return status.into_response(),
    };
    if let Err(status) = owns_terminal(&state, &id, &user_id) {
        return status.into_response();
    }
    let stream = output_stream(state, id, query.after_sequence.unwrap_or(0)).map(|payload| {
        Ok::<_, std::convert::Infallible>(Event::default().data(payload.to_string()))
    });
    Sse::new(stream).into_response()
}

pub async fn handle_ws(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let user_id = if let Some(ticket) = query.ticket.as_deref() {
        // Same one-use identity ticket as realtime; access stays terminal-owner-only.
        match state.realtime.consume_ticket(ticket).await {
            Some(id) => id,
            None => return StatusCode::UNAUTHORIZED.into_response(),
        }
    } else {
        match current_user(&state, &headers) {
            Ok(id) => id,
            Err(status) => return status.into_response(),
        }
    };
    if let Err(status) = owns_terminal(&state, &id, &user_id) {
        return status.into_response();
    }
    ws.max_message_size(MAX_INPUT_BYTES * 2)
        .on_upgrade(move |socket| terminal_ws(socket, state, id, query.after_sequence.unwrap_or(0)))
        .into_response()
}

pub(crate) async fn terminal_ws<S>(socket: S, state: Arc<AppState>, id: String, cursor: u64)
where
    S: futures::Sink<Message, Error = axum::Error>
        + futures::Stream<Item = Result<Message, axum::Error>>
        + Unpin,
{
    let (mut sender, mut receiver) = socket.split();
    let output = output_stream(Arc::clone(&state), id.clone(), cursor);
    futures::pin_mut!(output);
    loop {
        tokio::select! {
            payload = output.next() => match payload {
                Some(payload) => {
                    let finished = matches!(payload["type"].as_str(), Some("exit" | "missing"));
                    if sender.send(Message::Text(payload.to_string().into())).await.is_err() || finished { break; }
                }
                None => break,
            },
            message = receiver.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    let status = handle_ws_message(&state, &id, &text);
                    if status != StatusCode::OK && sender.send(Message::Text(json!({"type": "input_error", "status": status.as_u16()}).to_string().into())).await.is_err() { break; }
                }
                Some(Ok(Message::Ping(data))) => if sender.send(Message::Pong(data)).await.is_err() { break; },
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                _ => {},
            }
        }
    }
}

fn handle_ws_message(state: &Arc<AppState>, id: &str, text: &str) -> StatusCode {
    let Ok(payload) = serde_json::from_str::<Value>(text) else {
        return StatusCode::BAD_REQUEST;
    };
    match payload["type"].as_str() {
        Some("input") => match serde_json::from_value::<InputRequest>(payload)
            .map_err(|_| StatusCode::BAD_REQUEST)
            .and_then(input_bytes)
        {
            Ok(bytes) => send_input(state, id, &bytes),
            Err(status) => status,
        },
        Some("resize") => {
            let Ok(req) = serde_json::from_value::<ResizeRequest>(payload) else {
                return StatusCode::BAD_REQUEST;
            };
            if !valid_size(req.cols, req.rows) {
                return StatusCode::BAD_REQUEST;
            }
            let sessions = state.terminal_sessions.lock().unwrap();
            if let Some(session) = sessions.get(id) {
                resize_terminal(session, req.cols, req.rows);
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        }
        _ => StatusCode::BAD_REQUEST,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_owner_required_and_input_validation_is_strict() {
        assert_eq!(check_owner(Some("alice"), "alice"), Ok(()));
        assert_eq!(
            check_owner(Some("alice"), "bob"),
            Err(StatusCode::NOT_FOUND)
        );
        assert_eq!(check_owner(None, "alice"), Err(StatusCode::NOT_FOUND));
        assert!(
            input_bytes(InputRequest {
                text: None,
                data: Some("%%%".into())
            })
            .is_err()
        );
        assert!(
            input_bytes(InputRequest {
                text: Some("hello".into()),
                data: Some("aA==".into())
            })
            .is_err()
        );
        assert_eq!(
            input_bytes(InputRequest {
                text: None,
                data: Some("aA==".into())
            })
            .unwrap(),
            b"h"
        );
        assert!(!valid_size(0, 30));
        assert!(!valid_size(120, u16::MAX));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn websocket_ticket_and_http_control_are_owner_scoped() {
        let state = AppState::new(":memory:", 0);
        let id = uuid::Uuid::new_v4().to_string();
        let metadata = BTreeMap::from([("owner_user_id".into(), "alice".into())]);
        let command_session = state
            .command_events
            .start_session(
                CommandOrigin::new(CommandSourceKind::Pty),
                None,
                metadata.clone(),
            )
            .await;
        let started = state
            .command_events
            .start_command(
                &command_session.id,
                Some("sleep 30".into()),
                None,
                None,
                metadata,
            )
            .await
            .unwrap();
        let terminal = spawn_terminal(
            "alice".into(),
            "sleep 30",
            std::env::temp_dir().to_str().unwrap(),
            80,
            24,
            None,
            Arc::clone(&state.command_events),
            command_session.id.clone(),
            started.run.id,
            started.block.id,
        )
        .unwrap();
        state
            .terminal_sessions
            .lock()
            .unwrap()
            .insert(id.clone(), terminal);
        let mut bob_headers = HeaderMap::new();
        bob_headers.insert("x-flamel-user-id", "bob".parse().unwrap());
        let input_status = handle_input(
            Path(id.clone()),
            State(Arc::clone(&state)),
            bob_headers.clone(),
            Json(InputRequest {
                text: Some("should not reach stdin".into()),
                data: None,
            }),
        )
        .await;
        let console_response = crate::routes::console::handle_command_session_stream(
            State(Arc::clone(&state)),
            bob_headers,
            Path(command_session.id),
            Query(Default::default()),
        )
        .await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new()
            .route("/terminal/{id}", axum::routing::get(handle_ws))
            .with_state(Arc::clone(&state));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let (bob_ticket, _) = state.realtime.issue_ticket("bob".into()).await;
        let wrong_owner = tokio_tungstenite::connect_async(format!(
            "ws://{address}/terminal/{id}?ticket={bob_ticket}"
        ))
        .await;
        let (alice_ticket, _) = state.realtime.issue_ticket("alice".into()).await;
        let accepted = tokio_tungstenite::connect_async(format!(
            "ws://{address}/terminal/{id}?ticket={alice_ticket}"
        ))
        .await;
        let reused = tokio_tungstenite::connect_async(format!(
            "ws://{address}/terminal/{id}?ticket={alice_ticket}"
        ))
        .await;
        server.abort();
        let removed = state.terminal_sessions.lock().unwrap().remove(&id);
        if let Some(mut terminal) = removed {
            kill_terminal(&mut terminal);
        }
        assert_eq!(input_status, StatusCode::NOT_FOUND);
        assert_eq!(console_response.status(), StatusCode::NOT_FOUND);
        assert!(
            matches!(wrong_owner, Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status() == 404)
        );
        assert!(accepted.is_ok());
        assert!(
            matches!(reused, Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status() == 401)
        );
    }
}
