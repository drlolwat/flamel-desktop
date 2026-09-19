use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::broadcast;

use crate::AppState;
use crate::claude::provider::{ProviderCredentials as ProvCreds, ProviderKind};
use crate::compaction::{
    CompactionPhase, bounded_context_tokens, codex_compaction_attempts,
    compaction_checkpoint_payload, compaction_complete_line, compaction_complete_payload,
    compaction_failed_line, compaction_phase_line, compaction_summary_delta_line,
    summarize_codex_compaction_context,
};
use crate::db::sessions::{
    SessionMeta, clear_session_claude_id, delete_session, fork_session, get_session,
    get_session_input_tokens, has_visible_session_messages, list_sessions,
    load_latest_compaction_history, load_persisted_session_history,
    load_persisted_session_messages, persist_compaction_checkpoint, reconcile_session_active_plan,
    save_compaction_summary, session_history_revision, set_session_active_plan, update_session,
    update_session_continuity,
};
use crate::db::shares::get_share_by_session_id;
use crate::routes::admin::get_current_user_id;

#[derive(Deserialize)]
pub struct ListQuery {
    q: Option<String>,
    #[serde(rename = "projectPath")]
    project_path: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSearchQuery {
    q: Option<String>,
    limit: Option<usize>,
    include_archived: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactSessionRequest {
    queued_prompt: Option<bool>,
    recovery_strategy: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkSessionRequest {
    title: Option<String>,
}

fn parse_json_body<T>(body: &Bytes) -> T
where
    T: for<'de> Deserialize<'de> + Default,
{
    if body.is_empty() {
        return T::default();
    }
    serde_json::from_slice(body).unwrap_or_default()
}

async fn session_with_runtime(state: &AppState, session: &SessionMeta) -> Value {
    let is_session_warm = state.session_pool.is_session_warm(&session.id).await;
    session_with_runtime_state(state, session, is_session_warm)
}

fn session_with_runtime_state(
    state: &AppState,
    session: &SessionMeta,
    is_session_warm: bool,
) -> Value {
    let mut value = serde_json::to_value(session).unwrap();
    if let Some(obj) = value.as_object_mut() {
        let is_running = matches!(
            session.continuity_status.as_str(),
            "queued" | "active" | "recovering"
        );
        let requires_resume = session.claude_session_id.is_some()
            && !is_session_warm
            && !is_running
            && (has_visible_session_messages(&state.db, &session.id)
                || legacy_jsonl_path(session).is_some_and(|path| path.is_file()));
        obj.insert("isSessionWarm".to_string(), json!(is_session_warm));
        obj.insert("requiresResume".to_string(), json!(requires_resume));
        match crate::git_operations::workspaces::load(
            &state.db,
            &session.id,
            session.user_id.as_deref().unwrap_or(""),
        ) {
            Ok(Some(workspace)) => {
                obj.insert("workspace".to_string(), json!(workspace));
            }
            Ok(None) => {}
            Err(error) => tracing::error!(
                session_id = session.id,
                "workspace metadata could not be loaded: {error}"
            ),
        }
    }
    value
}

pub async fn handle_workspace_review(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    };
    let Some(session) =
        get_session(&state.db, &id).filter(|session| session.user_id.as_deref() == Some(&user_id))
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Session not found"})),
        );
    };
    let home = crate::routes::admin::get_user_home(&state, Some(&user_id));
    let access = crate::routes::path_guard::access_root(&home, state.local_mode);
    if let Err(error) = crate::routes::path_guard::guard_existing(&access, &session.project_path) {
        return error;
    }
    match tokio::task::spawn_blocking(move || {
        crate::git_operations::workspaces::review(&state.db, &id, &user_id)
    })
    .await
    {
        Ok(Ok(review)) => (StatusCode::OK, Json(review)),
        Ok(Err(error)) => (StatusCode::CONFLICT, Json(json!({"error": error}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error.to_string()})),
        ),
    }
}

