use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::capability_client::CapabilityResult;
use crate::db::{capabilities, capability_packs, skill_catalog};
use crate::routes::admin::{get_current_user_id, get_user_home, require_admin};

#[derive(Deserialize)]
pub struct CapabilityQuery {
    #[serde(rename = "projectPath")]
    pub(crate) project_path: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteRequest {
    tool_name: String,
    #[serde(default)]
    arguments: Value,
    session_id: Option<String>,
    #[serde(default)]
    background: bool,
}

fn user(state: &AppState, headers: &HeaderMap) -> Result<String, (StatusCode, Json<Value>)> {
    get_current_user_id(state, headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Unauthorized"})),
        )
    })
}

fn runtime_assistant_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-flamel-assistant-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| uuid::Uuid::parse_str(value).is_ok())
        .map(str::to_owned)
}

async fn resolved_credential(
    state: &AppState,
    user_id: &str,
    entry: &capabilities::CapabilityCatalogEntry,
    cancellation: &CancellationToken,
) -> Result<Option<String>, String> {
    let Some(stored) = crate::db::credentials::record(&state.db, user_id, &entry.id) else {
        return Ok(None);
    };
    let decrypted = state.credential_vault.decrypt(&stored.encrypted_value)?;
    if stored.kind != "oauth2" {
        return Ok(Some(decrypted));
    }
    let resolved =
        crate::developer_packs::refresh_credential_if_needed(&decrypted, cancellation).await?;
    if resolved.refreshed {
        let encrypted = state.credential_vault.encrypt(&resolved.value)?;
        let reference = crate::db::credentials::store(
            &state.db,
            user_id,
            &entry.id,
            &stored.kind,
            &encrypted,
            &stored.metadata,
            resolved
                .expires_at
                .as_deref()
                .or(stored.expires_at.as_deref()),
        )?;
        capabilities::record_audit(
            &state.db,
            user_id,
            Some(&entry.id),
            entry.connection_id.as_deref(),
            None,
            "credential_refresh",
            "allow",
            None,
            "OAuth access token refreshed and rotated in the credential vault",
            &json!({"credentialRef":reference}),
        );
    }
    Ok(Some(resolved.value))
}

pub async fn handle_list_catalog(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<CapabilityQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    (
        StatusCode::OK,
        Json(json!({
            "capabilities": capabilities::list_catalog(
                &state.db,
                &user_id,
                query.project_path.as_deref(),
            ),
            "adminManaged": true,
            "autonomy": "maximum",
        })),
    )
}

pub async fn handle_create_catalog_entry(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = require_admin(&state, &headers) {
        return response;
    }
    match capabilities::create_catalog_entry(&state.db, &body) {
        Ok(entry) => (StatusCode::CREATED, Json(json!(entry))),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error": error}))),
    }
}

pub async fn handle_connect(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<CapabilityQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match capabilities::connect(&state.db, &id, &user_id, query.project_path.as_deref()) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"ok": true, "status": "configured"})),
        ),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error": error}))),
    }
}

pub async fn handle_disconnect(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<CapabilityQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    capabilities::disconnect(&state.db, &id, &user_id, query.project_path.as_deref());
    let snapshots = refresh_skill_snapshots(&state, &user_id, query.project_path.as_deref());
    (
        StatusCode::OK,
        Json(json!({"ok": true,"skillSnapshots":snapshots})),
    )
}

async fn run_entry(
    entry: &capabilities::CapabilityCatalogEntry,
    call: Option<(&str, &Value)>,
    bearer: Option<&str>,
    cancellation: &CancellationToken,
) -> Result<CapabilityResult, String> {
    if entry.builtin {
        return Ok(CapabilityResult {
            tools: vec![],
            resources: vec![],
            prompts: vec![],
            output: None,
            latency_ms: 0,
            process_id: None,
        });
    }
    if entry.transport == "stdio" {
        crate::capability_client::run_stdio(
            entry
                .command
                .as_deref()
                .ok_or("Integration command is missing")?,
            &entry.args,
            call,
            cancellation,
        )
        .await
    } else if entry.transport == "native" {
        crate::developer_packs::run_native(
            entry
                .native_adapter
                .as_deref()
                .ok_or("Signed native pack is missing its reviewed adapter id")?,
            call,
            bearer,
            cancellation,
        )
        .await
    } else {
        let token = bearer
            .map(crate::developer_packs::bearer_token)
            .transpose()?;
        crate::capability_client::run_remote(
            entry
                .remote_url
                .as_deref()
                .ok_or("Integration URL is missing")?,
            call,
            token.as_deref(),
            cancellation,
        )
        .await
    }
}

