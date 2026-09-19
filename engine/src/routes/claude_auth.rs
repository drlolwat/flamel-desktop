use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

use crate::db::claude_auth::{
    delete_credentials, get_credentials, has_credentials, upsert_credentials,
};

use crate::AppState;

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const SCOPES: &str =
    "user%3Aprofile%20user%3Ainference%20user%3Asessions%3Aclaude_code%20user%3Amcp_servers";

use crate::routes::admin::get_current_user_id;

// GET /api/me/claude-auth
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

    let connected = has_credentials(&state.db, &user_id);
    if connected {
        let creds = get_credentials(&state.db, &state.credential_vault, &user_id).unwrap();
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
    } else if state.local_mode && local_claude_credentials_present() {
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

pub fn local_claude_credentials_present() -> bool {
    if std::env::var("ANTHROPIC_API_KEY").is_ok_and(|k| !k.trim().is_empty()) {
        return true;
    }

    let config_dir = std::env::var("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::flamel_local_user_home().join(".claude"));
    let creds_path = config_dir.join(".credentials.json");
    if !creds_path.exists() {
        return false;
    }

    let Ok(content) = std::fs::read_to_string(&creds_path) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&content) else {
        return false;
    };
    parsed.get("claudeAiOauth").is_some() || parsed.get("oauthToken").is_some()
}

// GET /api/me/claude-auth/oauth/start
// In desktop mode: binds a temporary local server on a random port for callback.
// In web mode: uses the server's own callback URL so the browser can redirect back.
pub async fn handle_oauth_start(
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

    let desktop_mode = true; // always use localhost callback

    // Generate PKCE
    let verifier_bytes: [u8; 32] = rand::random();
    let code_verifier = base64_url_encode(&verifier_bytes);

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let code_challenge = base64_url_encode(&hasher.finalize());

    let state_bytes: [u8; 16] = rand::random();
    let oauth_state = hex::encode(state_bytes);

    let redirect_uri = if desktop_mode {
        // Desktop: bind temp localhost callback server
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("Failed to bind callback port: {e}")})),
                );
            }
        };
        let port = listener.local_addr().unwrap().port();
        let uri = format!("http://localhost:{port}/callback");

        let state_clone = Arc::clone(&state);
        tokio::spawn(async move {
            run_callback_server(listener, state_clone).await;
        });
        uri
    } else {
        // Web mode: use the server's own callback route.
        // Reconstruct server URL from the request's Origin or Host header.
        let origin = headers
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let host = headers.get("host").and_then(|v| v.to_str().ok()).map(|h| {
            // If request came via HTTPS (reverse proxy), use https
            let proto = headers
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("http");
            format!("{proto}://{h}")
        });

        let base_url = origin
            .or(host)
            .unwrap_or_else(|| "http://localhost:8181".to_string());
        format!("{base_url}/api/me/claude-auth/oauth/callback")
    };

    // Store PKCE state
    {
        let mut states = state.oauth_states.lock().unwrap();
        states.insert(
            oauth_state.clone(),
            OAuthPkceState {
                code_verifier,
                redirect_uri: redirect_uri.clone(),
                user_id,
                created_at: std::time::SystemTime::now(),
            },
        );
    }

    let redirect_uri_enc = urlencoded(&redirect_uri);
    let url = format!(
        "{AUTHORIZE_URL}?response_type=code&client_id={CLIENT_ID}&redirect_uri={redirect_uri_enc}&scope={SCOPES}&code_challenge={code_challenge}&code_challenge_method=S256&state={oauth_state}"
    );

    if desktop_mode {
        // Open browser from server side in desktop mode
        let url_clone = url.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = open::that(&url_clone) {
                tracing::warn!("failed to open browser for OAuth: {e}");
            }
        });
    }

    (StatusCode::OK, Json(json!({"url": url})))
}

/// Minimal one-shot HTTP server that handles the OAuth redirect, exchanges code for tokens,
/// saves credentials, then exits.
async fn run_callback_server(listener: tokio::net::TcpListener, state: Arc<AppState>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Wait up to 5 minutes for the browser to complete OAuth
    let accept = tokio::time::timeout(std::time::Duration::from_secs(300), listener.accept()).await;

    let (mut stream, _) = match accept {
        Ok(Ok(s)) => s,
        _ => return, // timeout or error — just exit
    };

    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await.unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);

    // Parse query string from first line: GET /callback?code=...&state=... HTTP/1.1
    let query = req
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|path| path.splitn(2, '?').nth(1))
        .unwrap_or("");

    let mut params: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
            params.insert(url_decode(k), url_decode(v));
        }
    }

    let (success, message) = exchange_code(&state, &params).await;

    // Send the response before shutting down
    let body = html_result_str(success, &message);
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
}