fn session_history(state: &AppState, session_id: &str) -> Vec<Value> {
    let mut db_rows = load_persisted_session_history(&state.db, session_id);
    if !db_rows.iter().any(|(role, _)| role == "compaction")
        && let Some((summary, pre_tokens, model, created_at)) =
            load_latest_compaction_history(&state.db, session_id)
    {
        db_rows.push((
            "compaction".to_string(),
            json!({
                "runId": format!("historic-{created_at}"),
                "phase": "complete",
                "label": "Compaction complete",
                "summary": summary,
                "preTokens": pre_tokens,
                "model": model,
                "recoveryStrategy": "persisted_checkpoint",
            })
            .to_string(),
        ));
    }

    db_rows
        .iter()
        .filter_map(|(role, message)| {
            let msg: Value = serde_json::from_str(message).ok()?;
            match role.as_str() {
                "user" => Some(json!({"type": "user", "message": msg})),
                "assistant" => Some(json!({"type": "assistant", "message": msg})),
                "tool_result" => Some(json!({"type": "user", "message": msg})),
                "compaction" => Some(json!({
                    "type": "compaction",
                    "uuid": msg.get("runId").and_then(Value::as_str),
                    "message": msg,
                })),
                _ => None,
            }
        })
        .collect()
}

fn legacy_jsonl_path(session: &SessionMeta) -> Option<PathBuf> {
    let claude_session_id = session.claude_session_id.as_deref()?;
    let home = std::env::var_os("HOME")?;
    let encoded_path = session.project_path.replace('/', "-");
    Some(
        PathBuf::from(home)
            .join(".claude")
            .join("projects")
            .join(encoded_path)
            .join(format!("{claude_session_id}.jsonl")),
    )
}

fn parse_legacy_jsonl_history(content: &str) -> Vec<Value> {
    content
        .lines()
        .filter_map(|line| {
            let entry: Value = serde_json::from_str(line.trim()).ok()?;
            let entry_type = entry.get("type")?.as_str()?;
            if entry_type != "user" && entry_type != "assistant" {
                return None;
            }
            if entry_type == "user"
                && let Some(content) = entry
                    .get("message")
                    .and_then(|message| message.get("content"))
                    .and_then(Value::as_array)
                && !content.is_empty()
                && content
                    .iter()
                    .all(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
            {
                return None;
            }
            Some(json!({
                "type": entry_type,
                "message": entry.get("message"),
                "uuid": entry.get("uuid"),
                "timestamp": entry.get("timestamp"),
            }))
        })
        .collect()
}

async fn session_history_with_legacy_fallback(
    state: &AppState,
    session: &SessionMeta,
) -> Vec<Value> {
    let messages = session_history(state, &session.id);
    if !messages.is_empty() {
        return messages;
    }

    let Some(jsonl_path) = legacy_jsonl_path(session) else {
        let messages = Vec::new();

        return messages;
    };
    let Ok(content) = std::fs::read_to_string(jsonl_path) else {
        let messages = Vec::new();

        return messages;
    };
    let messages = parse_legacy_jsonl_history(&content);

    messages
}

pub async fn handle_list_sessions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Json<Value> {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            tracing::warn!("list_sessions: no user_id — returning empty");
            return Json(json!([]));
        }
    };
    tracing::info!("list_sessions: user_id={}", user_id);
    let mut sessions = list_sessions(
        &state.db,
        Some(&user_id),
        query.q.as_deref(),
        query.project_path.as_deref(),
    );
    for session in &mut sessions {
        if session.active_plan_id.is_none()
            && reconcile_session_active_plan(&state.db, &session.id).is_some()
            && let Some(reconciled) = get_session(&state.db, &session.id)
        {
            *session = reconciled;
        }
    }
    let warm_session_ids = state.session_pool.warm_session_ids().await;
    let mut values = Vec::with_capacity(sessions.len());
    for session in sessions {
        let is_session_warm = warm_session_ids.contains(&session.id);
        values.push(session_with_runtime_state(
            &state,
            &session,
            is_session_warm,
        ));
    }
    Json(Value::Array(values))
}

pub async fn handle_get_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    match get_session(&state.db, &id) {
        Some(session) => {
            // Hard gate: session must belong to the current user
            if session.user_id.as_deref() != Some(&current_user_id) {
                return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
            }
            let share = get_share_by_session_id(&state.db, &id);
            let mut val = session_with_runtime(&state, &session).await;
            if let Some(s) = share {
                val.as_object_mut()
                    .unwrap()
                    .insert("shareToken".to_string(), json!(s.token));
            }
            (StatusCode::OK, Json(val))
        }
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    }
}

