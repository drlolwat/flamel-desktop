use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

use crate::AppState;
use crate::db::{git_accounts, sessions};
use crate::routes::admin::get_current_user_id;

fn http_client(timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("flamel")
        .timeout(Duration::from_secs(timeout_secs))
        .connect_timeout(Duration::from_secs(8)) // fail fast on unreachable hosts
        .build()
        .unwrap_or_default()
}

fn internal_git_credentials_user_candidates(state: &AppState, headers: &HeaderMap) -> Vec<String> {
    let mut candidates = Vec::new();

    if let Some(flamel_id) = headers
        .get("x-flamel-user-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        candidates.push(flamel_id.to_string());
    }

    if state.local_mode {
        candidates.push(crate::FLAMEL_LOCAL_USER_ID.to_string());
    }

    if let Some(session_user_id) = headers
        .get("x-flamel-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|session_id| sessions::get_session(&state.db, session_id))
        .and_then(|session| session.user_id)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        candidates.push(session_user_id);
    }

    candidates.dedup();
    candidates
}

pub async fn handle_list_git_accounts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Value> {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return Json(json!([])),
    };
    let accounts = git_accounts::list_git_accounts(&state.db, Some(&user_id));
    Json(serde_json::to_value(accounts).unwrap())
}

pub async fn handle_create_git_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let account_type = match body.get("type").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "type required"})),
            );
        }
    };
    let token = match body.get("token").and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "token required"})),
            );
        }
    };
    let base_url = if account_type == "github" {
        "https://api.github.com".to_string()
    } else {
        match body.get("baseUrl").and_then(|v| v.as_str()) {
            Some(u) if !u.is_empty() => u.trim_end_matches('/').to_string(),
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "baseUrl required for gitea"})),
                );
            }
        }
    };

    // Fetch profile with a short timeout — non-fatal if it times out or fails
    let (username, avatar_url) = fetch_profile(&account_type, &base_url, &token).await;

    let label = body
        .get("label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            username.clone().unwrap_or_else(|| {
                if account_type == "github" {
                    "GitHub".to_string()
                } else {
                    "Gitea".to_string()
                }
            })
        });

    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::to_value(json!({"error": "Forbidden"})).unwrap()),
            );
        }
    };
    let account = git_accounts::create_git_account(
        &state.db,
        &state.credential_vault,
        Some(&user_id),
        &account_type,
        &label,
        &base_url,
        &token,
        username.as_deref(),
        avatar_url.as_deref(),
    );

    let account = match account {
        Ok(account) => account,
        Err(error) => {
            tracing::error!(%error, "failed to store Git account credential");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Could not securely store Git account"})),
            );
        }
    };

    (
        StatusCode::CREATED,
        Json(serde_json::to_value(account).unwrap()),
    )
}

pub async fn handle_delete_git_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    if let Some(account) =
        git_accounts::get_git_account_with_token(&state.db, &state.credential_vault, &id)
    {
        if account.user_id.as_deref() != Some(&user_id) {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }
    git_accounts::delete_git_account(&state.db, &id);
    (StatusCode::OK, Json(json!({"ok": true})))
}

