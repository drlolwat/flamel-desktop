use crate::{AppState, db::settings, routes::admin::get_current_user_id};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub fn forge_url() -> String {
    std::env::var("FLAMEL_GIT_URL").unwrap_or_else(|_| "https://git.flamel.dev".into())
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .user_agent("Flamel-Server/forge")
        .build()
        .unwrap_or_default()
}

async fn assertion(state: &AppState, _headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    let access = state.service_access.read().await.clone().ok_or((
        StatusCode::UNAUTHORIZED,
        "Sign in to Flamel to use hosted Git".into(),
    ))?;
    access
        .forge_assertion()
        .await
        .map_err(|error| (StatusCode::UNAUTHORIZED, error))
}

pub async fn repositories(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Value, (StatusCode, String)> {
    let assertion = assertion(state, headers).await?;
    let response = client()
        .get(format!("{}/api/v1/auth/server/repositories", forge_url()))
        .bearer_auth(assertion)
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    decode_forge_response(response).await
}

pub async fn create_repository(
    state: &AppState,
    headers: &HeaderMap,
    body: &Value,
) -> Result<Value, (StatusCode, String)> {
    let assertion = assertion(state, headers).await?;
    let response = client()
        .post(format!("{}/api/v1/auth/server/repositories", forge_url()))
        .bearer_auth(assertion)
        .json(body)
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    decode_forge_response(response).await
}

pub async fn repository_token(
    state: &AppState,
    headers: &HeaderMap,
    body: &Value,
) -> Result<Value, (StatusCode, String)> {
    let assertion = assertion(state, headers).await?;
    let response = client()
        .post(format!("{}/api/v1/auth/server/token", forge_url()))
        .bearer_auth(assertion)
        .json(body)
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    decode_forge_response(response).await
}

pub async fn account_tokens(
    state: &AppState,
    headers: &HeaderMap,
    method: reqwest::Method,
    token_id: Option<&str>,
    body: Option<&Value>,
) -> Result<Value, (StatusCode, String)> {
    let assertion = assertion(state, headers).await?;
    let suffix = token_id.map_or_else(String::new, |id| format!("/{id}"));
    let mut request = client()
        .request(
            method,
            format!("{}/api/v1/auth/server/account/tokens{suffix}", forge_url()),
        )
        .bearer_auth(assertion);
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    decode_forge_response(response).await
}

pub async fn link_project(
    state: &AppState,
    headers: &HeaderMap,
    repository_id: &str,
    body: &Value,
) -> Result<Value, (StatusCode, String)> {
    let assertion = assertion(state, headers).await?;
    let response = client()
        .put(format!(
            "{}/api/v1/auth/server/projects/{repository_id}",
            forge_url()
        ))
        .bearer_auth(assertion)
        .json(body)
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    decode_forge_response(response).await
}

async fn decode_forge_response(response: reqwest::Response) -> Result<Value, (StatusCode, String)> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let body = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice::<Value>(&bytes)
            .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?
    };
    if status.is_success() {
        Ok(body)
    } else {
        Err((
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            body.get("error")
                .and_then(Value::as_str)
                .unwrap_or("Flamel Git request failed")
                .to_owned(),
        ))
    }
}

pub(crate) async fn authenticated_forge_request(
    state: &AppState,
    headers: &HeaderMap,
    method: reqwest::Method,
    path: &str,
    payload: Option<&Value>,
) -> Result<Value, (StatusCode, String)> {
    if !path.starts_with("/api/v1/") || path.contains("..") || path.contains('#') {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid internal Forge request path".into(),
        ));
    }
    let assertion = assertion(state, headers).await?;
    let endpoint = format!("{}{}", forge_url().trim_end_matches('/'), path);
    let mut request = client()
        .request(method, endpoint)
        .bearer_auth(assertion)
        .header(header::ACCEPT, "application/json");
    if let Some(payload) = payload {
        request = request.json(payload);
    }
    let response = request
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    decode_forge_response(response).await
}

const MAX_FORGE_STRING_CHARS: usize = 64 * 1024;
const MAX_FORGE_ARRAY_ITEMS: usize = 100;
const MAX_FORGE_OBJECT_KEYS: usize = 64;
const MAX_FORGE_JSON_DEPTH: usize = 8;

struct ForgeOperation {
    method: reqwest::Method,
    suffix: String,
    scopes: &'static [&'static str],
    payload: Option<Value>,
}