pub async fn handle_session_bootstrap(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let mut session = match get_session(&state.db, &id) {
        Some(session) if session.user_id.as_deref() == Some(&current_user_id) => session,
        Some(_) => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    };

    // Older sessions predate server-owned plan selection. Repair only the safe,
    // unambiguous case and return the resulting state in the same response.
    if session.active_plan_id.is_none() {
        reconcile_session_active_plan(&state.db, &id);
        if let Some(reconciled) = get_session(&state.db, &id) {
            session = reconciled;
        }
    }
    let active_plan = session
        .active_plan_id
        .as_deref()
        .and_then(|plan_id| crate::db::plans::get_plan(&state.db, plan_id));
    let messages = session_history_with_legacy_fallback(&state, &session).await;
    let realtime_sequence = crate::db::realtime::max_sequence(&state.db, &current_user_id, &id);
    let session_value = session_with_runtime(&state, &session).await;
    (
        StatusCode::OK,
        Json(json!({
            "session": session_value,
            "activePlan": active_plan,
            "messages": messages,
            "binding": {
                "sessionId": session.id,
                "streamSessionId": session.id,
                "activeRequestId": session.active_request_id,
                "activePlanId": session.active_plan_id,
                "realtimeSequence": realtime_sequence,
                "updatedAt": session.updated_at,
            }
        })),
    )
}

pub async fn handle_set_session_active_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let session = match get_session(&state.db, &id) {
        Some(session) if session.user_id.as_deref() == Some(&current_user_id) => session,
        Some(_) => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    };
    let active_plan_id = body.get("activePlanId").and_then(Value::as_str);
    if let Some(plan_id) = active_plan_id {
        let valid = crate::db::plans::get_plan(&state.db, plan_id).is_some_and(|plan| {
            plan.meta.session_id.as_deref() == Some(session.id.as_str())
                && !plan.meta.archived
                && crate::db::plans::get_plan_user_id(&state.db, plan_id).as_deref()
                    == Some(current_user_id.as_str())
        });
        if !valid {
            return (
                StatusCode::CONFLICT,
                Json(json!({"error": "Plan is not attached to this session"})),
            );
        }
    }
    match set_session_active_plan(&state.db, &id, active_plan_id) {
        Some(updated) => (StatusCode::OK, Json(serde_json::to_value(updated).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    }
}

pub async fn handle_fork_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let session = match get_session(&state.db, &id) {
        Some(session) => session,
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    };
    if session.user_id.as_deref() != Some(&current_user_id) {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    }

    let request: ForkSessionRequest = parse_json_body(&body);
    let new_session_id = uuid::Uuid::new_v4().to_string();
    match fork_session(&state.db, &id, &new_session_id, request.title.as_deref()) {
        Some(fork) => {
            let value = session_with_runtime(&state, &fork).await;
            (StatusCode::OK, Json(value))
        }
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Failed to fork session"})),
        ),
    }
}

