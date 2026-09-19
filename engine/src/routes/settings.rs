use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;
use crate::compaction::{
    codex_compaction_settings_for_user, save_codex_compaction_settings_for_user,
};
use crate::db::settings::{get_user_settings, update_user_settings};
use crate::routes::admin::get_current_user_id;

fn compaction_settings_json(settings: crate::compaction::CodexCompactionSettings) -> Value {
    json!({
        "codex_compaction_model": settings.model,
        "codex_compaction_reasoning_effort": settings.reasoning_effort,
        "codex_compaction_fallback_enabled": settings.fallback_enabled,
    })
}

pub async fn handle_get_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
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
    let settings = get_user_settings(&state.db, &user_id);
    (
        StatusCode::OK,
        Json(serde_json::to_value(settings).unwrap()),
    )
}

pub async fn handle_update_settings(
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

    let git_name = body.get("git_name").and_then(|v| v.as_str());
    let git_email = body.get("git_email").and_then(|v| v.as_str());
    let git_signing_key = body.get("git_signing_key").and_then(|v| v.as_str());
    let preferred_provider = body.get("preferred_provider").and_then(|v| v.as_str());
    let cross_provider_fallback = body
        .get("cross_provider_fallback")
        .and_then(|v| v.as_bool());
    let codex_planning_reasoning_effort = body
        .get("codex_planning_reasoning_effort")
        .and_then(|v| v.as_str());
    let claude_planning_reasoning_effort = body
        .get("claude_planning_reasoning_effort")
        .and_then(|v| v.as_str());
    if [
        codex_planning_reasoning_effort,
        claude_planning_reasoning_effort,
    ]
    .into_iter()
    .flatten()
    .any(|effort| !matches!(effort, "high" | "xhigh" | "max" | "ultra"))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Planning reasoning must be high, xhigh, max, or ultra"})),
        );
    }

    let settings = update_user_settings(
        &state.db,
        &user_id,
        git_name,
        git_email,
        git_signing_key,
        preferred_provider,
        cross_provider_fallback,
        codex_planning_reasoning_effort,
        claude_planning_reasoning_effort,
    );
    (
        StatusCode::OK,
        Json(serde_json::to_value(settings).unwrap()),
    )
}

pub async fn handle_get_compaction_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
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
    let settings = codex_compaction_settings_for_user(&state.db, Some(&user_id));
    (StatusCode::OK, Json(compaction_settings_json(settings)))
}

pub async fn handle_update_compaction_settings(
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
    let current = codex_compaction_settings_for_user(&state.db, Some(&user_id));
    let model = if body.get("codex_compaction_model").is_some() {
        body.get("codex_compaction_model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        current.model
    };
    let reasoning_effort = if body.get("codex_compaction_reasoning_effort").is_some() {
        body.get("codex_compaction_reasoning_effort")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        current.reasoning_effort
    };
    let fallback_enabled = body
        .get("codex_compaction_fallback_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(current.fallback_enabled);
    let settings = save_codex_compaction_settings_for_user(
        &state.db,
        &user_id,
        Some(&model),
        Some(&reasoning_effort),
        fallback_enabled,
    );
    (StatusCode::OK, Json(compaction_settings_json(settings)))
}

pub async fn handle_local_secret(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if !state.local_mode {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Local secrets are available only on Flamel Desktop"})),
        );
    }
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Unauthorized"})),
        );
    };
    let Some(name) = body.get("name").and_then(Value::as_str) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Secret name is required"})),
        );
    };
    let action = body.get("action").and_then(Value::as_str).unwrap_or("");
    let session_id = headers
        .get("x-flamel-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let result = match action {
        "store" => {
            let Some(value) = body.get("value").and_then(Value::as_str) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "Secret value is required"})),
                );
            };
            if value.is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "Secret value is required"})),
                );
            }
            crate::db::settings::set_local_secret(
                &state.db,
                &state.credential_vault,
                &user_id,
                name,
                value,
            )
            .and_then(|_| {
                if let Some(session_id) = session_id.as_deref() {
                    crate::db::sessions::redact_session_message_secret(
                        &state.db,
                        session_id,
                        &user_id,
                        value,
                        &format!("[stored securely as local secret: {name}]"),
                    )?;
                }
                Ok(json!({"ok": true, "name": name}))
            })
        }
        "load" => crate::db::settings::get_local_secret(
            &state.db,
            &state.credential_vault,
            &user_id,
            name,
        )
        .and_then(|value| value.ok_or_else(|| "Local secret not found".to_string()))
        .map(|value| json!({"ok": true, "value": value})),
        "status" => crate::db::settings::get_local_secret(
            &state.db,
            &state.credential_vault,
            &user_id,
            name,
        )
        .map(|value| json!({"ok": true, "stored": value.is_some(), "name": name})),
        "delete" => crate::db::settings::delete_local_secret(&state.db, &user_id, name)
            .map(|deleted| json!({"ok": true, "deleted": deleted, "name": name})),
        _ => Err("Unsupported local secret action".into()),
    };

    match result {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(error) if error == "Local secret not found" => {
            (StatusCode::NOT_FOUND, Json(json!({"error": error})))
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error": error}))),
    }
}