fn operation_error(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, message.into())
}

fn required_text<'a>(
    body: &'a Value,
    key: &str,
    max_chars: usize,
) -> Result<&'a str, (StatusCode, String)> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.chars().count() <= max_chars)
        .ok_or_else(|| {
            operation_error(format!(
                "{key} is required and must be at most {max_chars} characters"
            ))
        })
}

fn optional_text(
    body: &Value,
    key: &str,
    max_chars: usize,
) -> Result<Option<String>, (StatusCode, String)> {
    let Some(value) = body.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| operation_error(format!("{key} must be a string")))?;
    if value.chars().count() > max_chars {
        return Err(operation_error(format!(
            "{key} must be at most {max_chars} characters"
        )));
    }
    Ok(Some(value.to_owned()))
}

fn issue_number(body: &Value) -> Result<i64, (StatusCode, String)> {
    body.get("number")
        .and_then(Value::as_i64)
        .filter(|number| (1..=i32::MAX as i64).contains(number))
        .ok_or_else(|| operation_error("number must be a positive 32-bit integer"))
}

fn merge_method(body: &Value) -> Result<String, (StatusCode, String)> {
    let method = body
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("merge");
    if !matches!(method, "merge" | "squash" | "rebase") {
        return Err(operation_error(
            "method must be one of merge, squash, or rebase",
        ));
    }
    Ok(method.to_owned())
}

