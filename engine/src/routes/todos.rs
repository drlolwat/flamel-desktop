use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;
use crate::db::todos;
use crate::routes::admin::get_current_user_id;

#[derive(Deserialize)]
pub struct ProjectQuery {
    #[serde(rename = "projectPath")]
    project_path: Option<String>,
}

pub async fn handle_list_todos(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ProjectQuery>,
) -> (StatusCode, Json<Value>) {
    let Some(project_path) = q.project_path else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "projectPath required"})),
        );
    };
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::OK,
                Json(serde_json::to_value(Vec::<()>::new()).unwrap()),
            );
        }
    };
    let items = todos::list_todos(&state.db, &project_path, Some(&user_id));
    (StatusCode::OK, Json(serde_json::to_value(items).unwrap()))
}

pub async fn handle_create_todo(
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
    let text = match body.get("text").and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "text required"})),
            );
        }
    };
    let project_path = match body.get("projectPath").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() => p,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "projectPath required"})),
            );
        }
    };
    let item = todos::create_todo(&state.db, project_path, text, &user_id);
    (
        StatusCode::CREATED,
        Json(serde_json::to_value(item).unwrap()),
    )
}

pub async fn handle_update_todo(
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
    if let Some(owner) = todos::get_todo_user_id(&state.db, &id) {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    } else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"})));
    }
    let done = body.get("done").and_then(|v| v.as_bool());
    let text = body.get("text").and_then(|v| v.as_str());
    match todos::update_todo(&state.db, &id, done, text) {
        Some(item) => (StatusCode::OK, Json(serde_json::to_value(item).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    }
}

pub async fn handle_delete_todo(
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
    if let Some(owner) = todos::get_todo_user_id(&state.db, &id) {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    } else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"})));
    }
    todos::delete_todo(&state.db, &id);
    (StatusCode::OK, Json(json!({"ok": true})))
}

pub async fn handle_clear_done_todos(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ProjectQuery>,
) -> (StatusCode, Json<Value>) {
    let Some(project_path) = q.project_path else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "projectPath required"})),
        );
    };
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    todos::clear_done_todos(&state.db, &project_path, Some(&user_id));
    (StatusCode::OK, Json(json!({"ok": true})))
}
