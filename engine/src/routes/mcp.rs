use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;
use crate::db::{mcp, sessions};
use crate::mcp_lifecycle::{
    BUILTIN_MCP_SERVER_NAMES, McpSessionPhase, phase_from_session, runtime_status_for_config,
};
use crate::routes::admin::get_current_user_id;

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(rename = "projectPath")]
    project_path: Option<String>,
}

#[derive(Deserialize)]
pub struct ReconnectBody {
    #[serde(rename = "serverName")]
    server_name: Option<String>,
}

pub async fn handle_session_mcp_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    let session = match sessions::get_session(&state.db, &session_id) {
        Some(session) => session,
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    };
    if let Some(owner) = session.user_id.as_deref() {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }

    let active = state.active_sessions.lock().await.contains(&session_id);
    let phase = phase_from_session(
        &session.continuity_status,
        active,
        session.last_error.as_deref(),
    );
    let mut servers = Vec::new();

    for name in BUILTIN_MCP_SERVER_NAMES {
        let config = builtin_mcp_config(name);
        servers.push(runtime_status_for_config(
            name, &config, true, true, "built-in", phase,
        ));
    }

    for config in mcp::list_mcp_configs(&state.db, Some(&session.project_path), Some(&user_id)) {
        if BUILTIN_MCP_SERVER_NAMES.contains(&config.name.as_str()) {
            continue;
        }
        let scope = if config.project_path.as_deref() == Some(session.project_path.as_str()) {
            "project"
        } else {
            "user"
        };
        let value = json!({"command": config.command, "args": config.args});
        servers.push(runtime_status_for_config(
            &config.name,
            &value,
            false,
            config.enabled,
            scope,
            phase,
        ));
    }

    (
        StatusCode::OK,
        Json(json!({
            "sessionId": session_id,
            "live": active,
            "phase": phase_label(phase),
            "continuityStatus": session.continuity_status,
            "lastError": session.last_error,
            "servers": servers,
        })),
    )
}

pub async fn handle_session_mcp_reconnect(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(body): Json<ReconnectBody>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    let session = match sessions::get_session(&state.db, &session_id) {
        Some(session) => session,
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    };
    if let Some(owner) = session.user_id.as_deref() {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }

    state.session_pool.kill_session(&session_id).await;
    (
        StatusCode::ACCEPTED,
        Json(json!({
            "ok": true,
            "sessionId": session_id,
            "serverName": body.server_name,
            "status": "pending",
            "message": "Warm provider process was stopped; MCP servers will start again on the next assistant turn.",
        })),
    )
}

fn builtin_mcp_config(name: &str) -> Value {
    match name {
        "flamel" => json!({
            "command": crate::bundled_binary_path("flamel_mcp").to_string_lossy(),
            "args": [],
            "tool_timeout_sec": 900,
            "startup_timeout_sec": 30,
        }),
        _ => json!({"command": "", "args": []}),
    }
}

fn phase_label(phase: McpSessionPhase) -> &'static str {
    match phase {
        McpSessionPhase::NoSession => "no_session",
        McpSessionPhase::Queued => "queued",
        McpSessionPhase::Running => "running",
        McpSessionPhase::Idle => "idle",
        McpSessionPhase::Failed => "failed",
    }
}

pub async fn handle_list_mcp(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Json<Value> {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return Json(json!([])),
    };
    Json(
        serde_json::to_value(mcp::list_mcp_configs(
            &state.db,
            query.project_path.as_deref(),
            Some(&user_id),
        ))
        .unwrap(),
    )
}

pub async fn handle_create_mcp(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let command = body.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args: Vec<String> = body
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let project_path = body.get("projectPath").and_then(|v| v.as_str());
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let config = mcp::create_mcp_config(
        &state.db,
        name,
        command,
        &args,
        project_path,
        enabled,
        &user_id,
    );
    (StatusCode::OK, Json(serde_json::to_value(config).unwrap()))
}

pub async fn handle_update_mcp(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    match mcp::get_mcp_config_user_id(&state.db, &id) {
        Some(owner) if owner != user_id => {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
        _ => {}
    }
    match mcp::update_mcp_config(&state.db, &id, &body) {
        Some(config) => (StatusCode::OK, Json(serde_json::to_value(config).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    }
}

pub async fn handle_delete_mcp(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    match mcp::get_mcp_config_user_id(&state.db, &id) {
        Some(owner) if owner != user_id => {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
        _ => {}
    }
    mcp::delete_mcp_config(&state.db, &id);
    (StatusCode::OK, Json(json!({"ok": true})))
}