fn forge_operation(body: &Value) -> Result<ForgeOperation, (StatusCode, String)> {
    let action = required_text(body, "action", 64)?;
    let number_suffix = |tail: &str| -> Result<String, (StatusCode, String)> {
        Ok(format!("{}/{}", issue_number(body)?, tail))
    };
    let read_scopes: &'static [&'static str] = &["repo:read", "issues:read"];
    let issue_scopes: &'static [&'static str] = &["repo:triage", "issues:write"];
    let write_scopes: &'static [&'static str] = &["repo:write", "issues:write"];
    let merge_scopes: &'static [&'static str] = &["repo:maintain", "issues:write"];

    match action {
        "flamel.issue.list" | "flamel.pr.list" => {
            let state = body.get("state").and_then(Value::as_str).unwrap_or("open");
            if !matches!(state, "open" | "closed" | "all") {
                return Err(operation_error("state must be open, closed, or all"));
            }
            let limit = body
                .get("limit")
                .and_then(Value::as_i64)
                .unwrap_or(50)
                .clamp(1, MAX_FORGE_ARRAY_ITEMS as i64);
            let offset = body
                .get("offset")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .clamp(0, 1_000_000);
            let resource = if action == "flamel.issue.list" {
                "issues"
            } else {
                "pulls"
            };
            let state_query = if state == "all" {
                String::new()
            } else {
                format!("&state={state}")
            };
            Ok(ForgeOperation {
                method: reqwest::Method::GET,
                suffix: format!("{resource}?limit={limit}&offset={offset}{state_query}"),
                scopes: read_scopes,
                payload: None,
            })
        }
        "flamel.issue.view" => Ok(ForgeOperation {
            method: reqwest::Method::GET,
            suffix: format!("issues/{}", issue_number(body)?),
            scopes: read_scopes,
            payload: None,
        }),
        "flamel.issue.create" => Ok(ForgeOperation {
            method: reqwest::Method::POST,
            suffix: "issues".into(),
            scopes: issue_scopes,
            payload: Some(json!({
                "title": required_text(body, "title", 256)?,
                "body": optional_text(body, "body", MAX_FORGE_STRING_CHARS)?.unwrap_or_default(),
                "kind": "issue",
            })),
        }),
        "flamel.issue.update" => {
            let title = optional_text(body, "title", 256)?;
            let description = optional_text(body, "body", MAX_FORGE_STRING_CHARS)?;
            let state = optional_text(body, "state", 16)?;
            if let Some(state) = state.as_deref()
                && !matches!(state, "open" | "closed")
            {
                return Err(operation_error("state must be open or closed"));
            }
            if title.is_none() && description.is_none() && state.is_none() {
                return Err(operation_error(
                    "at least one of title, body, or state is required",
                ));
            }
            let mut payload = json!({});
            if let Some(title) = title {
                payload["title"] = Value::String(title);
            }
            if let Some(description) = description {
                payload["body"] = Value::String(description);
            }
            if let Some(state) = state {
                payload["state"] = Value::String(state);
            }
            Ok(ForgeOperation {
                method: reqwest::Method::PATCH,
                suffix: format!("issues/{}", issue_number(body)?),
                scopes: issue_scopes,
                payload: Some(payload),
            })
        }
        "flamel.issue.comment" => Ok(ForgeOperation {
            method: reqwest::Method::POST,
            suffix: number_suffix("comments").map(|value| format!("issues/{value}"))?,
            scopes: issue_scopes,
            payload: Some(json!({
                "body": required_text(body, "body", MAX_FORGE_STRING_CHARS)?,
            })),
        }),
        "flamel.pr.view" | "flamel.pr.status" => Ok(ForgeOperation {
            method: reqwest::Method::GET,
            suffix: format!("pulls/{}", issue_number(body)?),
            scopes: read_scopes,
            payload: None,
        }),
        "flamel.pr.create" => Ok(ForgeOperation {
            method: reqwest::Method::POST,
            suffix: "pulls".into(),
            scopes: write_scopes,
            payload: Some(json!({
                "title": required_text(body, "title", 256)?,
                "body": optional_text(body, "body", MAX_FORGE_STRING_CHARS)?.unwrap_or_default(),
                "head_branch": required_text(body, "head", 255)?,
                "base_branch": optional_text(body, "base", 255)?,
                "draft": body.get("draft").and_then(Value::as_bool).unwrap_or(false),
            })),
        }),
        "flamel.pr.review" => {
            let state = required_text(body, "review_state", 32)?;
            if !matches!(state, "commented" | "approved" | "changes_requested") {
                return Err(operation_error(
                    "review_state must be commented, approved, or changes_requested",
                ));
            }
            Ok(ForgeOperation {
                method: reqwest::Method::POST,
                suffix: number_suffix("reviews").map(|value| format!("pulls/{value}"))?,
                scopes: read_scopes,
                payload: Some(json!({
                    "state": state,
                    "body": optional_text(body, "body", MAX_FORGE_STRING_CHARS)?.unwrap_or_default(),
                    "commit_sha": optional_text(body, "commit_sha", 64)?,
                })),
            })
        }
        "flamel.pr.comment" => {
            let line = body
                .get("line")
                .and_then(Value::as_i64)
                .filter(|line| (1..=i32::MAX as i64).contains(line))
                .ok_or_else(|| operation_error("line must be a positive 32-bit integer"))?;
            let side = body
                .get("side")
                .and_then(Value::as_str)
                .unwrap_or("RIGHT")
                .to_ascii_uppercase();
            if !matches!(side.as_str(), "LEFT" | "RIGHT") {
                return Err(operation_error("side must be LEFT or RIGHT"));
            }
            Ok(ForgeOperation {
                method: reqwest::Method::POST,
                suffix: number_suffix("review-comments").map(|value| format!("pulls/{value}"))?,
                scopes: read_scopes,
                payload: Some(json!({
                    "path": required_text(body, "file", 1024)?,
                    "line": line,
                    "side": side,
                    "body": required_text(body, "body", MAX_FORGE_STRING_CHARS)?,
                })),
            })
        }
        "flamel.pr.merge_queue" => Ok(ForgeOperation {
            method: reqwest::Method::POST,
            suffix: number_suffix("merge-queue").map(|value| format!("pulls/{value}"))?,
            scopes: merge_scopes,
            payload: Some(json!({"method": merge_method(body)?})),
        }),
        "flamel.pr.merge" => Ok(ForgeOperation {
            method: reqwest::Method::POST,
            suffix: number_suffix("merge").map(|value| format!("pulls/{value}"))?,
            scopes: merge_scopes,
            payload: Some(json!({"method": merge_method(body)?})),
        }),
        _ => Err(operation_error("unsupported Flamel Git operation")),
    }
}

fn bound_forge_json(value: Value, depth: usize) -> Value {
    if depth >= MAX_FORGE_JSON_DEPTH {
        return Value::Null;
    }
    match value {
        Value::String(value) if value.chars().count() > MAX_FORGE_STRING_CHARS => {
            let mut bounded = value
                .chars()
                .take(MAX_FORGE_STRING_CHARS)
                .collect::<String>();
            bounded.push_str("\n… [truncated by Flamel]");
            Value::String(bounded)
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .take(MAX_FORGE_ARRAY_ITEMS)
                .map(|value| bound_forge_json(value, depth + 1))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .take(MAX_FORGE_OBJECT_KEYS)
                .map(|(key, value)| (key, bound_forge_json(value, depth + 1)))
                .collect(),
        ),
        value => value,
    }
}