fn is_authentication_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("authentication required")
        || error.contains("unauthorized")
        || error.contains("forbidden")
        || error.contains("status 401")
        || error.contains("status 403")
        || error.contains("http 401")
        || error.contains("http 403")
}

fn is_recoverable_transport_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    !is_authentication_error(&error)
        && (error.contains("transport closed")
            || error.contains("connection closed")
            || error.contains("connection reset")
            || error.contains("broken pipe")
            || error.contains("could not start integration")
            || error.contains("mcp initialization failed"))
}

pub async fn handle_test_connection(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<CapabilityQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some(entry) =
        capabilities::list_catalog(&state.db, &user_id, query.project_path.as_deref())
            .into_iter()
            .find(|entry| entry.id == id)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Integration not found"})),
        );
    };
    capabilities::reset_circuit(&state.db, &id, &user_id, query.project_path.as_deref());
    let cancellation = CancellationToken::new();
    let credential = match resolved_credential(&state, &user_id, &entry, &cancellation).await {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok":false,"status":"needs_auth","error":error})),
            );
        }
    };
    let result = run_entry(&entry, None, credential.as_deref(), &cancellation).await;
    match result {
        Ok(probe) => {
            capabilities::upsert_discovered_tools(&state.db, &id, &probe.tools);
            capabilities::upsert_discovered_surfaces(
                &state.db,
                &id,
                &probe.resources,
                &probe.prompts,
            );
            capabilities::record_health(
                &state.db,
                &id,
                &user_id,
                query.project_path.as_deref(),
                true,
                probe.latency_ms,
                probe.tools.len() as i64,
                probe.process_id,
                None,
            );
            capabilities::record_audit(
                &state.db,
                &user_id,
                Some(&id),
                entry.connection_id.as_deref(),
                None,
                "connection_test",
                "allow",
                None,
                "Connection test succeeded",
                &json!({"latencyMs":probe.latency_ms,"toolCount":probe.tools.len(),"resourceCount":probe.resources.len(),"promptCount":probe.prompts.len()}),
            );
            (
                StatusCode::OK,
                Json(
                    json!({"ok": true, "status": "ready", "latencyMs": probe.latency_ms, "toolCount": probe.tools.len(), "resourceCount":probe.resources.len(), "promptCount":probe.prompts.len()}),
                ),
            )
        }
        Err(error) => {
            let status = if is_authentication_error(&error) {
                "needs_auth"
            } else {
                "failed"
            };
            capabilities::record_health(
                &state.db,
                &id,
                &user_id,
                query.project_path.as_deref(),
                false,
                0,
                0,
                None,
                Some(&error),
            );
            capabilities::record_audit(
                &state.db,
                &user_id,
                Some(&id),
                entry.connection_id.as_deref(),
                None,
                "connection_test",
                "error",
                None,
                "Connection test failed",
                &json!({"error":error}),
            );
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok": false, "status": status, "error": error})),
            )
        }
    }
}

pub async fn handle_list_tools(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = user(&state, &headers) {
        return response;
    }
    (
        StatusCode::OK,
        Json(json!({"tools": capabilities::list_tools(&state.db, &id, false)})),
    )
}

pub async fn handle_list_surfaces(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = user(&state, &headers) {
        return response;
    }
    (
        StatusCode::OK,
        Json(json!({"surfaces": capabilities::list_surfaces(&state.db, &id)})),
    )
}

pub async fn handle_set_tool(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((id, tool)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = require_admin(&state, &headers) {
        return response;
    }
    let enabled = body.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    if capabilities::set_tool_enabled(&state.db, &id, &tool, enabled) {
        (StatusCode::OK, Json(json!({"ok":true,"enabled":enabled})))
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Tool not found"})),
        )
    }
}

struct ExecutedCapability {
    response: Value,
    retained: Value,
}