pub async fn handle_update_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    if let Some(session) = get_session(&state.db, &id) {
        if session.user_id.as_deref() != Some(&current_user_id) {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }

    let title = body.get("title").and_then(|v| v.as_str());
    let pinned = body.get("pinned").and_then(|v| v.as_bool());

    match update_session(&state.db, &id, title, pinned) {
        Some(session) => (StatusCode::OK, Json(serde_json::to_value(session).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    }
}

pub async fn handle_delete_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    if let Some(session) = get_session(&state.db, &id) {
        if session.user_id.as_deref() != Some(&current_user_id) {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }
    delete_session(&state.db, &id);
    (StatusCode::OK, Json(json!({"ok": true})))
}

pub async fn handle_get_session_history(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let session = match get_session(&state.db, &id) {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    };
    if session.user_id.as_deref() != Some(&current_user_id) {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    }

    let messages = session_history_with_legacy_fallback(&state, &session).await;
    (StatusCode::OK, Json(json!({"messages": messages})))
}

/// POST /api/sessions/{id}/kill
///
/// Kills the active Claude runner for this session (if any).
/// Used by subscribeToLiveStream on the frontend when the user presses Stop
/// while watching a live stream without a requestId.
pub async fn handle_kill_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    if let Some(session) = get_session(&state.db, &id) {
        if session.user_id.as_deref() != Some(&current_user_id) {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }
    state.session_pool.kill_session(&id).await;
    (StatusCode::OK, Json(json!({"ok": true})))
}

/// GET /api/sessions/{id}/stream
///
/// Multi-device / reconnect endpoint.
/// 1. Replays all persisted session_messages as NDJSON `message` events (full history).
/// 2. Emits `{"type":"streaming","active":true/false}` so the client knows whether to show a spinner.
/// 3. If the session has an active Claude turn, subscribes to the live broadcast and forwards
///    events in real-time (with 25s keep-alive pings) until the runner finishes.
pub async fn handle_session_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::from(format!("{}\n", json!({"error": "Forbidden"}))))
                .unwrap()
                .into_response();
        }
    };

    let session = match get_session(&state.db, &id) {
        Some(s) => s,
        None => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from(format!("{}\n", json!({"error": "Not found"}))))
                .unwrap()
                .into_response();
        }
    };
    if session.user_id.as_deref() != Some(&current_user_id) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from(format!("{}\n", json!({"error": "Forbidden"}))))
            .unwrap()
            .into_response();
    }

    // --- Step 1: Load history from DB ---
    let history_lines: Vec<String> = load_persisted_session_messages(&state.db, &id)
        .into_iter()
        .filter_map(|(role, message)| {
            let msg: Value = serde_json::from_str(&message).ok()?;
            let event_type = match role.as_str() {
                "assistant" => "assistant",
                "user" | "tool_result" => "tool_result",
                _ => return None,
            };
            Some(format!(
                "{}\n",
                json!({
                    "type": "message",
                    "message": {
                        "type": event_type,
                        "message": msg,
                    }
                })
            ))
        })
        .collect();

    // --- Step 2: Check continuity state ---
    let continuity_status = session.continuity_status.clone();
    let recovery_strategy = session.recovery_strategy.clone();
    let active_request_id = session.active_request_id.clone();
    let last_checkpoint_at = session.last_checkpoint_at.clone();
    let last_error = session.last_error.clone();
    let provider = session.provider.clone();
    let is_session_warm = state.session_pool.is_session_warm(&id).await;
    let is_running = matches!(
        continuity_status.as_str(),
        "queued" | "active" | "recovering"
    );
    let requires_resume = session.claude_session_id.is_some()
        && !is_session_warm
        && !is_running
        && has_visible_session_messages(&state.db, &id);
    let continuity_line = format!(
        "{}\n",
        json!({
            "type": "continuity",
            "status": continuity_status,
            "recoveryStrategy": recovery_strategy,
            "activeRequestId": active_request_id,
            "lastCheckpointAt": last_checkpoint_at,
            "lastError": last_error,
            "provider": provider,
            "isSessionWarm": is_session_warm,
            "requiresResume": requires_resume,
        })
    );
    let is_active = state.active_sessions.lock().await.contains(&id)
        || matches!(
            session.continuity_status.as_str(),
            "queued" | "active" | "recovering"
        );

    // Subscribe to live broadcast BEFORE we decide (avoid TOCTOU — runner may finish between
    // the active check and the subscribe). We drop it immediately if not active.
    let live_rx: Option<broadcast::Receiver<String>> = if is_active {
        let mut streams = state.live_streams.lock().await;
        let tx = streams.entry(id.clone()).or_insert_with(|| {
            tracing::warn!(
                "GET /stream: active session {} had no live stream; creating relay channel",
                id
            );
            let (tx, _) = broadcast::channel::<String>(2048);
            tx
        });
        Some(tx.subscribe())
    } else {
        None
    };

    // Durable questions are projected from owner-scoped storage before live
    // subscription so a refresh cannot lose or resurrect a resolved prompt.
    let pending_questions =
        crate::db::input_requests::list_pending_for_session(&state.db, &current_user_id, &id)
            .unwrap_or_default();
    let durable_ids = pending_questions
        .iter()
        .map(|request| request.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut pending_ask_lines = pending_questions
        .iter()
        .map(|request| {
            format!(
                "{}\n",
                crate::routes::realtime::pending_question_projection(request)
            )
        })
        .collect::<Vec<_>>();
    if let Some(legacy) = state.pending_asks.lock().await.get(&id) {
        pending_ask_lines.extend(
            legacy
                .iter()
                .filter(|(request_id, _)| !durable_ids.contains(request_id.as_str()))
                .map(|(_, event)| format!("{}\n", event)),
        );
    }

    let session_id = id.clone();
    let stream = async_stream::stream! {
        // Replay history
        for line in history_lines {
            yield Ok::<_, std::convert::Infallible>(line);
        }

        // Tell client whether the session is actively streaming
        yield Ok(format!("{}\n", json!({"type": "streaming", "active": is_active})));
        yield Ok(continuity_line.clone());

        // Replay any pending ask_question events so refresh mid-question restores the prompt
        for line in pending_ask_lines {
            yield Ok(line);
        }

        // If not active (or we couldn't get a receiver), we're done
        let mut live_rx = match live_rx {
            Some(rx) => rx,
            None => return,
        };

        // Forward live broadcast with 25s keep-alive pings
        let mut ping_interval = tokio::time::interval(tokio::time::Duration::from_secs(25));
        ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ping_interval.tick().await; // consume immediate first tick

        loop {
            tokio::select! {
                result = live_rx.recv() => {
                    match result {
                        Ok(line) => { yield Ok(line); }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("GET /stream subscriber (session={}) lagged by {} messages", session_id, n);
                        }
                        Err(broadcast::error::RecvError::Closed) => { break; }
                    }
                }
                _ = ping_interval.tick() => {
                    yield Ok(format!("{}\n", json!({"type": "ping"})));
                }
            }
        }
    };

    Response::builder()
        .header("content-type", "application/x-ndjson")
        .header("transfer-encoding", "chunked")
        .header("cache-control", "no-cache")
        .header("x-accel-buffering", "no")
        .header("connection", "close")
        .body(Body::from_stream(stream))
        .unwrap()
        .into_response()
}