fn normalize_forge_operation_response(
    action: &str,
    body: Value,
) -> Result<Value, (StatusCode, String)> {
    let malformed = || {
        (
            StatusCode::BAD_GATEWAY,
            "Flamel Git returned a malformed operation response".to_owned(),
        )
    };
    let valid = match action {
        "flamel.issue.list" | "flamel.pr.list" => {
            body.get("issues").and_then(Value::as_array).is_some()
        }
        "flamel.issue.view" => body.get("issue").and_then(Value::as_object).is_some(),
        "flamel.issue.create" | "flamel.issue.update" => {
            body.get("issue").and_then(Value::as_object).is_some()
        }
        "flamel.issue.comment" | "flamel.pr.review" | "flamel.pr.comment" => {
            body.get("id").and_then(Value::as_str).is_some()
        }
        "flamel.pr.create" => body
            .get("pull_request")
            .and_then(Value::as_object)
            .is_some(),
        "flamel.pr.view" | "flamel.pr.status" => {
            body.get("issue").and_then(Value::as_object).is_some()
                && body.get("pull").and_then(Value::as_object).is_some()
                && body.get("checks").and_then(Value::as_array).is_some()
        }
        "flamel.pr.merge_queue" => {
            body.get("id").and_then(Value::as_str).is_some()
                && body.get("status").and_then(Value::as_str) == Some("queued")
        }
        "flamel.pr.merge" => {
            body.get("merged").and_then(Value::as_bool) == Some(true)
                && body.get("sha").and_then(Value::as_str).is_some()
        }
        _ => false,
    };
    if !valid {
        return Err(malformed());
    }
    if action == "flamel.pr.status" {
        return Ok(bound_forge_json(
            json!({
                "issue": body.get("issue"),
                "pull": body.get("pull"),
                "merge_queue": body.get("merge_queue"),
                "checks": body.get("checks"),
            }),
            0,
        ));
    }
    Ok(bound_forge_json(body, 0))
}

fn valid_repository_component(value: &str) -> bool {
    !value.contains('/') && !matches!(value, "." | "..")
}

fn normalize_repository_catalog(body: Value) -> Result<Value, (StatusCode, String)> {
    let repositories = body
        .get("repositories")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                "Flamel Git returned a malformed repository catalog".to_owned(),
            )
        })?;
    let mut normalized = Vec::with_capacity(repositories.len().min(MAX_FORGE_ARRAY_ITEMS));
    for repository in repositories.iter().take(MAX_FORGE_ARRAY_ITEMS) {
        let required_string = |key: &str| {
            repository
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_GATEWAY,
                        format!("Flamel Git repository catalog entry is missing {key}"),
                    )
                })
        };
        let id = required_string("id")?;
        let owner = required_string("owner")?;
        let name = required_string("name")?;
        let full_name = required_string("full_name")?;
        if full_name != format!("{owner}/{name}") {
            return Err((
                StatusCode::BAD_GATEWAY,
                "Flamel Git repository catalog entry has inconsistent identity".to_owned(),
            ));
        }
        let permissions = repository
            .get("permissions")
            .and_then(Value::as_object)
            .filter(|permissions| {
                permissions.get("access").and_then(Value::as_str).is_some()
                    && permissions.get("read").and_then(Value::as_bool).is_some()
                    && permissions.get("write").and_then(Value::as_bool).is_some()
                    && permissions
                        .get("maintain")
                        .and_then(Value::as_bool)
                        .is_some()
                    && permissions.get("admin").and_then(Value::as_bool).is_some()
            })
            .ok_or_else(|| {
                (
                    StatusCode::BAD_GATEWAY,
                    "Flamel Git repository catalog entry has invalid permissions".to_owned(),
                )
            })?;
        normalized.push(json!({
            "id": id,
            "owner": owner,
            "name": name,
            "full_name": full_name,
            "description": repository.get("description").cloned().unwrap_or(Value::Null),
            "clone_url": required_string("clone_url")?,
            "web_url": required_string("web_url")?,
            "private": repository.get("private").and_then(Value::as_bool).unwrap_or(false),
            "default_branch": required_string("default_branch")?,
            "updated_at": repository.get("updated_at").cloned().unwrap_or(Value::Null),
            "permissions": permissions,
        }));
    }
    Ok(bound_forge_json(
        json!({
            "user": body.get("user").cloned().unwrap_or(Value::Null),
            "repositories": normalized,
        }),
        0,
    ))
}