fn payload_is_ephemeral(db: &crate::db::Db, capability_id: &str, tool_name: &str) -> bool {
    capabilities::list_tools(db, capability_id, false)
        .into_iter()
        .find(|tool| tool.name == tool_name)
        .and_then(|tool| {
            tool.annotations
                .get("payloadRetention")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .as_deref()
        == Some("ephemeral")
}

fn redacted_task_payload(tool_name: &str, phase: &str, value: &Value) -> Value {
    let fields = value
        .as_object()
        .map(|object| object.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    json!({
        "redacted":true,
        "retention":"ephemeral",
        "toolName":tool_name,
        "phase":phase,
        "fields":fields,
        "summary":if phase == "output" {
            Some("Personal capability output was returned for the current task")
        } else {
            None
        },
    })
}

async fn execute_capability(
    state: Arc<AppState>,
    entry: capabilities::CapabilityCatalogEntry,
    user_id: String,
    project_path: Option<String>,
    assistant_id: Option<String>,
    request: ExecuteRequest,
    task_id: String,
    cancellation: CancellationToken,
    ephemeral_payload: bool,
) -> Result<ExecutedCapability, String> {
    capabilities::policy_decision_for_assistant(
        &state.db,
        &entry.id,
        &request.tool_name,
        &user_id,
        project_path.as_deref(),
        assistant_id.as_deref(),
    )?;
    if entry.circuit_state == "open" {
        return Err("Connection circuit is open. Test the connection to retry.".into());
    }
    if entry.pack_id.is_some()
        && !capabilities::connection_has_exact_scopes(
            &state.db,
            &entry.id,
            &user_id,
            project_path.as_deref(),
        )
    {
        return Err("Connected pack scopes do not match the signed manifest".into());
    }
    let enabled = capabilities::list_tools(&state.db, &entry.id, true);
    let advertised = enabled.iter().any(|tool| tool.name == request.tool_name);
    if (entry.pack_id.is_some() && !advertised) || (!enabled.is_empty() && !advertised) {
        return Err("Tool is disabled or was not advertised by this integration".into());
    }
    if entry.pack_id.is_some() {
        capabilities::validate_tool_arguments(
            &state.db,
            &entry.id,
            &request.tool_name,
            &request.arguments,
        )?;
    }
    let started = Instant::now();
    let credential = resolved_credential(&state, &user_id, &entry, &cancellation).await?;
    let mut attempt = 0_u8;
    let result = loop {
        attempt += 1;
        let result = run_entry(
            &entry,
            Some((&request.tool_name, &request.arguments)),
            credential.as_deref(),
            &cancellation,
        )
        .await;
        let should_retry = result
            .as_ref()
            .err()
            .is_some_and(|error| is_recoverable_transport_error(error))
            && attempt < 3
            && !cancellation.is_cancelled();
        if !should_retry {
            break result;
        }
        capabilities::record_restart(&state.db, &entry.id, &user_id, project_path.as_deref());
        capabilities::record_audit(
            &state.db,
            &user_id,
            Some(&entry.id),
            entry.connection_id.as_deref(),
            request.session_id.as_deref(),
            "connector_restart",
            "retry",
            Some(&request.tool_name),
            "Transient connector failure; retrying",
            &json!({"attempt":attempt,"taskId":task_id}),
        );
        tokio::select! {
            _ = cancellation.cancelled() => break Err("Capability task cancelled".into()),
            _ = tokio::time::sleep(Duration::from_millis(150 * u64::from(attempt))) => {}
        }
    };
    let duration = started.elapsed().as_millis() as i64;
    match result {
        Ok(probe) => {
            capabilities::upsert_discovered_tools(&state.db, &entry.id, &probe.tools);
            capabilities::upsert_discovered_surfaces(
                &state.db,
                &entry.id,
                &probe.resources,
                &probe.prompts,
            );
            capabilities::record_health(
                &state.db,
                &entry.id,
                &user_id,
                project_path.as_deref(),
                true,
                duration,
                probe.tools.len() as i64,
                probe.process_id,
                None,
            );
            capabilities::record_invocation(
                &state.db,
                &entry.id,
                entry.connection_id.as_deref(),
                &user_id,
                request.session_id.as_deref(),
                &request.tool_name,
                "success",
                duration,
                None,
            );
            capabilities::record_audit(
                &state.db,
                &user_id,
                Some(&entry.id),
                entry.connection_id.as_deref(),
                request.session_id.as_deref(),
                "tool_call",
                "allow",
                Some(&request.tool_name),
                "Capability executed",
                &json!({"durationMs":duration,"taskId":task_id,"attempts":attempt}),
            );
            let response = probe.output.unwrap_or_else(|| json!({}));
            let retained = if ephemeral_payload {
                redacted_task_payload(&request.tool_name, "output", &response)
            } else {
                response.clone()
            };
            Ok(ExecutedCapability { response, retained })
        }
        Err(error) => {
            let retained_error = if ephemeral_payload {
                "Personal capability request failed; details were not retained"
            } else {
                &error
            };
            capabilities::record_health(
                &state.db,
                &entry.id,
                &user_id,
                project_path.as_deref(),
                false,
                duration,
                entry.tool_count,
                None,
                Some(retained_error),
            );
            capabilities::record_invocation(
                &state.db,
                &entry.id,
                entry.connection_id.as_deref(),
                &user_id,
                request.session_id.as_deref(),
                &request.tool_name,
                "error",
                duration,
                Some("execution_failed"),
            );
            capabilities::record_audit(
                &state.db,
                &user_id,
                Some(&entry.id),
                entry.connection_id.as_deref(),
                request.session_id.as_deref(),
                "tool_call",
                "error",
                Some(&request.tool_name),
                "Capability execution failed",
                &json!({
                    "durationMs":duration,
                    "taskId":task_id,
                    "error":if ephemeral_payload {"redacted"} else {error.as_str()},
                    "attempts":attempt,
                    "payloadRetention":if ephemeral_payload {"ephemeral"} else {"task"},
                }),
            );
            Err(error)
        }
    }
}

pub async fn handle_execute(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<CapabilityQuery>,
    Json(request): Json<ExecuteRequest>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let assistant_id = runtime_assistant_id(&headers);
    let Some(entry) =
        capabilities::list_catalog(&state.db, &user_id, query.project_path.as_deref())
            .into_iter()
            .find(|entry| entry.id == id && entry.connected && entry.enabled)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Connected integration not found"})),
        );
    };
    if entry.builtin {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Built-in tools are called directly"})),
        );
    }
    let ephemeral_payload = payload_is_ephemeral(&state.db, &entry.id, &request.tool_name);
    let retained_input = if ephemeral_payload {
        redacted_task_payload(&request.tool_name, "input", &request.arguments)
    } else {
        request.arguments.clone()
    };
    let task = capabilities::create_task(
        &state.db,
        &entry.id,
        entry.connection_id.as_deref(),
        &user_id,
        request.session_id.as_deref(),
        &request.tool_name,
        &retained_input,
    );
    let cancellation = CancellationToken::new();
    state
        .capability_cancellations
        .lock()
        .await
        .insert(task.id.clone(), cancellation.clone());
    if request.background {
        let task_id = task.id.clone();
        let db = Arc::clone(&state.db);
        capabilities::start_task(&db, &task_id);
        tokio::spawn(async move {
            let result = execute_capability(
                Arc::clone(&state),
                entry,
                user_id,
                query.project_path,
                assistant_id.clone(),
                request,
                task_id.clone(),
                cancellation,
                ephemeral_payload,
            )
            .await;
            match result {
                Ok(output) => {
                    capabilities::finish_task(&db, &task_id, Some(&output.retained), None)
                }
                Err(error) => {
                    let retained = if ephemeral_payload {
                        "Personal capability request failed; details were not retained"
                    } else {
                        &error
                    };
                    capabilities::finish_task(&db, &task_id, None, Some(retained));
                }
            }
            state.capability_cancellations.lock().await.remove(&task_id);
        });
        return (StatusCode::ACCEPTED, Json(json!({"task":task})));
    }
    capabilities::start_task(&state.db, &task.id);
    let result = execute_capability(
        Arc::clone(&state),
        entry,
        user_id,
        query.project_path,
        assistant_id,
        request,
        task.id.clone(),
        cancellation,
        ephemeral_payload,
    )
    .await;
    state.capability_cancellations.lock().await.remove(&task.id);
    match result {
        Ok(output) => {
            capabilities::finish_task(&state.db, &task.id, Some(&output.retained), None);
            (
                StatusCode::OK,
                Json(json!({"taskId":task.id,"result":output.response})),
            )
        }
        Err(error) => {
            let retained = if ephemeral_payload {
                "Personal capability request failed; details were not retained"
            } else {
                &error
            };
            capabilities::finish_task(&state.db, &task.id, None, Some(retained));
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"taskId":task.id,"error":error})),
            )
        }
    }
}

