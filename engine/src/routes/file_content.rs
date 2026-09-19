use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::AppState;
use crate::routes::admin::{get_current_user_id, get_user_home};

#[derive(Deserialize)]
pub struct GetQuery {
    path: Option<String>,
}

fn content_revision(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn media_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "css" => "text/css",
        "csv" => "text/csv",
        "htm" | "html" => "text/html",
        "js" | "jsx" | "mjs" | "cjs" => "text/javascript",
        "json" | "jsonc" => "application/json",
        "md" | "mdx" => "text/markdown",
        "rs" => "text/rust",
        "toml" => "application/toml",
        "ts" | "tsx" | "mts" | "cts" => "text/typescript",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        _ => "text/plain",
    }
}

fn file_revision(path: &Path, bytes: Vec<u8>) -> Value {
    let binary =
        std::str::from_utf8(&bytes).is_err() || bytes.iter().take(8192).any(|byte| *byte == 0);
    let modified_at = std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().to_string());
    json!({
        "path": path.to_string_lossy(),
        "content": if binary { String::new() } else { String::from_utf8_lossy(&bytes).into_owned() },
        "revision": content_revision(&bytes),
        "mediaType": media_type(path),
        "size": bytes.len(),
        "modifiedAt": modified_at,
        "binary": binary,
    })
}

fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let temp_path: PathBuf = parent.join(format!(".{name}.flamel-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(content)?;
        file.sync_all()?;
        std::fs::rename(&temp_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

pub async fn handle_get_file_content(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<GetQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let user_home = get_user_home(&state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);

    let path = match &query.path {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "path required"})),
            );
        }
    };

    let safe_path = match super::path_guard::guard_existing(&access_root, path) {
        Ok(p) => p,
        Err(e) => return e,
    };

    match std::fs::read(&safe_path) {
        Ok(bytes) => {
            let revision = file_revision(&safe_path, bytes);
            if revision.get("binary").and_then(Value::as_bool) == Some(true) {
                return (
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    Json(
                        json!({"error": "Binary files cannot be opened as text", "file": revision}),
                    ),
                );
            }
            (StatusCode::OK, Json(revision))
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Cannot read file"})),
        ),
    }
}

pub async fn handle_get_file_asset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<GetQuery>,
) -> Response {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))).into_response();
        }
    };
    let user_home = get_user_home(&state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);

    let path = match &query.path {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "path required"})),
            )
                .into_response();
        }
    };

    let safe_path = match super::path_guard::guard_existing(&access_root, path) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    let content_type = match image_content_type(&safe_path) {
        Some(content_type) => content_type,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "unsupported asset type"})),
            )
                .into_response();
        }
    };

    let bytes = match std::fs::read(&safe_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Cannot read file"})),
            )
                .into_response();
        }
    };

    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response_headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );

    (StatusCode::OK, response_headers, bytes).into_response()
}

pub async fn handle_save_file_content(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let user_home = get_user_home(&state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);

    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "path and content required"})),
            );
        }
    };

    let safe_path = match super::path_guard::guard_write(&access_root, path) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let content = match body.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "path and content required"})),
            );
        }
    };

    if let Some(expected) = body.get("expectedRevision").and_then(Value::as_str) {
        if safe_path.exists() {
            match std::fs::read(&safe_path) {
                Ok(bytes) if content_revision(&bytes) != expected => {
                    return (
                        StatusCode::CONFLICT,
                        Json(json!({
                            "error": "File changed since it was opened",
                            "code": "revision_conflict",
                            "path": path,
                            "expectedRevision": expected,
                            "current": file_revision(&safe_path, bytes),
                        })),
                    );
                }
                Ok(_) => {}
                Err(_) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": "Cannot verify file revision"})),
                    );
                }
            }
        }
    }

    match atomic_write(&safe_path, content.as_bytes()) {
        Ok(()) => {
            let revision = file_revision(&safe_path, content.as_bytes().to_vec());
            (StatusCode::OK, Json(json!({"ok": true, "file": revision})))
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Cannot write file"})),
        ),
    }
}

fn image_content_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("gif") => Some("image/gif"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("png") => Some("image/png"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}