/// POST /api/sessions/{id}/compact
///
/// Runs an explicit live compaction checkpoint for an existing session.
pub async fn handle_compact_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::from(format!("{}\n", json!({"error": "Forbidden"}))))
                .unwrap()
                .into_response();
        }
    };

    let session = match get_session(&state.db, &id) {
        Some(session) => session,
        None => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from(format!("{}\n", json!({"error": "Not found"}))))
                .unwrap()
                .into_response();
        }
    };
    if session.user_id.as_deref() != Some(&current_user_id) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from(format!("{}\n", json!({"error": "Forbidden"}))))
            .unwrap()
            .into_response();
    }

    let request: CompactSessionRequest = parse_json_body(&body);
    let queued_prompt = request.queued_prompt.unwrap_or(false);
    let recovery_strategy = request.recovery_strategy.unwrap_or_else(|| {
        if queued_prompt {
            "manual_compact_and".to_string()
        } else {
            "manual_compaction".to_string()
        }
    });

    let provider_kind = session
        .model
        .as_deref()
        .map(ProviderKind::from_model)
        .unwrap_or(ProviderKind::Codex);
    let credentials = match provider_kind {
        ProviderKind::Claude => {
            crate::routes::claude_auth::try_refresh_credentials(&state, &current_user_id)
                .await
                .map(|credentials| ProvCreds {
                    user_id: credentials.user_id,
                    access_token: credentials.access_token,
                    refresh_token: credentials.refresh_token,
                    expires_at: credentials.expires_at,
                    scopes: credentials.scopes,
                    extra: None,
                })
        }
        ProviderKind::Codex => {
            crate::routes::codex_auth::try_refresh_codex_credentials(&state, &current_user_id)
                .await
                .map(|credentials| ProvCreds {
                    user_id: credentials.user_id,
                    access_token: credentials.access_token,
                    refresh_token: credentials.refresh_token,
                    expires_at: credentials.expires_at,
                    scopes: credentials.scopes,
                    extra: credentials.id_token.map(|token| json!({"id_token": token})),
                })
        }
    };
    let user_home = std::env::var("HOME").unwrap_or_default();
    let session_id = id.clone();
    let project_path = session.project_path.clone();
    let run_id = uuid::Uuid::new_v4().to_string();
    let live_tx = {
        let streams = state.live_streams.lock().await;
        streams.get(&session_id).cloned()
    };
    let compaction_attempts = codex_compaction_attempts(
        &state.db,
        Some(&current_user_id),
        session.model.as_deref(),
        None,
    );

    let stream = async_stream::stream! {
        let emit = |line: String| -> String {
            if let Some(tx) = &live_tx {
                let _ = tx.send(line.clone());
            }
            line
        };

        let pre_tokens = bounded_context_tokens(
            get_session_input_tokens(&state.db, &session_id),
            session.model.as_deref(),
        );
        yield Ok::<_, std::convert::Infallible>(emit(compaction_phase_line(
            "manual_compaction",
            &session_id,
            &run_id,
            CompactionPhase::CollectingContext,
            json!({ "pre_tokens": pre_tokens }),
        )));

        if !has_visible_session_messages(&state.db, &session_id) {
            yield Ok(emit(compaction_failed_line(
                "manual_compaction",
                &session_id,
                &run_id,
                "No compactable session context found",
            )));
            return;
        }

        let attempt_count = compaction_attempts.len();
        let mut completed_attempt = None;
        for (attempt_index, attempt) in compaction_attempts.iter().enumerate() {
            let fallback_used = attempt_index > 0;
            let mut payload = attempt.event_payload(attempt_count > 1, fallback_used);
            if fallback_used {
                if let Some(object) = payload.as_object_mut() {
                    object.insert("summaryReset".to_string(), json!(true));
                }
            }
            yield Ok(emit(compaction_phase_line(
                "manual_compaction",
                &session_id,
                &run_id,
                CompactionPhase::SummarizingMessages,
                payload,
            )));
            yield Ok(emit(compaction_phase_line(
                "manual_compaction",
                &session_id,
                &run_id,
                CompactionPhase::SummarizingToolOutput,
                attempt.event_payload(attempt_count > 1, fallback_used),
            )));

            let mut streamed_summary_chars = 0usize;
            let (delta_tx, mut delta_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let summary_future = summarize_codex_compaction_context(
                &project_path,
                &user_home,
                credentials.as_ref(),
                &state.db,
                &session_id,
                attempt,
                move |delta| {
                    let _ = delta_tx.send(delta.to_string());
                },
            );
            tokio::pin!(summary_future);
            let summary = loop {
                tokio::select! {
                    Some(delta) = delta_rx.recv() => {
                        streamed_summary_chars += delta.chars().count();
                        yield Ok(emit(compaction_summary_delta_line(
                            "manual_compaction",
                            &session_id,
                            &run_id,
                            &delta,
                            streamed_summary_chars,
                        )));
                    }
                    result = &mut summary_future => break result,
                }
            };
            while let Ok(delta) = delta_rx.try_recv() {
                streamed_summary_chars += delta.chars().count();
                yield Ok(emit(compaction_summary_delta_line(
                    "manual_compaction",
                    &session_id,
                    &run_id,
                    &delta,
                    streamed_summary_chars,
                )));
            }
            if let Some(summary) = summary {
                completed_attempt = Some((summary, attempt.clone(), fallback_used));
                break;
            }
        }

        let (summary, completed_attempt, fallback_used) = match completed_attempt {
            Some(completed) => completed,
            None => {
                yield Ok(emit(compaction_failed_line(
                    "manual_compaction",
                    &session_id,
                    &run_id,
                    "Compaction summary failed",
                )));
                return;
            }
        };
        let summary_model = completed_attempt.model;
        let summary_reasoning_effort = completed_attempt.reasoning_effort;
        let checkpoint_payload = compaction_checkpoint_payload(
            &summary,
            pre_tokens,
            &summary_model,
            &summary_reasoning_effort,
            fallback_used,
            queued_prompt,
            Some(recovery_strategy.as_str()),
        );

        yield Ok(emit(compaction_phase_line(
            "manual_compaction",
            &session_id,
            &run_id,
            CompactionPhase::ReplacingContext,
            checkpoint_payload.clone(),
        )));
        if let Err(error) = crate::db::context::activate_checkpoint(
            &state.db,
            &session_id,
            &summary.text,
            summary.preserved_recent_messages,
            pre_tokens,
            summary.estimated_summary_tokens,
            &summary_model,
            &summary_reasoning_effort,
        ) {
            yield Ok(emit(compaction_failed_line(
                "manual_compaction",
                &session_id,
                &run_id,
                &format!("Compaction checkpoint validation failed: {error}"),
            )));
            return;
        }
        save_compaction_summary(
            &state.db,
            &session_id,
            &summary.text,
            pre_tokens,
            &summary_model,
        );
        state.session_pool.kill_session(&session_id).await;
        state.session_pool.clear_claude_session_id(&session_id).await;
        clear_session_claude_id(&state.db, &session_id);
        update_session_continuity(
            &state.db,
            &session_id,
            "idle",
            Some(recovery_strategy.as_str()),
            None,
            true,
            None,
        );

        yield Ok(emit(compaction_phase_line(
            "manual_compaction",
            &session_id,
            &run_id,
            CompactionPhase::RestoringPrompt,
            json!({ "queuedPrompt": queued_prompt }),
        )));
        let completion_payload = compaction_complete_payload(
            &run_id,
            pre_tokens,
            &summary_model,
            checkpoint_payload.clone(),
        );
        persist_compaction_checkpoint(&state.db, &session_id, &completion_payload);
        yield Ok(emit(compaction_complete_line(
            "manual_compaction",
            &session_id,
            &run_id,
            pre_tokens,
            &summary_model,
            checkpoint_payload,
        )));
    };

    Response::builder()
        .header("content-type", "application/x-ndjson")
        .header("transfer-encoding", "chunked")
        .header("cache-control", "no-cache")
        .header("x-accel-buffering", "no")
        .header("connection", "close")
        .body(Body::from_stream(stream))
        .unwrap()
        .into_response()
}