async fn exchange_code(
    state: &Arc<AppState>,
    params: &std::collections::HashMap<String, String>,
) -> (bool, String) {
    if let Some(err) = params.get("error") {
        return (false, format!("OAuth error: {err}"));
    }

    let code = match params.get("code") {
        Some(c) => c.clone(),
        None => return (false, "Missing authorization code".into()),
    };
    let oauth_state = match params.get("state") {
        Some(s) => s.clone(),
        None => return (false, "Missing state parameter".into()),
    };

    let pkce = {
        let mut states = state.oauth_states.lock().unwrap();
        states.remove(&oauth_state)
    };
    let pkce = match pkce {
        Some(p) => p,
        None => return (false, "Invalid or expired state — please try again".into()),
    };

    let client = reqwest::Client::new();
    let resp = match client
        .post(TOKEN_URL)
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": pkce.redirect_uri,
            "client_id": CLIENT_ID,
            "code_verifier": pkce.code_verifier,
            "state": oauth_state,
        }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return (false, format!("Token request failed: {e}")),
    };

    let status = resp.status();
    let raw = match resp.text().await {
        Ok(t) => t,
        Err(e) => return (false, format!("Failed to read token response: {e}")),
    };
    tracing::debug!("Token response status: {status}");
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(b) => b,
        Err(e) => return (false, format!("Failed to parse token response: {e}")),
    };

    if let Some(err) = body.get("error").and_then(|v| v.as_str()) {
        let desc = body
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return (false, format!("Token error: {err} — {desc}"));
    }

    let access_token = match body.get("access_token").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return (false, "No access_token in response".into()),
    };
    let refresh_token = body
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let expires_in = body
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + expires_in;
    let scopes = body
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("user:profile user:inference user:sessions:claude_code user:mcp_servers");

    match upsert_credentials(
        &state.db,
        &state.credential_vault,
        &pkce.user_id,
        &access_token,
        refresh_token.as_deref(),
        expires_at,
        scopes,
    ) {
        Ok(()) => (
            true,
            "Connected successfully! You can close this tab.".into(),
        ),
        Err(e) => (false, format!("Failed to save credentials: {e}")),
    }
}

/// Refresh the OAuth token for a user. Returns the refreshed credentials on success.
/// Called automatically from chat.rs before spawning a Claude CLI process.
pub async fn try_refresh_credentials(
    state: &AppState,
    user_id: &str,
) -> Option<crate::db::claude_auth::ClaudeCredentials> {
    let creds = get_credentials(&state.db, &state.credential_vault, user_id)?;
    let now = chrono::Utc::now().timestamp();
    // Only refresh if expired or expiring within 5 minutes
    if creds.expires_at > now + 300 {
        return Some(creds);
    }
    let refresh_token = creds.refresh_token.as_deref()?;

    tracing::info!("refreshing OAuth token for user {}", user_id);

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
        tracing::error!("token refresh failed ({})", status);
        return None;
    }

    let access_token = body.get("access_token").and_then(|v| v.as_str())?;
    let new_refresh = body.get("refresh_token").and_then(|v| v.as_str());
    let expires_in = body
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .unwrap_or(3600);
    let expires_at = now + expires_in;
    let scopes = body
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or(&creds.scopes);

    upsert_credentials(
        &state.db,
        &state.credential_vault,
        user_id,
        access_token,
        new_refresh.or(creds.refresh_token.as_deref()),
        expires_at,
        scopes,
    )
    .ok()?;

    tracing::info!(
        "OAuth token refreshed for user {}, expires in {}s",
        user_id,
        expires_in
    );
    get_credentials(&state.db, &state.credential_vault, user_id)
}

// POST /api/me/claude-auth/refresh
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
    match try_refresh_credentials(&state, &user_id).await {
        Some(creds) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "expires_at": creds.expires_at,
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

// DELETE /api/me/claude-auth
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
    delete_credentials(&state.db, &user_id);
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "local_cli_still_available": state.local_mode && local_claude_credentials_present(),
        })),
    )
}

// Kept for any routes that might still reference it (callback no longer used via main router)
#[derive(Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

pub async fn handle_oauth_callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CallbackQuery>,
) -> axum::response::Html<String> {
    let mut map = std::collections::HashMap::new();
    if let Some(c) = &params.code {
        map.insert("code".to_string(), c.clone());
    }
    if let Some(s) = &params.state {
        map.insert("state".to_string(), s.clone());
    }
    if let Some(e) = &params.error {
        map.insert("error".to_string(), e.clone());
    }
    let (success, msg) = exchange_code(&state, &map).await;
    axum::response::Html(html_result_str(success, &msg))
}

// --- helpers ---

pub struct OAuthPkceState {
    pub code_verifier: String,
    pub redirect_uri: String,
    pub user_id: String,
    pub created_at: std::time::SystemTime,
}

fn base64_url_encode(bytes: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(bytes)
}

fn urlencoded(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                vec![c.to_string()]
            } else {
                vec![format!("%{:02X}", c as u8)]
            }
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next().unwrap_or('0');
            let h2 = chars.next().unwrap_or('0');
            if let Ok(byte) = u8::from_str_radix(&format!("{h1}{h2}"), 16) {
                result.push(byte as char);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

fn html_result_str(success: bool, message: &str) -> String {
    let (icon, color) = if success {
        ("✓", "#22c55e")
    } else {
        ("✗", "#ef4444")
    };
    format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Claude Auth</title>
<style>body{{font-family:system-ui,sans-serif;display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0;background:#0f172a;color:#f1f5f9}}
.box{{text-align:center;padding:2rem}}.icon{{font-size:3rem;color:{color}}}.msg{{margin-top:1rem;font-size:1.1rem}}</style></head>
<body><div class="box"><div class="icon">{icon}</div><div class="msg">{message}</div></div></body></html>"#
    )
}

// POST /api/me/claude-auth/oauth/complete — accepts pasted callback URL from browser
pub async fn handle_oauth_complete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let _user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };

    let url_str = match body.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Missing url field"})),
            );
        }
    };

    // Parse query params from the pasted URL
    let query = url_str.splitn(2, '?').nth(1).unwrap_or("");
    let mut params: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
            params.insert(url_decode(k), url_decode(v));
        }
    }

    let (success, message) = exchange_code(&state, &params).await;
    if success {
        (
            StatusCode::OK,
            Json(json!({"ok": true, "message": message})),
        )
    } else {
        (StatusCode::BAD_REQUEST, Json(json!({"error": message})))
    }
}
