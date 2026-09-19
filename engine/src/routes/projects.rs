use axum::{Json, extract::State, http::HeaderMap};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

use crate::AppState;
use crate::routes::admin::{get_current_user_id, get_user_home};

pub async fn handle_get_projects(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Value> {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            tracing::warn!("get_projects: no user_id — returning empty");
            return Json(json!([]));
        }
    };
    let user_home = get_user_home(&state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);
    let canonical_root = if super::path_guard::is_unrestricted_root(&access_root) {
        None
    } else {
        Some(std::fs::canonicalize(&access_root).unwrap_or_else(|_| access_root.clone()))
    };

    let home = if state.local_mode {
        user_home.clone()
    } else {
        std::env::var("HOME").map(PathBuf::from).unwrap_or_default()
    };
    let mut projects: Vec<Value> = Vec::new();

    // Read ~/.claude.json for known projects
    let claude_json = home.join(".claude.json");
    if let Ok(content) = std::fs::read_to_string(&claude_json) {
        if let Ok(config) = serde_json::from_str::<Value>(&content) {
            if let Some(proj_map) = config.get("projects").and_then(|v| v.as_object()) {
                for path in proj_map.keys() {
                    if let Ok(canonical) = std::fs::canonicalize(path) {
                        if let Some(root) = &canonical_root {
                            if !canonical.starts_with(root) {
                                continue;
                            }
                        }
                    } else {
                        continue;
                    }
                    let name = path.rsplit('/').next().unwrap_or(path);
                    projects.push(json!({"path": path, "name": name}));
                }
            }
        }
    }

    // Scan ~/.claude/projects/ for known projects
    let projects_dir = home.join(".claude").join("projects");
    if let Ok(entries) = std::fs::read_dir(&projects_dir) {
        for entry in entries.flatten() {
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let decoded_path = dir_name.replace('-', "/");
            if let Ok(canonical) = std::fs::canonicalize(&decoded_path) {
                if let Some(root) = &canonical_root {
                    if !canonical.starts_with(root) {
                        continue;
                    }
                }
            } else {
                continue;
            }
            if !projects
                .iter()
                .any(|p| p.get("path").and_then(|v| v.as_str()) == Some(&decoded_path))
            {
                let name = decoded_path.rsplit('/').next().unwrap_or(&decoded_path);
                projects.push(json!({"path": decoded_path, "name": name}));
            }
        }
    }

    Json(json!(projects))
}