/// Truncates a session's message history at the Nth user message (0-indexed keep count),
/// deletes all session_blocks, and clears claude_session_id so the next turn starts fresh.
/// Used by the edit + re-run feature.
pub async fn handle_truncate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(u) => u,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let session = match get_session(&state.db, &id) {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    };
    if session.user_id.as_deref() != Some(&user_id) {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    }

    // Number of user messages to KEEP (everything before the edited one)
    let keep_user_count = body
        .get("keepUserMessageCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    // Perform all DB work in a scoped block so conn is dropped before the async await below.
    let history_changed = match (|| -> rusqlite::Result<bool> {
        let mut conn = state.db.conn();
        let tx = conn.transaction()?;

        // Find the rowid of the (keep_user_count)-th user message (0-indexed) — delete from there onward
        let cutoff_id: Option<i64> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM session_messages WHERE session_id = ?1 AND role = 'user' ORDER BY id ASC LIMIT 1 OFFSET ?2"
            )?;
            stmt.query_row(rusqlite::params![&id, keep_user_count as i64], |row| {
                row.get::<_, i64>(0)
            })
            .ok()
        };

        let history_changed = if let Some(cutoff) = cutoff_id {
            tx.execute(
                "UPDATE context_events SET active=0
                 WHERE session_id=?1 AND source_kind='session_message'
                   AND CAST(source_id AS INTEGER) >= ?2",
                rusqlite::params![&id, cutoff],
            )?;
            let deleted = tx.execute(
                "DELETE FROM session_messages WHERE session_id = ?1 AND id >= ?2",
                rusqlite::params![&id, cutoff],
            )?;
            deleted > 0
        } else {
            false
        };

        if history_changed {
            crate::db::sessions::bump_session_history_revision_with_conn(&tx, &id)?;
        }

        // Delete ALL session_blocks — frontend will rebuild from the kept messages
        tx.execute("DELETE FROM session_blocks WHERE session_id = ?1", [&id])?;

        // Clear claude_session_id so next turn spawns a fresh Claude (no --resume)
        tx.execute(
            "UPDATE sessions SET claude_session_id = NULL WHERE id = ?1",
            [&id],
        )?;
        tx.commit()?;
        Ok(history_changed)
    })() {
        Ok(changed) => changed,
        Err(error) => {
            tracing::error!(%error, session_id = id, "failed to truncate session history");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Could not edit this session"})),
            );
        }
    }; // conn dropped here

    // Kill the warm session if it's still in the pool (async, safe now that conn is dropped)
    state.session_pool.kill_session(&id).await;

    (
        StatusCode::OK,
        Json(json!({"ok": true, "historyChanged": history_changed})),
    )
}

