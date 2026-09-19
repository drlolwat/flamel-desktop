use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{AppState, db::artifacts::GeneratedArtifact};

use super::admin::{get_current_user_id, get_user_home};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactQuery {
    session_id: Option<String>,
    download: Option<bool>,
}

fn media_type(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "gif" => Some("image/gif"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn is_provider_generated_image(path: &Path) -> bool {
    let components = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    components
        .windows(2)
        .any(|pair| pair[0] == ".codex" && pair[1] == "generated_images")
}

pub fn import_generated_image(
    state: &AppState,
    user_id: &str,
    session_id: &str,
    tool_call_id: Option<&str>,
    source_path: &str,
) -> Result<GeneratedArtifact, String> {
    let source = std::fs::canonicalize(source_path)
        .map_err(|error| format!("generated image is unavailable: {error}"))?;
    if !is_provider_generated_image(&source) {
        return Err("provider output is not in a generated image directory".to_string());
    }
    let media_type = media_type(&source)
        .ok_or_else(|| "provider output is not a supported image".to_string())?;
    let bytes = std::fs::read(&source)
        .map_err(|error| format!("failed to read generated image: {error}"))?;
    if bytes.is_empty() || bytes.len() > 64 * 1024 * 1024 {
        return Err("generated image has an invalid size".to_string());
    }

    let id = uuid::Uuid::new_v4().to_string();
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png");
    let artifact_root = std::env::var_os("FLAMEL_ARTIFACTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| state.home_dir.join(".artifacts"));
    let user_root = artifact_root.join(user_id).join(&id);
    std::fs::create_dir_all(&user_root)
        .map_err(|error| format!("failed to create artifact storage: {error}"))?;
    let stored = user_root.join(format!("content.{extension}"));
    std::fs::write(&stored, &bytes)
        .map_err(|error| format!("failed to store generated image: {error}"))?;

    let name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("generated-image.png");
    let sha256 = hex::encode(Sha256::digest(&bytes));
    crate::db::artifacts::insert(
        &state.db,
        &id,
        user_id,
        session_id,
        tool_call_id,
        "image",
        name,
        media_type,
        bytes.len() as i64,
        &sha256,
        &stored.to_string_lossy(),
        &json!({"source": "provider_image_generation"}),
    )
}

pub async fn handle_list_artifacts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ArtifactQuery>,
) -> (StatusCode, Json<Value>) {
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    };
    let Some(session_id) = query.session_id.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "sessionId required"})),
        );
    };
    match crate::db::artifacts::list_for_session(&state.db, &user_id, session_id) {
        Ok(artifacts) => (
            StatusCode::OK,
            Json(
                json!({"artifacts": artifacts.iter().map(GeneratedArtifact::public_ref).collect::<Vec<_>>() }),
            ),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        ),
    }
}

pub async fn handle_get_artifact(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> (StatusCode, Json<Value>) {
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    };
    match crate::db::artifacts::get_for_user(&state.db, &id, &user_id) {
        Ok(Some(artifact)) => (StatusCode::OK, Json(artifact.public_ref())),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "artifact not found"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        ),
    }
}

pub async fn handle_get_artifact_content(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ArtifactQuery>,
) -> Response {
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))).into_response();
    };
    let artifact = match crate::db::artifacts::get_for_user(&state.db, &id, &user_id) {
        Ok(Some(artifact)) => artifact,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "artifact not found"})),
            )
                .into_response();
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": error})),
            )
                .into_response();
        }
    };
    let bytes = match std::fs::read(&artifact.storage_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::GONE,
                Json(json!({"error": "artifact content unavailable"})),
            )
                .into_response();
        }
    };
    let mut response_headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&artifact.media_type) {
        response_headers.insert(header::CONTENT_TYPE, value);
    }
    response_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    response_headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if query.download.unwrap_or(false) {
        let safe_name = artifact.name.replace(['"', '\r', '\n'], "_");
        if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{safe_name}\"")) {
            response_headers.insert(header::CONTENT_DISPOSITION, value);
        }
    }
    (StatusCode::OK, response_headers, bytes).into_response()
}

fn valid_mcp_artifact_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".txt") else {
        return false;
    };
    !stem.is_empty()
        && stem.len() <= 160
        && stem
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

pub async fn handle_get_mcp_artifact(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(name): AxumPath<String>,
    Query(query): Query<ArtifactQuery>,
) -> Response {
    if get_current_user_id(&state, &headers).is_none() {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))).into_response();
    }
    if !valid_mcp_artifact_name(&name) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid artifact name"})),
        )
            .into_response();
    }

    let root = std::env::temp_dir().join("flamel-mcp-artifacts");
    let canonical_root = match std::fs::canonicalize(&root) {
        Ok(path) => path,
        Err(_) => {
            return (
                StatusCode::GONE,
                Json(json!({"error": "artifact content unavailable"})),
            )
                .into_response();
        }
    };
    let artifact = match std::fs::canonicalize(root.join(&name)) {
        Ok(path) if path.parent() == Some(canonical_root.as_path()) => path,
        _ => {
            return (
                StatusCode::GONE,
                Json(json!({"error": "artifact content unavailable"})),
            )
                .into_response();
        }
    };
    let metadata = match std::fs::metadata(&artifact) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= 64 * 1024 * 1024 => metadata,
        _ => {
            return (
                StatusCode::GONE,
                Json(json!({"error": "artifact content unavailable"})),
            )
                .into_response();
        }
    };
    let bytes = match std::fs::read(&artifact) {
        Ok(bytes) if bytes.len() as u64 == metadata.len() => bytes,
        _ => {
            return (
                StatusCode::GONE,
                Json(json!({"error": "artifact content unavailable"})),
            )
                .into_response();
        }
    };

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response_headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if query.download.unwrap_or(false)
        && let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
    {
        response_headers.insert(header::CONTENT_DISPOSITION, value);
    }
    (StatusCode::OK, response_headers, bytes).into_response()
}

pub async fn handle_save_artifact(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    };
    let Some(target_path) = body.get("targetPath").and_then(Value::as_str) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "targetPath required"})),
        );
    };
    let artifact = match crate::db::artifacts::get_for_user(&state.db, &id, &user_id) {
        Ok(Some(artifact)) => artifact,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "artifact not found"})),
            );
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": error})),
            );
        }
    };
    let user_home = get_user_home(&state, Some(&user_id));
    let access_root = super::path_guard::access_root(&user_home, state.local_mode);
    let target = match super::path_guard::guard_write(&access_root, target_path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    match std::fs::copy(&artifact.storage_path, &target) {
        Ok(_) => (StatusCode::OK, Json(json!({"ok": true, "path": target}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("failed to save artifact: {error}")})),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::valid_mcp_artifact_name;

    #[test]
    fn mcp_artifact_names_are_capability_scoped() {
        assert!(valid_mcp_artifact_name(
            "file_read-8c3a198f-e891-4517-ab2f-512657821130.txt"
        ));
        for invalid in [
            "../secret.txt",
            "nested/secret.txt",
            "secret.json",
            ".txt",
            "secret%2Fother.txt",
        ] {
            assert!(
                !valid_mcp_artifact_name(invalid),
                "accepted invalid artifact name: {invalid}"
            );
        }
    }
}