pub async fn handle_metrics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    (
        StatusCode::OK,
        Json(capabilities::metrics(&state.db, &user_id)),
    )
}

pub async fn handle_cancel_task(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if capabilities::cancel_task(&state.db, &id, &user_id) {
        if let Some(token) = state.capability_cancellations.lock().await.get(&id) {
            token.cancel();
        }
        (
            StatusCode::OK,
            Json(json!({"ok":true,"status":"cancelling"})),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Running task not found"})),
        )
    }
}

pub async fn handle_list_packs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = user(&state, &headers) {
        return response;
    }
    (
        StatusCode::OK,
        Json(json!({
            "packs":capability_packs::list_pack_installations(&state.db),
            "catalogSource":"capabilities",
            "signedOnly":true,
        })),
    )
}

pub async fn handle_preview_pack(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(envelope): Json<capability_packs::CapabilityPackEnvelope>,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = require_admin(&state, &headers) {
        return response;
    }
    match capability_packs::preview_capability_pack(&state.db, &envelope) {
        Ok(preview) => (StatusCode::OK, Json(preview)),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error,"verified":false,"installed":false})),
        ),
    }
}

pub async fn handle_install_pack(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(envelope): Json<capability_packs::CapabilityPackEnvelope>,
) -> (StatusCode, Json<Value>) {
    let admin = match require_admin(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match capability_packs::install_capability_pack(&state.db, &envelope, &admin) {
        Ok(installation) => {
            capabilities::record_audit(
                &state.db,
                &admin,
                installation.capability_id.as_deref(),
                None,
                None,
                "capability_pack_install",
                "allow",
                None,
                "Signed Flamel capability pack installed",
                &json!({
                    "packId":installation.pack_id,
                    "activeVersion":installation.active_version,
                    "previousVersion":installation.previous_version,
                    "signed":true,
                }),
            );
            (
                StatusCode::CREATED,
                Json(json!({"ok":true,"pack":installation})),
            )
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error,"installed":false})),
        ),
    }
}

