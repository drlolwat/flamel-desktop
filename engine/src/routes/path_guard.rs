use axum::{Json, http::StatusCode};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn access_root(home: &Path, local_mode: bool) -> PathBuf {
    if local_mode {
        PathBuf::new()
    } else {
        home.to_path_buf()
    }
}

pub fn is_unrestricted_root(root: &Path) -> bool {
    root.as_os_str().is_empty()
}

/// Canonicalize `requested` and verify it falls within the allowed root.
/// For paths that must already exist (reads, directory listings).
pub fn guard_existing(root: &Path, requested: &str) -> Result<PathBuf, (StatusCode, Json<Value>)> {
    if !is_unrestricted_root(root) && requested.contains("..") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "invalid path"})),
        ));
    }
    let canonical = std::fs::canonicalize(requested).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "path not found"})),
        )
    })?;
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    if !is_unrestricted_root(root) && !canonical.starts_with(&canonical_root) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "path outside allowed root"})),
        ));
    }
    Ok(canonical)
}

/// Validate a path for writing — the file may not exist yet, so canonicalize the parent.
pub fn guard_write(root: &Path, requested: &str) -> Result<PathBuf, (StatusCode, Json<Value>)> {
    if !is_unrestricted_root(root) && requested.contains("..") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "invalid path"})),
        ));
    }
    let p = Path::new(requested);
    let parent = p.parent().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid path"})),
        )
    })?;
    let canonical_parent = std::fs::canonicalize(parent).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "parent directory not found"})),
        )
    })?;
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    if !is_unrestricted_root(root) && !canonical_parent.starts_with(&canonical_root) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "path outside allowed root"})),
        ));
    }
    let filename = p.file_name().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid path"})),
        )
    })?;
    Ok(canonical_parent.join(filename))
}
