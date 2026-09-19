use axum::{Json, extract::Query, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path; // used by build_file_tree / load_gitignore

#[derive(Deserialize)]
pub struct ListQuery {
    path: Option<String>,
    show_hidden: Option<String>,
}

const DEFAULT_IGNORED: &[&str] = &[
    "node_modules",
    ".git",
    ".DS_Store",
    "dist",
    ".next",
    "__pycache__",
    ".turbo",
    "build",
    "coverage",
    ".cache",
];

fn load_gitignore(dir: &Path) -> ignore::gitignore::Gitignore {
    let (gi, _) = ignore::gitignore::Gitignore::new(dir.join(".gitignore"));
    gi
}

fn is_ignored(
    name: &str,
    full_path: &Path,
    is_dir: bool,
    gi: &ignore::gitignore::Gitignore,
) -> bool {
    if DEFAULT_IGNORED.contains(&name) {
        return true;
    }
    gi.matched(full_path, is_dir).is_ignore()
}

fn build_file_tree(
    dir: &Path,
    gi: &ignore::gitignore::Gitignore,
    show_hidden: bool,
    depth: usize,
) -> Vec<Value> {
    if depth > 6 {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut names: Vec<_> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();

    let mut nodes = Vec::new();
    for name in names {
        if !show_hidden && name.starts_with('.') {
            continue;
        }

        let full_path = dir.join(&name);
        let metadata = match std::fs::metadata(&full_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let is_dir = metadata.is_dir();

        if is_ignored(&name, &full_path, is_dir, gi) {
            continue;
        }

        let path_str = full_path.to_string_lossy().to_string();

        if is_dir {
            // Load a child gitignore if one exists, else reuse parent's
            let child_gi = load_gitignore(&full_path);
            let children = build_file_tree(&full_path, &child_gi, show_hidden, depth + 1);
            nodes.push(json!({
                "name": name,
                "path": path_str,
                "type": "directory",
                "children": children,
            }));
        } else {
            nodes.push(json!({
                "name": name,
                "path": path_str,
                "type": "file",
            }));
        }
    }

    nodes
}

pub async fn handle_list_files(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::AppState>>,
    headers: axum::http::HeaderMap,
    Query(query): Query<ListQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = crate::routes::admin::get_current_user_id(&state, &headers);
    let user_home = crate::routes::admin::get_user_home(&state, user_id.as_deref());
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);

    let path_str = match &query.path {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "path required"})),
            );
        }
    };

    let safe_path = match super::path_guard::guard_existing(&access_root, path_str) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let show_hidden = query.show_hidden.as_deref() == Some("true");
    let gi = load_gitignore(&safe_path);
    let tree = build_file_tree(&safe_path, &gi, show_hidden, 0);

    (
        StatusCode::OK,
        Json(json!({"path": path_str, "tree": tree})),
    )
}
