use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;

pub async fn handle_permission(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let approved = body
        .get("approved")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut resolvers = state.permission_resolvers.lock().unwrap();
    if let Some(sender) = resolvers.remove(&request_id) {
        let _ = sender.send(approved);
        (StatusCode::OK, Json(json!({"ok": true})))
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "No pending permission request for this requestId"})),
        )
    }
}