pub async fn handle_rollback_pack(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(pack_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let admin = match require_admin(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match capability_packs::rollback_capability_pack(&state.db, &pack_id, &admin) {
        Ok(installation) => {
            capabilities::record_audit(
                &state.db,
                &admin,
                installation.capability_id.as_deref(),
                None,
                None,
                "capability_pack_rollback",
                "allow",
                None,
                "Capability pack rolled back",
                &json!({
                    "packId":installation.pack_id,
                    "activeVersion":installation.active_version,
                    "previousVersion":installation.previous_version,
                }),
            );
            (StatusCode::OK, Json(json!({"ok":true,"pack":installation})))
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn handle_uninstall_pack(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(pack_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let admin = match require_admin(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let affected_skill_scopes = skill_catalog::installed_scopes_for_pack(&state.db, &pack_id);
    match capability_packs::uninstall_capability_pack(&state.db, &pack_id, &admin) {
        Ok(installation) => {
            let skill_snapshots = affected_skill_scopes
                .iter()
                .map(|(user_id, project_path)| {
                    let project_path = (!project_path.is_empty()).then_some(project_path.as_str());
                    json!({
                        "userId":user_id,
                        "projectPath":project_path,
                        "snapshots":refresh_skill_snapshots(&state,user_id,project_path),
                    })
                })
                .collect::<Vec<_>>();
            capabilities::record_audit(
                &state.db,
                &admin,
                None,
                None,
                None,
                "capability_pack_uninstall",
                "allow",
                None,
                "Capability pack uninstalled; credentials revoked and schedules disabled",
                &json!({
                    "packId":installation.pack_id,
                    "state":installation.state,
                    "credentialsRevoked":true,
                    "schedulesDisabled":true,
                    "skillSnapshotsRefreshed":skill_snapshots.len(),
                }),
            );
            (
                StatusCode::OK,
                Json(json!({"ok":true,"pack":installation,"skillSnapshots":skill_snapshots})),
            )
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn handle_marketplace(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = user(&state, &headers) {
        return response;
    }
    (
        StatusCode::OK,
        Json(json!({"entries":capabilities::list_marketplace(&state.db),"adminManaged":true})),
    )
}

pub async fn handle_install_marketplace(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let admin = match require_admin(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match capabilities::install_marketplace(&state.db, &id) {
        Ok(capability_id) => {
            capabilities::record_audit(
                &state.db,
                &admin,
                Some(&capability_id),
                None,
                None,
                "marketplace_install",
                "allow",
                None,
                "Marketplace integration approved",
                &json!({"marketplaceId":id}),
            );
            (
                StatusCode::CREATED,
                Json(json!({"ok":true,"capabilityId":capability_id})),
            )
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn handle_preview_openclaw_import(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = user(&state, &headers) {
        return response;
    }
    match crate::openclaw_import::preview_openclaw_import(&body) {
        Ok(preview) => (StatusCode::OK, Json(json!(preview))),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error,"dryRun":true,"imported":false})),
        ),
    }
}

pub async fn handle_import_marketplace(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let admin = match require_admin(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match capabilities::import_marketplace_manifest(&state.db, &body) {
        Ok(result) => {
            capabilities::record_audit(
                &state.db,
                &admin,
                None,
                None,
                None,
                "registry_import",
                "allow",
                None,
                "Private capability registry imported",
                &result,
            );
            (StatusCode::CREATED, Json(result))
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

#[derive(Deserialize)]
pub struct SkillQuery {
    pub(crate) query: Option<String>,
    #[serde(rename = "projectPath")]
    pub(crate) project_path: Option<String>,
    pub(crate) provider: Option<String>,
}
#[derive(Deserialize)]
pub struct LimitQuery {
    limit: Option<i64>,
}

fn refresh_skill_snapshots(state: &AppState, user_id: &str, project_path: Option<&str>) -> Value {
    skill_catalog::refresh_user_skill_projections(
        &state.db,
        user_id,
        project_path,
        &skill_user_home(state, user_id),
    )
}

pub async fn handle_skills(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<SkillQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    (
        StatusCode::OK,
        Json(json!({
            "skills":skill_catalog::list_skills_for_assistant(
                &state.db,
                &user_id,
                query.project_path.as_deref(),
                query.provider.as_deref().filter(|provider| !provider.is_empty()),
                query.query.as_deref().unwrap_or(""),
                runtime_assistant_id(&headers).as_deref(),
            ),
            "instructionOnly":true,
            "reviewRequired":true,
        })),
    )
}

pub async fn handle_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<SkillQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match skill_catalog::inspect_skill(&state.db, &id, &user_id, query.project_path.as_deref()) {
        Ok(skill)
            if runtime_assistant_id(&headers).is_none_or(|assistant_id| {
                skill
                    .get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(|slug| {
                        skill_catalog::assistant_skill_allowed(&state.db, &assistant_id, slug)
                    })
            }) =>
        {
            (StatusCode::OK, Json(skill))
        }
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Skill was not found or is outside this assistant policy"})),
        ),
        Err(error) => (StatusCode::NOT_FOUND, Json(json!({"error":error}))),
    }
}

pub async fn handle_propose_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let slug = body.get("slug").and_then(Value::as_str).unwrap_or("");
    let input = body.get("skill").cloned().unwrap_or_else(|| json!({}));
    let input = match serde_json::from_value::<skill_catalog::SkillVersionInput>(input) {
        Ok(input) => input,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":format!("Invalid skill proposal: {error}")})),
            );
        }
    };
    match skill_catalog::propose_skill(&state.db, slug, &user_id, &input) {
        Ok(skill) => {
            capabilities::record_audit(
                &state.db,
                &user_id,
                None,
                None,
                None,
                "skill_proposed",
                "review_required",
                None,
                "Instruction-only skill proposed for Flamel review",
                &json!({"skillId":skill["id"],"version":skill["version"],"quarantined":skill["quarantined"]}),
            );
            (StatusCode::CREATED, Json(json!({"skill":skill})))
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn handle_review_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let admin = match require_admin(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let version = body.get("version").and_then(Value::as_str).unwrap_or("");
    let approve = body.get("action").and_then(Value::as_str) == Some("approve");
    let affected_scopes = skill_catalog::installed_scopes_for_skill(&state.db, &id);
    match skill_catalog::review_skill(&state.db, &id, version, &admin, approve) {
        Ok(skill) => {
            let skill_snapshots = if approve {
                Vec::new()
            } else {
                affected_scopes
                    .iter()
                    .map(|(user_id, project_path)| {
                        let project_path =
                            (!project_path.is_empty()).then_some(project_path.as_str());
                        json!({
                            "userId":user_id,
                            "projectPath":project_path,
                            "snapshots":refresh_skill_snapshots(&state,user_id,project_path),
                        })
                    })
                    .collect::<Vec<_>>()
            };
            capabilities::record_audit(
                &state.db,
                &admin,
                None,
                None,
                None,
                "skill_reviewed",
                if approve { "allow" } else { "deny" },
                None,
                "Flamel skill review completed",
                &json!({
                    "skillId":id,
                    "version":version,
                    "skillSnapshotsRefreshed":skill_snapshots.len(),
                }),
            );
            (
                StatusCode::OK,
                Json(json!({"skill":skill,"skillSnapshots":skill_snapshots})),
            )
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn handle_add_skill_version(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let admin = match require_admin(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let input = match serde_json::from_value::<skill_catalog::SkillVersionInput>(body) {
        Ok(input) => input,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":format!("Invalid skill version: {error}")})),
            );
        }
    };
    match skill_catalog::add_reviewed_version(&state.db, &id, &admin, &input) {
        Ok(skill) => {
            capabilities::record_audit(
                &state.db,
                &admin,
                None,
                None,
                None,
                "skill_version_added",
                "allow",
                None,
                "Reviewed Flamel skill version added",
                &json!({"skillId":id,"version":skill["version"],"contentHash":skill["contentHash"]}),
            );
            (StatusCode::CREATED, Json(json!({"skill":skill})))
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn handle_install_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let project_path = body.get("projectPath").and_then(Value::as_str);
    let version = body.get("version").and_then(Value::as_str);
    let projection = body
        .get("providerProjectionEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    match skill_catalog::install_skill(&state.db, &id, &user_id, project_path, version, projection)
    {
        Ok(skill) => {
            let snapshots = refresh_skill_snapshots(&state, &user_id, project_path);
            capabilities::record_audit(
                &state.db,
                &user_id,
                None,
                None,
                None,
                "skill_installed",
                "allow",
                None,
                "Reviewed Flamel skill installed",
                &json!({"skillId":id,"version":skill["version"],"projectPath":project_path}),
            );
            (
                StatusCode::OK,
                Json(json!({"skill":skill,"snapshots":snapshots})),
            )
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn handle_rollback_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let project_path = body.get("projectPath").and_then(Value::as_str);
    match skill_catalog::rollback_skill(&state.db, &id, &user_id, project_path) {
        Ok(skill) => {
            let snapshots = refresh_skill_snapshots(&state, &user_id, project_path);
            (
                StatusCode::OK,
                Json(json!({"skill":skill,"snapshots":snapshots})),
            )
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn handle_set_skill_state(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let project_path = body.get("projectPath").and_then(Value::as_str);
    let enabled = body.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    let projection = body
        .get("providerProjectionEnabled")
        .and_then(Value::as_bool);
    match skill_catalog::set_skill_enabled(
        &state.db,
        &id,
        &user_id,
        project_path,
        enabled,
        projection,
    ) {
        Ok(()) => {
            let snapshots = refresh_skill_snapshots(&state, &user_id, project_path);
            (
                StatusCode::OK,
                Json(json!({"ok":true,"enabled":enabled,"snapshots":snapshots})),
            )
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn handle_uninstall_skill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let project_path = body.get("projectPath").and_then(Value::as_str);
    match skill_catalog::uninstall_skill(&state.db, &id, &user_id, project_path) {
        Ok(()) => {
            let snapshots = refresh_skill_snapshots(&state, &user_id, project_path);
            capabilities::record_audit(
                &state.db,
                &user_id,
                None,
                None,
                None,
                "skill_uninstalled",
                "allow",
                None,
                "Flamel skill uninstalled and provider snapshot refreshed",
                &json!({"skillId":id,"projectPath":project_path}),
            );
            (
                StatusCode::OK,
                Json(json!({"ok":true,"snapshots":snapshots})),
            )
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}

pub async fn handle_set_credential(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let value = body.get("value").and_then(Value::as_str).unwrap_or("");
    let credential_kind = body.get("kind").and_then(Value::as_str).unwrap_or("token");
    if value.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Credential value is required"})),
        );
    }
    if let Err(error) = capabilities::validate_credential_kind(&state.db, &id, credential_kind) {
        return (StatusCode::BAD_REQUEST, Json(json!({"error":error})));
    }
    let encrypted = match state.credential_vault.encrypt(value) {
        Ok(v) => v,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":error})),
            );
        }
    };
    match crate::db::credentials::store(
        &state.db,
        &user_id,
        &id,
        credential_kind,
        &encrypted,
        &body.get("metadata").cloned().unwrap_or_else(|| json!({})),
        body.get("expiresAt").and_then(Value::as_str),
    ) {
        Ok(reference) => {
            capabilities::record_audit(
                &state.db,
                &user_id,
                Some(&id),
                None,
                None,
                "credential_set",
                "allow",
                None,
                "Credential stored",
                &json!({"credentialRef":reference}),
            );
            (
                StatusCode::CREATED,
                Json(json!({"ok":true,"credentialRef":reference})),
            )
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}
pub async fn handle_delete_credential(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if crate::db::credentials::delete(&state.db, &user_id, &id) {
        capabilities::record_audit(
            &state.db,
            &user_id,
            Some(&id),
            None,
            None,
            "credential_delete",
            "allow",
            None,
            "Credential removed",
            &json!({}),
        );
        (StatusCode::OK, Json(json!({"ok":true})))
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Credential not found"})),
        )
    }
}

pub async fn handle_tasks(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LimitQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    (
        StatusCode::OK,
        Json(
            json!({"tasks":capabilities::list_tasks(&state.db,&user_id,query.limit.unwrap_or(50))}),
        ),
    )
}
pub async fn handle_audit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LimitQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    (
        StatusCode::OK,
        Json(
            json!({"events":capabilities::list_audit(&state.db,&user_id,query.limit.unwrap_or(100))}),
        ),
    )
}
pub async fn handle_invocations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LimitQuery>,
) -> (StatusCode, Json<Value>) {
    let user_id = match user(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    (
        StatusCode::OK,
        Json(
            json!({"invocations":capabilities::list_invocations(&state.db,&user_id,query.limit.unwrap_or(100))}),
        ),
    )
}
pub async fn handle_policies(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = require_admin(&state, &headers) {
        return response;
    };
    (
        StatusCode::OK,
        Json(json!({"policies":capabilities::list_policies(&state.db)})),
    )
}
pub async fn handle_create_policy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let admin = match require_admin(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match capabilities::create_policy(&state.db, &admin, &body) {
        Ok(policy) => (StatusCode::CREATED, Json(policy)),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error":error}))),
    }
}
pub async fn handle_delete_policy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = require_admin(&state, &headers) {
        return response;
    };
    if capabilities::delete_policy(&state.db, &id) {
        (StatusCode::OK, Json(json!({"ok":true})))
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Policy not found"})),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{is_authentication_error, is_recoverable_transport_error, redacted_task_payload};
    use serde_json::json;

    #[test]
    fn classifies_common_remote_auth_failures() {
        assert!(is_authentication_error("HTTP status 401 Unauthorized"));
        assert!(is_authentication_error("MCP request failed: status 403"));
        assert!(!is_authentication_error("Transport closed"));
    }

    #[test]
    fn retries_transport_failures_but_not_auth_or_tool_errors() {
        assert!(is_recoverable_transport_error("Transport closed"));
        assert!(is_recoverable_transport_error(
            "MCP initialization failed: connection reset"
        ));
        assert!(!is_recoverable_transport_error(
            "HTTP status 401 Unauthorized"
        ));
        assert!(!is_recoverable_transport_error(
            "Tool rejected invalid input"
        ));
    }

    #[test]
    fn ephemeral_task_payloads_never_retain_personal_values_or_vendor_summaries() {
        let input = redacted_task_payload(
            "gmail.message.send",
            "input",
            &json!({
                "body":{"raw":"private mail body"},
                "location":"private home address",
                "state":"alarm is disarmed"
            }),
        );
        let output = redacted_task_payload(
            "google-drive.file.export",
            "output",
            &json!({
                "data":{"text":"private document text"},
                "summary":"private vendor-controlled summary"
            }),
        );
        let retained = format!("{input}{output}");
        for secret in [
            "private mail body",
            "private home address",
            "alarm is disarmed",
            "private document text",
            "private vendor-controlled summary",
        ] {
            assert!(!retained.contains(secret));
        }
        assert_eq!(input["retention"], "ephemeral");
        assert_eq!(output["redacted"], true);
        assert_eq!(
            output["summary"],
            "Personal capability output was returned for the current task"
        );
    }
}

fn skill_user_home(state: &AppState, _user_id: &str) -> std::path::PathBuf {
    state.home_dir.clone()
}
