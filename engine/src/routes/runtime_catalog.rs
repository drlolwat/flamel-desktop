use crate::{
    AppState, catalog_trust,
    db::runtime_catalog,
    routes::admin::{get_current_user_id, require_admin},
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCatalogControl {
    action: String,
    channel: Option<String>,
}

pub async fn handle_runtime_catalog_personas(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if get_current_user_id(&state, &headers).is_none() {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    }
    match runtime_catalog::active_runtime_personas(&state.db) {
        Ok(personas) => (StatusCode::OK, Json(json!(personas))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        ),
    }
}

pub async fn handle_runtime_catalog_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = require_admin(&state, &headers) {
        return response;
    }
    (
        StatusCode::OK,
        Json(json!(runtime_catalog::runtime_catalog_status(&state.db))),
    )
}

pub async fn handle_runtime_catalog_control(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RuntimeCatalogControl>,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = require_admin(&state, &headers) {
        return response;
    }
    let result = match body.action.as_str() {
        "freeze" => Ok(runtime_catalog::set_runtime_catalog_frozen(&state.db, true)),
        "unfreeze" => Ok(runtime_catalog::set_runtime_catalog_frozen(
            &state.db, false,
        )),
        "channel" => runtime_catalog::set_runtime_catalog_channel(
            &state.db,
            body.channel.as_deref().unwrap_or(""),
        ),
        "rollback" => runtime_catalog::rollback_runtime_catalog(&state.db),
        "refresh" => {
            let _guard = state.runtime_catalog_sync.lock().await;
            let status = runtime_catalog::runtime_catalog_status(&state.db);
            if status.frozen {
                Ok(status)
            } else {
                match catalog_trust::get_jwks(&state.jwks_cache).await {
                    Ok(jwks) => runtime_catalog::sync_runtime_catalog(&state.db, &jwks).await,
                    Err(error) => Err(error),
                }
            }
        }
        _ => Err("Runtime catalog action is not supported".to_string()),
    };
    match result {
        Ok(status) => (StatusCode::OK, Json(json!(status))),
        Err(error) => {
            runtime_catalog::record_runtime_catalog_error(&state.db, &error);
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error":error,
                    "status":runtime_catalog::runtime_catalog_status(&state.db),
                })),
            )
        }
    }
}