pub async fn handle_list_git_account_repos(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    if id == "flamel-git" {
        return match crate::routes::flamel_git::repositories(&state, &headers).await {
            Ok(body) => {
                let repositories = body
                    .get("repositories")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|repository| {
                        json!({
                            "id": repository["id"],
                            "name": repository["name"],
                            "fullName": repository["full_name"],
                            "description": repository["description"],
                            "cloneUrl": repository["clone_url"],
                            "sshUrl": Value::Null,
                            "updatedAt": repository["updated_at"],
                            "isPrivate": repository["private"],
                            "defaultBranch": repository["default_branch"],
                            "language": Value::Null,
                            "stars": repository["stars"],
                        })
                    })
                    .collect::<Vec<_>>();
                (StatusCode::OK, Json(json!(repositories)))
            }
            Err((status, error)) => (status, Json(json!({"error":error}))),
        };
    }
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    let account =
        match git_accounts::get_git_account_with_token(&state.db, &state.credential_vault, &id) {
            Some(a) => a,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": "Account not found"})),
                );
            }
        };
    if account.user_id.as_deref() != Some(&current_user_id) {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    }

    let client = http_client(15);

    let repos = if account.account_type == "github" {
        let res = client
            .get("https://api.github.com/user/repos?sort=pushed&per_page=40&affiliation=owner,collaborator")
            .bearer_auth(&account.token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await;

        match res {
            Ok(r) if r.status().is_success() => {
                let data: Vec<Value> = r.json().await.unwrap_or_default();
                data.into_iter()
                    .map(|r| {
                        json!({
                            "id": r["id"].as_i64().unwrap_or(0).to_string(),
                            "name": r["name"],
                            "fullName": r["full_name"],
                            "description": r["description"],
                            "cloneUrl": r["clone_url"],
                            "sshUrl": r["ssh_url"],
                            "updatedAt": r["pushed_at"].as_str().unwrap_or(""),
                            "isPrivate": r["private"].as_bool().unwrap_or(false),
                            "defaultBranch": r["default_branch"],
                            "language": r["language"],
                            "stars": r["stargazers_count"].as_i64().unwrap_or(0),
                        })
                    })
                    .collect::<Vec<_>>()
            }
            Ok(r) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": "GitHub API error", "status": r.status().as_u16()})),
                );
            }
            Err(e) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": e.to_string()})),
                );
            }
        }
    } else {
        // /api/v1/user/repos — lists all repos the authenticated user owns or can access
        let url = format!("{}/api/v1/user/repos?limit=50&page=1", account.base_url);
        let res = client
            .get(&url)
            .bearer_auth(&account.token)
            .header("Accept", "application/json")
            .send()
            .await;

        match res {
            Ok(r) if r.status().is_success() => {
                let data: Vec<Value> = r.json().await.unwrap_or_default();
                data.into_iter()
                    .map(|r| {
                        json!({
                            "id": r["id"].as_i64().unwrap_or(0).to_string(),
                            "name": r["name"],
                            "fullName": r["full_name"],
                            "description": r["description"],
                            "cloneUrl": r["clone_url"],
                            "sshUrl": r["ssh_url"],
                            "updatedAt": r["updated"].as_str()
                                .or_else(|| r["updated_at"].as_str())
                                .unwrap_or(""),
                            "isPrivate": r["private"].as_bool().unwrap_or(false),
                            "defaultBranch": r["default_branch"],
                            "language": r["language"],
                            "stars": r["stars_count"].as_i64().unwrap_or(0),
                        })
                    })
                    .collect::<Vec<_>>()
            }
            Ok(r) => {
                let status = r.status().as_u16();
                let body = r.text().await.unwrap_or_default();
                tracing::warn!("Gitea API error {}: {}", status, body);
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": format!("Gitea API error ({}): {}", status, body)})),
                );
            }
            Err(e) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": e.to_string()})),
                );
            }
        }
    };

    (StatusCode::OK, Json(json!(repos)))
}

/// Internal endpoint — returns accounts with real tokens. Only for MCP subprocess use.
pub async fn handle_list_git_accounts_internal(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Value> {
    for user_id in internal_git_credentials_user_candidates(&state, &headers) {
        let accounts = git_accounts::list_git_accounts_with_tokens(
            &state.db,
            &state.credential_vault,
            Some(&user_id),
        );
        if !accounts.is_empty() {
            return Json(serde_json::to_value(accounts).unwrap());
        }
    }

    Json(json!([]))
}

async fn fetch_profile(
    account_type: &str,
    base_url: &str,
    token: &str,
) -> (Option<String>, Option<String>) {
    let client = http_client(5); // short timeout — non-fatal

    let url = if account_type == "github" {
        "https://api.github.com/user".to_string()
    } else {
        format!("{}/api/v1/user", base_url)
    };

    let Ok(res) = client.get(&url).bearer_auth(token).send().await else {
        return (None, None);
    };
    if !res.status().is_success() {
        return (None, None);
    }
    let Ok(profile): Result<Value, _> = res.json().await else {
        return (None, None);
    };

    let username = profile["login"]
        .as_str()
        .or_else(|| profile["full_name"].as_str())
        .map(|s| s.to_string());
    let avatar_url = profile["avatar_url"].as_str().map(|s| s.to_string());

    (username, avatar_url)
}