/// Context window utilization stats for a session — used by the MCP tool and frontend bar.
pub async fn handle_context_stats(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(u) => u,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let session = match get_session(&state.db, &id) {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    };
    if session.user_id.as_deref() != Some(&user_id) {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    }

    let context_window = crate::compaction::model_context_window(session.model.as_deref());
    let auto_compact_threshold =
        crate::compaction::auto_compaction_threshold(session.model.as_deref());
    let input_tokens = bounded_context_tokens(session.input_tokens, session.model.as_deref());
    let output_tokens = session.output_tokens;
    let fill_pct = (input_tokens as f64 / context_window as f64 * 100.0 * 10.0).round() / 10.0;
    let remaining = context_window - input_tokens;
    let turns_remaining = if output_tokens > 0 {
        Some((remaining / output_tokens.max(1)).min(999))
    } else {
        None
    };

    let diagnostics = crate::db::context::diagnostics(&state.db, &id);
    (
        StatusCode::OK,
        Json(json!({
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "context_window": context_window,
            "fill_pct": fill_pct,
            "remaining_tokens": remaining,
            "auto_compact_threshold": auto_compact_threshold,
            "auto_compact_ratio": crate::compaction::AUTO_COMPACTION_FILL_RATIO,
            "compacted_context_target": crate::compaction::compacted_context_target(session.model.as_deref()),
            "turns_remaining": turns_remaining,
            "ledger": diagnostics,
        })),
    )
}

