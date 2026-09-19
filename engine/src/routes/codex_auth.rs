use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

use crate::AppState;
use crate::db::provider_auth::{self, ProviderCredentials};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_URL: &str = "https://auth0.openai.com/oauth/token";
const PROVIDER: &str = "codex";

use crate::routes::admin::get_current_user_id;

// GET /api/me/codex-auth
pub async fn handle_get_auth_status(
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

    let connected = provider_auth::has_credentials(&state.db, &user_id, PROVIDER);
    if connected {
        let creds =
            provider_auth::get_credentials(&state.db, &state.credential_vault, &user_id, PROVIDER)
                .unwrap();
        let now = chrono::Utc::now().timestamp();
        (
            StatusCode::OK,
            Json(json!({
                "connected": true,
                "source": "flamel",
                "expires_at": creds.expires_at,
                "expired": creds.expires_at < now,
                "scopes": creds.scopes,
            })),
        )
    } else if state.local_mode && local_codex_credentials_present() {
        (
            StatusCode::OK,
            Json(json!({
                "connected": true,
                "source": "local_cli",
            })),
        )
    } else {
        (StatusCode::OK, Json(json!({"connected": false})))
    }
}

pub async fn try_refresh_codex_credentials(
    state: &AppState,
    user_id: &str,
) -> Option<ProviderCredentials> {
    let creds =
        provider_auth::get_credentials(&state.db, &state.credential_vault, user_id, PROVIDER)?;
    let now = chrono::Utc::now().timestamp();
    if creds.expires_at > now + 300 {
        return Some(creds);
    }
    let refresh_token = creds.refresh_token.as_deref()?;

    tracing::info!("refreshing Codex OAuth token for user {}", user_id);

    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_URL)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLIENT_ID,
        }))
        .send()
        .await
        .ok()?;

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.ok()?;

    if !status.is_success() {
        tracing::error!("Codex token refresh failed ({})", status);
        return None;
    }

    let access_token = body.get("access_token").and_then(|v| v.as_str())?;
    let new_refresh = body.get("refresh_token").and_then(|v| v.as_str());
    let new_id_token = body.get("id_token").and_then(|v| v.as_str());
    let expires_in = body
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .unwrap_or(3600);
    let expires_at = now + expires_in;
    let scopes = body
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or(&creds.scopes);

    provider_auth::upsert_credentials(
        &state.db,
        &state.credential_vault,
        user_id,
        PROVIDER,
        access_token,
        new_refresh.or(creds.refresh_token.as_deref()),
        new_id_token.or(creds.id_token.as_deref()),
        expires_at,
        scopes,
        None,
    )
    .ok()?;

    tracing::info!(
        "Codex OAuth token refreshed for user {}, expires in {}s",
        user_id,
        expires_in
    );
    provider_auth::get_credentials(&state.db, &state.credential_vault, user_id, PROVIDER)
}

// POST /api/me/codex-auth/refresh
pub async fn handle_refresh_auth(
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
    match try_refresh_codex_credentials(&state, &user_id).await {
        Some(creds) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "expires_at": creds.expires_at,
            })),
        ),
        None if state.local_mode && local_codex_credentials_present() => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "source": "local_cli",
            })),
        ),
        None => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Refresh failed — token may be invalid or no refresh token stored. Please reconnect."
            })),
        ),
    }
}

// DELETE /api/me/codex-auth
pub async fn handle_revoke_auth(
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
    provider_auth::delete_credentials(&state.db, &user_id, PROVIDER);
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "local_cli_still_available": state.local_mode && local_codex_credentials_present(),
        })),
    )
}

pub fn local_codex_home() -> PathBuf {
    std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::flamel_local_user_home().join(".codex"))
}

pub fn local_codex_credentials_present() -> bool {
    let auth_json_path = local_codex_home().join("auth.json");
    let Ok(content) = std::fs::read_to_string(&auth_json_path) else {
        return false;
    };
    let Ok(auth) = serde_json::from_str::<Value>(&content) else {
        return false;
    };
    auth.get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        .is_some_and(|token| !token.trim().is_empty())
}

