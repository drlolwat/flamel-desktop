use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;
use crate::db::prompts;
use crate::routes::admin::get_current_user_id;

pub async fn handle_list_prompts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Value> {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return Json(json!([])),
    };
    Json(serde_json::to_value(prompts::list_prompts(&state.db, Some(&user_id))).unwrap())
}

pub async fn handle_create_prompt(
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
    let title = body.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let content = body.get("content").and_then(|v| v.as_str()).unwrap_or("");
    if title.is_empty() || content.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "title and content required"})),
        );
    }
    let prompt = prompts::create_prompt(&state.db, title, content, &user_id);
    (StatusCode::OK, Json(serde_json::to_value(prompt).unwrap()))
}

pub async fn handle_delete_prompt(
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
    if let Some(owner) = prompts::get_prompt_user_id(&state.db, &id) {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    } else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"})));
    }
    prompts::delete_prompt(&state.db, &id);
    (StatusCode::OK, Json(json!({"ok": true})))
}

pub async fn handle_use_prompt(
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
    if let Some(owner) = prompts::get_prompt_user_id(&state.db, &id) {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    } else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"})));
    }
    prompts::increment_prompt_use(&state.db, &id);
    (StatusCode::OK, Json(json!({"ok": true})))
}
