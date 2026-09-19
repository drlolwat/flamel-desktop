use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;
use crate::routes::admin::{get_current_user_id, get_user_home};

#[derive(Deserialize)]
pub struct DirQuery {
    path: Option<String>,
}

pub async fn handle_list_dir(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<DirQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let user_home = get_user_home(&state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);
    let home_str = user_home.to_string_lossy().to_string();
    let raw = q.path.unwrap_or_else(|| home_str.clone());

    // Expand ~ prefix to home_dir
    let expanded = if raw == "~" {
        home_str.clone()
    } else if raw.starts_with("~/") {
        format!("{}/{}", home_str, &raw[2..])
    } else {
        raw
    };

    let safe_path = match super::path_guard::guard_existing(&access_root, &expanded) {
        Ok(p) => p,
        Err(e) => return e,
    };

    if !safe_path.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Not a directory"})),
        );
    }

    let canonical_home = std::fs::canonicalize(&user_home).unwrap_or_else(|_| user_home.clone());
    let canonical_root = if super::path_guard::is_unrestricted_root(&access_root) {
        None
    } else {
        Some(std::fs::canonicalize(&access_root).unwrap_or_else(|_| access_root.clone()))
    };
    let parent = if let Some(root) = &canonical_root {
        if safe_path.as_path() == root.as_path() {
            None
        } else {
            safe_path
                .parent()
                .filter(|p| p.starts_with(root))
                .map(|p| p.to_string_lossy().to_string())
        }
    } else {
        safe_path.parent().map(|p| p.to_string_lossy().to_string())
    };

    let read_dir = match std::fs::read_dir(&safe_path) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            );
        }
    };

    let mut dirs: Vec<Value> = Vec::new();
    let mut files: Vec<Value> = Vec::new();

    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip hidden
        if name.starts_with('.') {
            continue;
        }
        let full = entry.path();
        let is_dir = full.is_dir();
        let is_git = is_dir && full.join(".git").exists();
        let path_str = full.to_string_lossy().to_string();

        // Get last modified time
        let modified = full
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let entry_val = json!({
            "name": name,
            "path": path_str,
            "type": if is_dir { "dir" } else { "file" },
            "is_git": is_git,
            "modified": modified,
        });

        if is_dir {
            dirs.push(entry_val);
        } else {
            files.push(entry_val);
        }
    }

    // Sort dirs and files alphabetically
    dirs.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
    });
    files.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
    });

    // Dirs first, then files
    let mut entries = dirs;
    entries.extend(files);

    (
        StatusCode::OK,
        Json(json!({
            "path": safe_path.to_string_lossy(),
            "home": canonical_home.to_string_lossy(),
            "root": canonical_root.as_ref().map(|p| p.to_string_lossy().to_string()),
            "parent": parent,
            "entries": entries,
        })),
    )
}

#[derive(Deserialize, Serialize)]
pub struct MkdirBody {
    path: String,
}

pub async fn handle_mkdir(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<MkdirBody>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let user_home = get_user_home(&state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);
    let home_str = user_home.to_string_lossy().to_string();
    let raw = &body.path;

    let expanded = if raw == "~" {
        home_str.clone()
    } else if raw.starts_with("~/") {
        format!("{}/{}", home_str, &raw[2..])
    } else {
        raw.clone()
    };

    // For mkdir the path may not exist yet; validate via the parent
    let safe_path = match super::path_guard::guard_write(&access_root, &expanded) {
        Ok(p) => p,
        Err(e) => return e,
    };

    match std::fs::create_dir_all(&safe_path) {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({"ok": true, "path": safe_path.to_string_lossy()})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        ),
    }
}