fn safe_user_id(user_id: &str) -> String {
    user_id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn codex_auth_workspace(state: &AppState, user_id: &str) -> (PathBuf, bool) {
    if state.local_mode {
        (local_codex_home(), true)
    } else {
        (
            PathBuf::from(format!("/tmp/codex-auth-{}", safe_user_id(user_id))),
            false,
        )
    }
}

fn find_codex_binary() -> String {
    if let Ok(p) = std::env::var("CODEX_CLI_PATH") {
        if std::path::Path::new(&p).exists() {
            return p;
        }
    }
    for p in ["/usr/local/bin/codex", "/usr/bin/codex"] {
        if std::path::Path::new(p).exists() {
            return p.to_string();
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = format!("{home}/.npm-global/bin/codex");
        if std::path::Path::new(&p).exists() {
            return p;
        }
    }
    "codex".to_string()
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    // CSI sequence: \x1b[...letter
                    for nc in chars.by_ref() {
                        if nc.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC sequence: \x1b]...BEL or \x1b]...\x1b\\
                    // Contains hyperlinks like \x1b]8;;URL\x07text\x1b]8;;\x07
                    // Extract URLs from the OSC payload
                    let mut payload = String::new();
                    for nc in chars.by_ref() {
                        if nc == '\x07' {
                            break;
                        }
                        if nc == '\x1b' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                        payload.push(nc);
                    }
                    // OSC 8 hyperlinks: ]8;;URL — emit the URL as text
                    if payload.starts_with("8;;") {
                        let url_part = &payload[3..];
                        if !url_part.is_empty() {
                            out.push_str(url_part);
                            out.push(' ');
                        }
                    }
                }
                _ => {
                    // Other escape — skip one char
                    chars.next();
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_auth_line(line: &str, url: &mut Option<String>, code: &mut Option<String>) {
    if url.is_none() {
        if let Some(start) = line.find("https://") {
            let rest = &line[start..];
            let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
            let candidate = rest[..end].trim_end_matches(['.', ',', ')', '"', '\'']);
            if candidate.len() > 12 {
                *url = Some(candidate.to_string());
            }
        }
    }
    if code.is_none() {
        for word in line.split_whitespace() {
            let w = word.trim_matches(['.', ':', ',', ')', '(', '"', '\'']);
            if w.len() >= 7 && w.contains('-') && !w.starts_with("http") {
                let all_valid = w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
                let has_upper = w.chars().any(|c| c.is_ascii_uppercase());
                if all_valid && has_upper {
                    *code = Some(w.to_string());
                    break;
                }
            }
        }
    }
}

// POST /api/me/codex-auth/device/start
pub async fn handle_device_start(
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

    let (codex_home_path, local_cli_home) = codex_auth_workspace(&state, &user_id);
    let codex_home = codex_home_path.to_string_lossy().into_owned();
    let pid_file = format!("{codex_home}/flamel-auth.pid");

    // Kill any running auth process from a previous attempt
    if let Ok(pid_str) = tokio::fs::read_to_string(&pid_file).await {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            let _ = std::process::Command::new("kill")
                .args(["-15", &pid.to_string()])
                .output();
        }
    }

    if !local_cli_home {
        let _ = tokio::fs::remove_dir_all(&codex_home).await;
    }
    if let Err(e) = tokio::fs::create_dir_all(&codex_home).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to create temp dir: {e}")})),
        );
    }

    let codex_bin = find_codex_binary();
    tracing::info!("starting codex device auth: {codex_bin}");

    // Redirect stdout/stderr to a log file so the process doesn't die from SIGPIPE
    let output_path = format!("{codex_home}/flamel-auth-output.log");
    let out_file = match std::fs::File::create(&output_path) {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to create output log: {e}")})),
            );
        }
    };
    let err_file = match out_file.try_clone() {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to clone file handle: {e}")})),
            );
        }
    };

    let mut command = tokio::process::Command::new(&codex_bin);
    crate::mcp_parent_guard::configure_tokio_child(&mut command);
    let child = match command
        .args(["login", "--device-auth"])
        .env("CODEX_HOME", &codex_home)
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .stdout(std::process::Stdio::from(out_file))
        .stderr(std::process::Stdio::from(err_file))
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to launch codex ({codex_bin}): {e}")})),
            );
        }
    };

    if let Some(pid) = child.id() {
        let _ = tokio::fs::write(&pid_file, pid.to_string()).await;
    }

    // Process runs independently — poll the output file for URL + user code
    let mut verification_url: Option<String> = None;
    let mut user_code: Option<String> = None;
    let mut last_len = 0usize;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);

    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        if let Ok(content) = tokio::fs::read_to_string(&output_path).await {
            if content.len() > last_len {
                last_len = content.len();
                let clean = strip_ansi(&content);
                for line in clean.lines() {
                    parse_auth_line(line, &mut verification_url, &mut user_code);
                }
                if verification_url.is_some() {
                    break;
                }
            }
        }
    }

    // Fallback: try to extract user_code from URL query param
    if user_code.is_none() {
        if let Some(url) = &verification_url {
            if let Some(pos) = url.find("user_code=") {
                let rest = &url[pos + 10..];
                let end = rest.find('&').unwrap_or(rest.len());
                user_code = Some(rest[..end].to_string());
            }
        }
    }

    let verification_url =
        verification_url.unwrap_or_else(|| "https://auth.openai.com/activate".to_string());

    (
        StatusCode::OK,
        Json(json!({
            "verification_url": verification_url,
            "user_code": user_code,
            "interval": 3,
            "expires_in": 900,
        })),
    )
}

// POST /api/me/codex-auth/device/poll
pub async fn handle_device_poll(
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

    let (codex_home_path, local_cli_home) = codex_auth_workspace(&state, &user_id);
    let auth_json_path = codex_home_path.join("auth.json");

    let content = match tokio::fs::read_to_string(&auth_json_path).await {
        Ok(c) => c,
        Err(_) => return (StatusCode::OK, Json(json!({"pending": true}))),
    };

    let auth: serde_json::Value = match serde_json::from_str(&content) {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to parse auth.json: {e}")})),
            );
        }
    };

    let tokens = match auth.get("tokens") {
        Some(t) => t,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "auth.json missing tokens field"})),
            );
        }
    };

    let access_token = match tokens.get("access_token").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "auth.json missing access_token"})),
            );
        }
    };

    let refresh_token = tokens
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let id_token = tokens
        .get("id_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let expires_at = chrono::Utc::now().timestamp() + 3600;

    match provider_auth::upsert_credentials(
        &state.db,
        &state.credential_vault,
        &user_id,
        PROVIDER,
        &access_token,
        refresh_token.as_deref(),
        id_token.as_deref(),
        expires_at,
        "openid offline_access",
        None,
    ) {
        Ok(()) => {
            if !local_cli_home {
                let _ = tokio::fs::remove_dir_all(&codex_home_path).await;
            }
            (StatusCode::OK, Json(json!({"ok": true})))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to save credentials: {e}")})),
        ),
    }
}
