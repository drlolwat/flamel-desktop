use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::routes::admin::get_current_user_id;
use crate::{AppState, db::work};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkQuery {
    project_path: String,
    query: Option<String>,
}

fn user(state: &AppState, headers: &HeaderMap) -> Result<String, (StatusCode, Json<Value>)> {
    get_current_user_id(state, headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Unauthorized"})),
        )
    })
}
fn result(value: Result<Value, String>, success: StatusCode) -> (StatusCode, Json<Value>) {
    match value {
        Ok(value) => (success, Json(value)),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<WorkQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    (
        StatusCode::OK,
        Json(
            json!({"items":work::list(&state.db,&user_id,&query.project_path,query.query.as_deref())}),
        ),
    )
}
pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    result(
        work::create(&state.db, &user_id, &body).map(|item| json!(item)),
        StatusCode::CREATED,
    )
}
pub async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match work::detail(&state.db, &user_id, &id) {
        Some(value) => (StatusCode::OK, Json(value)),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Work item not found"})),
        ),
    }
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
    result(
        work::update(&state.db, &user_id, &id, &body).map(|item| json!(item)),
        StatusCode::OK,
    )
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
    if work::delete(&state.db, &user_id, &id) {
        (StatusCode::OK, Json(json!({"ok":true})))
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Work item not found"})),
        )
    }
}
pub async fn comment(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    result(
        work::add_comment(
            &state.db,
            &user_id,
            &id,
            body.get("body").and_then(Value::as_str).unwrap_or(""),
        ),
        StatusCode::CREATED,
    )
}
pub async fn dependency(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    result(
        work::add_dependency(
            &state.db,
            &user_id,
            &id,
            body.get("dependsOnId")
                .and_then(Value::as_str)
                .unwrap_or(""),
        )
        .map(|_| json!({"ok":true})),
        StatusCode::CREATED,
    )
}
pub async fn artifact(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    result(
        work::add_artifact(&state.db, &user_id, &id, &body),
        StatusCode::CREATED,
    )
}