fn repository_id_from_catalog<'a>(
    catalog: &'a Value,
    owner: &str,
    repository_name: &str,
) -> Option<&'a str> {
    let full_name = format!("{owner}/{repository_name}");
    catalog
        .get("repositories")
        .and_then(Value::as_array)
        .and_then(|repositories| {
            repositories.iter().find_map(|repository| {
                (repository.get("full_name").and_then(Value::as_str) == Some(full_name.as_str()))
                    .then(|| repository.get("id").and_then(Value::as_str))
                    .flatten()
            })
        })
}

async fn repository_operation(
    state: &AppState,
    headers: &HeaderMap,
    body: &Value,
) -> Result<Value, (StatusCode, String)> {
    let action = required_text(body, "action", 64)?;
    let owner = required_text(body, "owner", 128)?;
    let repository_name = required_text(body, "repo", 128)?;
    if !valid_repository_component(owner) || !valid_repository_component(repository_name) {
        return Err(operation_error("invalid repository owner or name"));
    }
    let operation = forge_operation(body)?;
    let listed = repositories(state, headers).await?;
    let repository_id =
        repository_id_from_catalog(&listed, owner, repository_name).ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Flamel Git repository is not available to this user".to_owned(),
            )
        })?;
    let token_response = repository_token(
        state,
        headers,
        &json!({"repository_id":repository_id,"scopes":operation.scopes}),
    )
    .await?;
    let token = token_response
        .get("token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                "Flamel Git returned an invalid repository credential".to_owned(),
            )
        })?;
    let endpoint = format!(
        "{}/api/v1/repos/{}/{}/{}",
        forge_url().trim_end_matches('/'),
        urlencoding::encode(owner),
        urlencoding::encode(repository_name),
        operation.suffix
    );
    let mut request = client()
        .request(operation.method, endpoint)
        .bearer_auth(token)
        .header(header::ACCEPT, "application/json");
    if let Some(payload) = operation.payload.as_ref() {
        request = request.json(payload);
    }
    let response = request
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let response = decode_forge_response(response).await?;
    normalize_forge_operation_response(action, response)
}

pub async fn handle_repository_operation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    match repository_operation(&state, &headers, &body).await {
        Ok(body) => (StatusCode::OK, Json(body)),
        Err((status, error)) => (status, Json(json!({"error":error}))),
    }
}

pub async fn handle_list_repositories(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    match repositories(&state, &headers)
        .await
        .and_then(normalize_repository_catalog)
    {
        Ok(body) => (StatusCode::OK, Json(body)),
        Err((status, error)) => (status, Json(json!({"error":error}))),
    }
}

pub async fn handle_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let user_id = get_current_user_id(&state, &headers);
    if user_id.is_none() {
        return (StatusCode::OK, Json(Value::Null));
    }
    (
        StatusCode::OK,
        Json(json!({
            "id":"flamel-git",
            "type":"flamel",
            "label":"Flamel Git",
            "baseUrl":forge_url(),
            "token":"***",
            "createdAt":"",
            "username":Value::Null,
            "avatarUrl":Value::Null,
        })),
    )
}

pub async fn handle_repository_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    match repository_token(&state, &headers, &body).await {
        Ok(body) => (StatusCode::CREATED, Json(body)),
        Err((status, error)) => (status, Json(json!({"error":error}))),
    }
}

pub async fn handle_repository_create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    match create_repository(&state, &headers, &body).await {
        Ok(body) => (StatusCode::CREATED, Json(body)),
        Err((status, error)) => (status, Json(json!({"error":error}))),
    }
}

pub async fn handle_account_tokens_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    match account_tokens(&state, &headers, reqwest::Method::GET, None, None).await {
        Ok(body) => (StatusCode::OK, Json(body)),
        Err((status, error)) => (status, Json(json!({"error":error}))),
    }
}

