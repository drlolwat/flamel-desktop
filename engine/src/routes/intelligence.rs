use crate::routes::admin::{get_current_user_id, get_user_home};
use crate::{AppState, db::intelligence};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;

fn authorize(state: &AppState, headers: &HeaderMap) -> Result<String, (StatusCode, Json<Value>)> {
    get_current_user_id(state, headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Unauthorized"})),
        )
    })
}

fn project_path(
    state: &AppState,
    user_id: &str,
    requested: &str,
) -> Result<String, (StatusCode, Json<Value>)> {
    let user_home = get_user_home(state, Some(user_id));
    let root = super::path_guard::access_root(&user_home, state.local_mode);
    super::path_guard::guard_existing(&root, requested)
        .map(|path| path.to_string_lossy().to_string())
}

pub async fn index(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match authorize(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let requested = body
        .get("projectPath")
        .and_then(Value::as_str)
        .unwrap_or("");
    let path = match project_path(&state, &user_id, requested) {
        Ok(path) => path,
        Err(response) => return response,
    };
    let db = Arc::clone(&state.db);
    match tokio::task::spawn_blocking(move || intelligence::index_project(&db, &path)).await {
        Ok(Ok(summary)) => (StatusCode::OK, Json(json!(summary))),
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error.to_string()})),
        ),
    }
}
pub async fn context(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match authorize(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let requested = body
        .get("projectPath")
        .and_then(Value::as_str)
        .unwrap_or("");
    let path = match project_path(&state, &user_id, requested) {
        Ok(path) => path,
        Err(response) => return response,
    };
    let query = body.get("query").and_then(Value::as_str).unwrap_or("");
    let budget = body
        .get("maxTokens")
        .and_then(Value::as_u64)
        .unwrap_or(4000) as usize;
    (
        StatusCode::OK,
        Json(intelligence::context_bundle(
            &state.db, &path, query, budget,
        )),
    )
}
pub async fn verification(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match authorize(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let requested = body
        .get("projectPath")
        .and_then(Value::as_str)
        .unwrap_or("");
    let path = match project_path(&state, &user_id, requested) {
        Ok(path) => path,
        Err(response) => return response,
    };
    (
        StatusCode::OK,
        Json(intelligence::verification_graph(&path)),
    )
}
pub async fn run_verification(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match authorize(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let requested = body
        .get("projectPath")
        .and_then(Value::as_str)
        .unwrap_or("");
    let path = match project_path(&state, &user_id, requested) {
        Ok(path) => path,
        Err(response) => return response,
    };
    let checks = body
        .get("checks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    match tokio::task::spawn_blocking(move || intelligence::run_verification(&path, &checks)).await
    {
        Ok(Ok(result)) => (StatusCode::OK, Json(result)),
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error.to_string()})),
        ),
    }
}
