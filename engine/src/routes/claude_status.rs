use axum::{Json, extract::State, http::StatusCode};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;
use crate::claude::spawn::find_claude_cli;
use crate::db::settings::{clear_anthropic_api_key, get_anthropic_api_key, set_anthropic_api_key};

/// GET /api/claude/status
///
/// Returns the auth state visible to the current process — no flamel login required.
/// Checks (in priority order):
///   1. ANTHROPIC_API_KEY env var
///   2. API key stored in app_settings
///   3. ~/.claude/.credentials.json (system claude auth / claude auth login)
pub async fn handle_get_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    let binary_path = find_claude_cli();
    let binary_found = binary_available(&binary_path);

    // 1. Env var (set before server started)
    if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
        if !k.is_empty() {
            return Json(json!({
                "binaryFound": binary_found,
                "binaryPath": binary_path,
                "authenticated": true,
                "authType": "api_key_env",
            }));
        }
    }

    // 2. Stored API key
    if let Some(k) = get_anthropic_api_key(&state.db) {
        if !k.is_empty() {
            return Json(json!({
                "binaryFound": binary_found,
                "binaryPath": binary_path,
                "authenticated": true,
                "authType": "api_key_stored",
            }));
        }
    }

    // 3. System credentials file
    let (authenticated, auth_type, expires_at, expired) = check_system_credentials();

    Json(json!({
        "binaryFound": binary_found,
        "binaryPath": binary_path,
        "authenticated": authenticated,
        "authType": auth_type,
        "expiresAt": expires_at,
        "expired": expired,
    }))
}

/// POST /api/claude/apikey
/// Body: { "apiKey": "sk-ant-..." }
pub async fn handle_set_api_key(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let key = match body.get("apiKey").and_then(|v| v.as_str()) {
        Some(k) if !k.is_empty() => k.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "apiKey is required"})),
            );
        }
    };
    match set_anthropic_api_key(&state.db, &key) {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))),
        Err(error) => {
            tracing::error!(%error, "failed to encrypt Anthropic API key");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Could not securely store API key"})),
            )
        }
    }
}

/// DELETE /api/claude/apikey
pub async fn handle_delete_api_key(State(state): State<Arc<AppState>>) -> Json<Value> {
    clear_anthropic_api_key(&state.db);
    Json(json!({"ok": true}))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn binary_available(path: &str) -> bool {
    std::process::Command::new(path)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn check_system_credentials() -> (bool, &'static str, Option<i64>, bool) {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return (false, "none", None, false),
    };

    // Check CLAUDE_CONFIG_DIR override first, then default ~/.claude
    let config_dir =
        std::env::var("CLAUDE_CONFIG_DIR").unwrap_or_else(|_| format!("{home}/.claude"));
    let creds_path = std::path::Path::new(&config_dir).join(".credentials.json");

    if !creds_path.exists() {
        return (false, "none", None, false);
    }

    let content = match std::fs::read_to_string(&creds_path) {
        Ok(c) => c,
        Err(_) => return (false, "none", None, false),
    };

    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return (false, "none", None, false),
    };

    // Claude Code stores { "claudeAiOauth": { "accessToken": "...", "expiresAt": ms_timestamp } }
    if let Some(oauth) = parsed.get("claudeAiOauth") {
        let expires_ms = oauth.get("expiresAt").and_then(|v| v.as_i64());
        let expires_at_secs = expires_ms.map(|ms| ms / 1000);
        let now = chrono::Utc::now().timestamp();
        let expired = expires_at_secs.map(|e| e < now).unwrap_or(false);
        return (true, "oauth", expires_at_secs, expired);
    }

    // Some installs use { "oauthToken": ... }
    if parsed.get("oauthToken").is_some() {
        return (true, "oauth", None, false);
    }

    (false, "none", None, false)
}
