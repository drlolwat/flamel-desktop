use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::AppState;
use crate::db::agents;
use crate::routes::admin::get_current_user_id;

#[derive(Deserialize)]
pub struct RunsQuery {
    #[serde(rename = "configId")]
    config_id: Option<String>,
}

// --- Agent configs ---

pub async fn handle_list_agent_configs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Value> {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return Json(json!([])),
    };
    Json(serde_json::to_value(agents::list_agent_configs(&state.db, Some(&user_id))).unwrap())
}

pub async fn handle_create_agent_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(uid) => uid,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    let git_account_id = match body.get("gitAccountId").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "gitAccountId required"})),
            );
        }
    };
    let repo_owner = match body.get("repoOwner").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "repoOwner required"})),
            );
        }
    };
    let repo_name = match body.get("repoName").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "repoName required"})),
            );
        }
    };
    let project_path = match body.get("projectPath").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "projectPath required"})),
            );
        }
    };
    let trigger_event = body
        .get("triggerEvent")
        .and_then(|v| v.as_str())
        .unwrap_or("issues");
    let trigger_label = body.get("triggerLabel").and_then(|v| v.as_str());
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("claude-opus-4-7");
    let permission_mode = body
        .get("permissionMode")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let auto_pr = body.get("autoPr").and_then(|v| v.as_bool()).unwrap_or(true);
    let auto_close = body
        .get("autoClose")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let comment_on_start = body
        .get("commentOnStart")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let require_approval = body
        .get("requireApproval")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let config = agents::create_agent_config(
        &state.db,
        &user_id,
        git_account_id,
        repo_owner,
        repo_name,
        trigger_event,
        trigger_label,
        project_path,
        model,
        permission_mode,
        auto_pr,
        auto_close,
        comment_on_start,
        require_approval,
    );
    (
        StatusCode::CREATED,
        Json(serde_json::to_value(config).unwrap()),
    )
}

pub async fn handle_update_agent_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(uid) => uid,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    if agents::get_agent_config_owned(&state.db, &id, &user_id).is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"})));
    }
    match agents::update_agent_config(&state.db, &id, &body) {
        Some(config) => (StatusCode::OK, Json(serde_json::to_value(config).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    }
}

pub async fn handle_delete_agent_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(uid) => uid,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    if agents::delete_agent_config_owned(&state.db, &id, &user_id) {
        (StatusCode::OK, Json(json!({"ok": true})))
    } else {
        (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"})))
    }
}

// --- Agent runs ---

pub async fn handle_list_agent_runs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<RunsQuery>,
) -> Json<Value> {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return Json(json!([])),
    };
    let runs = agents::list_agent_runs(&state.db, q.config_id.as_deref(), Some(&user_id));
    Json(serde_json::to_value(runs).unwrap())
}

pub async fn handle_approve_agent_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(uid) => uid,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    let Some(run) = agents::get_agent_run_owned(&state.db, &id, &user_id) else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"})));
    };
    if run.status != "awaiting_approval" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Run is not awaiting approval"})),
        );
    }
    agents::update_agent_run_status(&state.db, &id, "pending", None);
    (StatusCode::OK, Json(json!({"ok": true})))
}

pub async fn handle_reject_agent_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(uid) => uid,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    if agents::get_agent_run_owned(&state.db, &id, &user_id).is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"})));
    }
    agents::update_agent_run_status(&state.db, &id, "failed", Some("Rejected by user"));
    (StatusCode::OK, Json(json!({"ok": true})))
}

pub async fn handle_kill_agent_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(uid) => uid,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    let Some(run) = agents::get_agent_run_owned(&state.db, &id, &user_id) else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"})));
    };
    agents::update_agent_run_status(&state.db, &id, "failed", Some("Killed by user"));

    // SIGINT the running subprocess (graceful stop)
    let killed = state.agent_pool.kill_run(&id).await;

    // Also kill any session pool entry if one was associated
    if !killed {
        if let Some(session_id) = run.session_id {
            let pool = Arc::clone(&state.session_pool);
            tokio::spawn(async move {
                pool.kill_session(&session_id).await;
            });
        }
    }

    state.agent_pool.emit(
        json!({"type": "agent_status", "runId": id, "status": "failed", "error": "Killed by user"}),
    );
    (StatusCode::OK, Json(json!({"ok": true})))
}

/// SSE stream — broadcasts agent events (status changes + live output) to subscribed clients.
/// Auth-checked: only returns runs owned by the authenticated user.
pub async fn handle_agent_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(uid) => uid,
        None => {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .unwrap();
        }
    };

    let mut rx = state.agent_pool.subscribe();

    // Collect all run IDs owned by this user for filtering incoming events
    let owned_run_ids: std::collections::HashSet<String> =
        agents::list_agent_runs(&state.db, None, Some(&user_id))
            .into_iter()
            .map(|r| r.id)
            .collect();

    let initial = {
        let runs = agents::list_agent_runs(&state.db, None, Some(&user_id));
        format!("{}\n", serde_json::json!({"type": "initial", "runs": runs}))
    };

    let stream = async_stream::stream! {
        yield Ok::<_, std::convert::Infallible>(initial);

        let mut ping = tokio::time::interval(tokio::time::Duration::from_secs(25));
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ping.tick().await; // consume immediate first tick

        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(line) => {
                            // Filter: only forward events for this user's runs
                            // Quick check — if runId in the JSON matches an owned ID
                            let forward = if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                                v.get("runId")
                                    .and_then(|id| id.as_str())
                                    .map(|id| owned_run_ids.contains(id))
                                    .unwrap_or(false)
                            } else {
                                false
                            };
                            if forward {
                                yield Ok(line);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("agent SSE subscriber lagged by {} messages", n);
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = ping.tick() => {
                    yield Ok(format!("{}\n", serde_json::json!({"type": "ping"})));
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
}
