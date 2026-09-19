use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;

use std::path::PathBuf;

use crate::AppState;

/// Extract the current flamel.dev user ID from request headers.
/// The auth middleware validates the JWT and injects this header — it cannot be spoofed.
pub fn get_current_user_id(_state: &AppState, headers: &HeaderMap) -> Option<String> {
    if let Some(flamel_id) = headers
        .get("x-flamel-user-id")
        .and_then(|v| v.to_str().ok())
    {
        if !flamel_id.is_empty() {
            return Some(flamel_id.to_string());
        }
    }
    tracing::warn!("get_current_user_id: no x-flamel-user-id header — request will be rejected");
    None
}

/// Returns the per-user home directory: `{home_dir}/users/{user_id}/`.
/// Auto-creates the directory if it doesn't exist.

pub async fn handle_get_me(
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
    let user_home = get_user_home(&state, Some(&user_id));
    let user_home_str = user_home.to_string_lossy();
    // Look up in flamel_dev_users
    let flamel_users = crate::identity::profiles(&state.db).unwrap_or_default();
    if let Some(fu) = flamel_users.iter().find(|u| u.id == user_id) {
        return (
            StatusCode::OK,
            Json(json!({
                "id": fu.id,
                "username": fu.label,
                "isAdmin": false,
                "userHome": user_home_str,
                "local": state.local_mode,
            })),
        );
    }
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": "User not found"})),
    )
}

// ── Admin guard ─────────────────────────────────────────────────────────────

pub(crate) fn require_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<Value>)> {
    let user_id = get_current_user_id(state, headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Unauthorized"})),
        )
    })?;
    if !crate::identity::is_bound(&state.db, &user_id) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Local profile required"})),
        ));
    }
    Ok(user_id)
}

// ── GET /api/admin/users ────────────────────────────────────────────────────

// ── PUT /api/admin/users/:id ────────────────────────────────────────────────

// ── DELETE /api/admin/users/:id ──────────────────────────────────────���──────

// ── GET /api/admin/stats ────────────────────────────────────────────────────

pub fn get_user_home(state: &AppState, _user_id: Option<&str>) -> PathBuf {
    state.home_dir.clone()
}