pub async fn handle_account_token_create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    match account_tokens(&state, &headers, reqwest::Method::POST, None, Some(&body)).await {
        Ok(body) => (StatusCode::CREATED, Json(body)),
        Err((status, error)) => (status, Json(json!({"error":error}))),
    }
}

pub async fn handle_account_token_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    match account_tokens(&state, &headers, reqwest::Method::DELETE, Some(&id), None).await {
        Ok(body) => (StatusCode::OK, Json(body)),
        Err((status, error)) => (status, Json(json!({"error":error}))),
    }
}

pub async fn handle_remote_credentials(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let Some(remote_url) = body.get("remote_url").and_then(Value::as_str) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"remote_url is required"})),
        );
    };
    let write = body.get("write").and_then(Value::as_bool).unwrap_or(false);
    let admin = body.get("admin").and_then(Value::as_bool).unwrap_or(false);
    let listed = match repositories(&state, &headers).await {
        Ok(value) => value,
        Err((status, error)) => return (status, Json(json!({"error":error}))),
    };
    let normalized = remote_url.trim_end_matches('/').trim_end_matches(".git");
    let repository_id = listed
        .get("repositories")
        .and_then(Value::as_array)
        .and_then(|repositories| {
            repositories.iter().find_map(|repository| {
                let clone_url = repository.get("clone_url")?.as_str()?;
                (clone_url.trim_end_matches('/').trim_end_matches(".git") == normalized)
                    .then(|| {
                        repository
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .flatten()
            })
        });
    let Some(repository_id) = repository_id else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Flamel Git repository is not available to this user"})),
        );
    };
    let scopes = if admin {
        json!(["repo:admin", "repo:write", "issues:write", "actions:write"])
    } else if write {
        json!(["repo:write", "issues:write", "actions:write"])
    } else {
        json!(["repo:read", "issues:read", "actions:read"])
    };
    match repository_token(
        &state,
        &headers,
        &json!({"repository_id":repository_id,"scopes":scopes}),
    )
    .await
    {
        Ok(value) => (StatusCode::CREATED, Json(value)),
        Err((status, error)) => (status, Json(json!({"error":error}))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(action: &str) -> Value {
        json!({
            "action": action,
            "owner": "flamel",
            "repo": "core",
            "number": 7,
            "title": "Typed operation",
            "body": "Body",
            "head": "feature/typed-operations",
            "review_state": "approved",
            "file": "src/lib.rs",
            "line": 12,
        })
    }

    #[test]
    fn repository_catalog_handles_empty_accounts_and_filters_by_exact_identity() {
        let empty = normalize_repository_catalog(json!({
            "user": {"id":"user"},
            "repositories": [],
        }))
        .unwrap();
        assert_eq!(
            empty
                .get("repositories")
                .and_then(Value::as_array)
                .unwrap()
                .len(),
            0
        );

        let catalog = normalize_repository_catalog(json!({
            "user": {"id":"user"},
            "repositories": [{
                "id": "repo-id",
                "owner": "flamel",
                "name": "core",
                "full_name": "flamel/core",
                "description": null,
                "clone_url": "https://git.flamel.dev/flamel/core.git",
                "web_url": "https://git.flamel.dev/flamel/core",
                "private": true,
                "default_branch": "main",
                "updated_at": null,
                "permissions": {
                    "access": "write",
                    "read": true,
                    "triage": true,
                    "write": true,
                    "maintain": false,
                    "admin": false,
                },
            }],
        }))
        .unwrap();
        assert_eq!(
            repository_id_from_catalog(&catalog, "flamel", "core"),
            Some("repo-id")
        );
        assert_eq!(
            repository_id_from_catalog(&catalog, "flamel", "hidden"),
            None
        );
        assert_eq!(
            catalog.pointer("/repositories/0/permissions/write"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn repository_catalog_rejects_malformed_or_ambiguous_permissions() {
        assert!(normalize_repository_catalog(json!({"items":[]})).is_err());
        assert!(
            normalize_repository_catalog(json!({
                "repositories":[{
                    "id":"repo-id",
                    "owner":"flamel",
                    "name":"core",
                    "full_name":"flamel/core",
                    "clone_url":"https://git.flamel.dev/flamel/core.git",
                    "web_url":"https://git.flamel.dev/flamel/core",
                    "default_branch":"main",
                    "permissions":{"access":"write","read":true}
                }]
            }))
            .is_err()
        );
    }

    #[test]
    fn operation_contracts_use_minimum_repository_scopes() {
        let read = forge_operation(&request("flamel.issue.view")).unwrap();
        assert_eq!(read.scopes, ["repo:read", "issues:read"]);

        let issue = forge_operation(&request("flamel.issue.create")).unwrap();
        assert_eq!(issue.scopes, ["repo:triage", "issues:write"]);

        let write = forge_operation(&request("flamel.pr.create")).unwrap();
        assert_eq!(write.scopes, ["repo:write", "issues:write"]);

        let review = forge_operation(&request("flamel.pr.review")).unwrap();
        assert_eq!(review.scopes, ["repo:read", "issues:read"]);

        let mut merge_request = request("flamel.pr.merge");
        merge_request["bypass"] = json!(true);
        merge_request["force"] = json!(true);
        let merge = forge_operation(&merge_request).unwrap();
        assert_eq!(merge.scopes, ["repo:maintain", "issues:write"]);
        assert!(!merge.scopes.contains(&"repo:admin"));
        let payload = merge.payload.unwrap();
        assert_eq!(payload, json!({"method":"merge"}));
        assert!(payload.get("bypass").is_none());
        assert!(payload.get("force").is_none());
    }

    #[test]
    fn list_contract_is_bounded_and_state_is_validated() {
        let mut body = request("flamel.issue.list");
        body["limit"] = json!(5000);
        body["offset"] = json!(25);
        body["state"] = json!("all");
        let operation = forge_operation(&body).unwrap();
        assert_eq!(operation.suffix, "issues?limit=100&offset=25");

        body["state"] = json!("deleted");
        assert!(forge_operation(&body).is_err());
    }

    #[test]
    fn operation_contract_rejects_invalid_mutations() {
        let mut merge = request("flamel.pr.merge");
        merge["method"] = json!("force");
        assert!(forge_operation(&merge).is_err());

        let mut update = request("flamel.issue.update");
        update.as_object_mut().unwrap().remove("title");
        update.as_object_mut().unwrap().remove("body");
        update.as_object_mut().unwrap().remove("state");
        assert!(forge_operation(&update).is_err());

        assert!(forge_operation(&request("flamel.raw.request")).is_err());
    }

    #[test]
    fn repository_components_reject_traversal_and_embedded_paths() {
        for value in [".", "..", "../admin", "owner/repo"] {
            assert!(!valid_repository_component(value));
        }
        assert!(valid_repository_component("flamel-core"));
    }

    #[test]
    fn forge_responses_fail_closed_and_bound_large_payloads() {
        let malformed =
            normalize_forge_operation_response("flamel.issue.list", json!({"items":[]}))
                .unwrap_err();
        assert_eq!(malformed.0, StatusCode::BAD_GATEWAY);

        let issues = (0..150)
            .map(|number| json!({"number":number,"title":"item"}))
            .collect::<Vec<_>>();
        let bounded =
            normalize_forge_operation_response("flamel.issue.list", json!({"issues":issues}))
                .unwrap();
        assert_eq!(
            bounded
                .get("issues")
                .and_then(Value::as_array)
                .unwrap()
                .len(),
            MAX_FORGE_ARRAY_ITEMS
        );

        let large_patch = "x".repeat(MAX_FORGE_STRING_CHARS + 100);
        let pull = normalize_forge_operation_response(
            "flamel.pr.view",
            json!({
                "issue": {},
                "pull": {},
                "checks": [],
                "patch": large_patch,
            }),
        )
        .unwrap();
        let patch = pull.get("patch").and_then(Value::as_str).unwrap();
        assert!(patch.chars().count() < MAX_FORGE_STRING_CHARS + 100);
        assert!(patch.contains("truncated by Flamel"));
    }

    #[test]
    fn pull_status_omits_patch_and_review_bodies() {
        let status = normalize_forge_operation_response(
            "flamel.pr.status",
            json!({
                "issue": {"number": 7},
                "pull": {"head_sha": "abc"},
                "checks": [],
                "merge_queue": null,
                "patch": "sensitive diff",
                "reviews": [{"body":"long review"}],
            }),
        )
        .unwrap();
        assert!(status.get("patch").is_none());
        assert!(status.get("reviews").is_none());
        assert_eq!(
            status.pointer("/issue/number").and_then(Value::as_i64),
            Some(7)
        );
    }
}
