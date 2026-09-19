use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;
use crate::db::memory_store::{self, MemoryDb};
use crate::routes::admin::get_current_user_id;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryQuery {
    project_path: Option<String>,
    limit: Option<i64>,
}

fn memory_db() -> MemoryDb {
    let path = std::env::var("FLAMEL_MEMORIES_DB").unwrap_or_else(|_| {
        dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("@flamel")
            .join("memories.db")
            .to_string_lossy()
            .to_string()
    });
    MemoryDb::open(&path)
}

fn user(state: &AppState, headers: &HeaderMap) -> Result<String, (StatusCode, Json<Value>)> {
    get_current_user_id(state, headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Unauthorized"})),
        )
    })
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<MemoryQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let db = memory_db();
    let memories = memory_store::list_memories(
        &db,
        &user_id,
        query.project_path.as_deref(),
        query.limit.unwrap_or(200).clamp(1, 500),
    );
    (StatusCode::OK, Json(json!({"memories":memories})))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let db = memory_db();
    let result = if let Some(pinned) = body.get("pinned").and_then(Value::as_bool) {
        memory_store::pin_memory(&db, &id, &user_id, pinned)
    } else if let Some(replacement) = body.get("supersededBy").and_then(Value::as_str) {
        memory_store::supersede_memory(&db, &id, &user_id, replacement)
    } else {
        Err("Set pinned or supersededBy".into())
    };
    match result {
        Ok(()) => (StatusCode::OK, Json(json!({"ok":true}))),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match memory_store::delete_memory(&memory_db(), &id, &user_id) {
        Ok(()) => (StatusCode::OK, Json(json!({"ok":true}))),
        Err(error) => (StatusCode::NOT_FOUND, Json(json!({"error":error}))),
    }
}