/// Owner-only search over the immutable context ledger. Archived revisions are
/// opt-in so normal retrieval never reintroduces obsolete streamed snapshots.
pub async fn handle_context_search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<ContextSearchQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(user_id) => user_id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let session = match get_session(&state.db, &id) {
        Some(session) => session,
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    };
    if session.user_id.as_deref() != Some(&user_id) {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    }
    let q = query.q.unwrap_or_default();
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let include_archived = query.include_archived.unwrap_or(false);
    let results =
        crate::db::context::retrieve_with_archive(&state.db, &id, &q, limit, include_archived);
    (
        StatusCode::OK,
        Json(json!({
            "query": q,
            "includeArchived": include_archived,
            "results": results,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_jsonl_parser_keeps_visible_turns_and_drops_tool_result_echoes() {
        let content = concat!(
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]},\"uuid\":\"u1\"}\n",
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"content\":\"done\"}]},\"uuid\":\"tool\"}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]},\"uuid\":\"a1\"}\n",
        );

        let messages = parse_legacy_jsonl_history(content);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["type"], "user");
        assert_eq!(messages[0]["uuid"], "u1");
        assert_eq!(messages[1]["type"], "assistant");
        assert_eq!(messages[1]["uuid"], "a1");
    }
}

#[cfg(test)]
mod local_history_regression {
    use super::*;
    #[tokio::test]
    async fn sqlite_history_survives_without_legacy_jsonl_or_service_cache() {
        let state = AppState::new(":memory:", 0);
        state.db.conn().execute("INSERT INTO sessions(id,project_path,created_at,updated_at,user_id,provider) VALUES('saved','/missing-workspace','now','now','local','codex')",[]).unwrap();
        crate::db::sessions::persist_session_message(
            &state.db,
            "saved",
            "user",
            r#"{"role":"user","content":"persisted work"}"#,
        );
        let session = crate::db::sessions::get_session(&state.db, "saved").unwrap();
        let history = session_history_with_legacy_fallback(&state, &session).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["message"]["content"], "persisted work");
    }
}
