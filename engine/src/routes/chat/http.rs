use super::*;

pub struct BackgroundChatTask {
    pub session_id: String,
    pub request_id: String,
}

pub(super) fn chat_access_root(
    user_home: &std::path::Path,
    local_mode: bool,
    local_access: bool,
) -> std::path::PathBuf {
    crate::routes::path_guard::access_root(user_home, local_mode || local_access)
}

pub(super) fn planning_mode_addendum(mode: &str) -> &'static str {
    match mode {
        "ultraplan" => {
            "\n\nPLANNING MODE ACTIVE: Before writing code or making changes, inspect the relevant context and resolve only consequential user choices with AskUserQuestion. Ask one question at a time, adapt to each answer, and ask no more than five questions for the planning request; skip questions when discovery leaves no material choice. When choices are useful, offer two or three mutually exclusive options with the recommendation first and a short impact statement. A question blocks plan authoring until it is answered or cancelled. Then call plan(action=\"create\", ...) with a direct execution handoff containing verified files, symbols, contracts, ordering, accepted decisions, assumptions, rejected or deferred alternatives, acceptance evidence, and reasoning floors. Do not expose private chain-of-thought. After creating the plan, STOP and wait for explicit approval."
        }
        "ultrareview" => {
            "\n\nREVIEW MODE ACTIVE: Before making any changes, perform a thorough review of the relevant code. Identify all issues, potential bugs, and improvement opportunities. Present your findings clearly before proposing any fixes."
        }
        _ => "",
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn launch_background_chat_task(
    state: Arc<AppState>,
    user_id: String,
    working_directory: String,
    message: String,
    title: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    permission_mode: String,
    runner_id: Option<String>,
) -> Result<BackgroundChatTask, String> {
    if state.is_shutting_down() {
        return Err("Flamel Server is restarting; retry this job after reconnect".to_string());
    }
    if runner_id.is_some() {
        return Err("Remote worker dispatch is unavailable in the local workspace".into());
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    let session_id = uuid::Uuid::new_v4().to_string();
    let user_id_opt = Some(user_id.clone());
    let local_access = state.local_mode;
    let user_home = state.home_dir.clone();
    let access_root = chat_access_root(&user_home, state.local_mode, local_access);
    if let Err((_status, Json(body))) =
        crate::routes::path_guard::guard_existing(&access_root, &working_directory)
    {
        let message = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("invalid working directory");
        return Err(message.to_string());
    }

    let provider_kind = model
        .as_deref()
        .map(ProviderKind::from_model)
        .unwrap_or(ProviderKind::Codex);
    let credentials: Option<ProvCreds> = match provider_kind {
        ProviderKind::Claude => {
            try_refresh_credentials(&state, &user_id)
                .await
                .map(|c| ProvCreds {
                    user_id: c.user_id,
                    access_token: c.access_token,
                    refresh_token: c.refresh_token,
                    expires_at: c.expires_at,
                    scopes: c.scopes,
                    extra: None,
                })
        }
        ProviderKind::Codex => {
            crate::routes::codex_auth::try_refresh_codex_credentials(&state, &user_id)
                .await
                .map(|pc| ProvCreds {
                    user_id: pc.user_id,
                    access_token: pc.access_token,
                    refresh_token: pc.refresh_token,
                    expires_at: pc.expires_at,
                    scopes: pc.scopes,
                    extra: pc.id_token.map(|t| serde_json::json!({"id_token": t})),
                })
        }
    };

    let home = if local_access {
        std::env::var("HOME").unwrap_or_default()
    } else {
        user_home.to_string_lossy().into_owned()
    };
    let mut mcp_servers = read_claude_mcp_servers(&home, &working_directory);
    let db_configs = list_mcp_configs(&state.db, Some(&working_directory), Some(&user_id));
    for config in db_configs.iter().filter(|c| c.enabled) {
        mcp_servers.as_object_mut().unwrap().insert(
            config.name.clone(),
            json!({"command": config.command, "args": config.args}),
        );
    }
    for (name, config) in effective_mcp_servers(&state.db, &user_id, &working_directory) {
        mcp_servers.as_object_mut().unwrap().insert(name, config);
    }
    for retired in ["git", "docs", "db", "viz", "browser", "memory"] {
        mcp_servers.as_object_mut().unwrap().remove(retired);
    }

    let port = state.port;
    let provider_impl = crate::claude::provider::get_provider(provider_kind);
    let cli_path = if provider_kind == ProviderKind::Claude {
        crate::claude::spawn::find_claude_cli()
    } else {
        provider_impl
            .find_binary()
            .unwrap_or_else(|_| "codex".to_string())
    };
    let disallowed_tools_csv = if provider_kind == ProviderKind::Claude {
        crate::claude::spawn::disallowed_tools_csv()
    } else {
        provider_impl.disallowed_tools_csv()
    };

    let flamel_mcp_path = crate::bundled_binary_path("flamel_mcp");
    mcp_servers.as_object_mut().unwrap().insert(
        "flamel".to_string(),
        json!({
            "command": flamel_mcp_path.to_string_lossy(),
            "args": [],
            "tool_timeout_sec": 900,
            "startup_timeout_sec": 30,
            "env": {
                "FLAMEL_URL": crate::callback::endpoint(&state),
                "FLAMEL_TOKEN": crate::mcp_auth::token_for_process(&state, &user_id, Some(&session_id)),
                "FLAMEL_SESSION_ID": session_id.clone(),
                "FLAMEL_RUN_ID": request_id,
                "FLAMEL_PROJECT_PATH": working_directory.clone(),
                "GIT_WORK_DIR": working_directory.clone(),
                "FLAMEL_USER_ID": user_id,
                "FLAMEL_USER_HOME": user_home.to_string_lossy(),
                "FLAMEL_SHELL_UNRESTRICTED": if local_access { "1" } else { "0" },
                "FLAMEL_PROVIDER_BINARY": cli_path,
                "FLAMEL_DISALLOWED_TOOLS": disallowed_tools_csv,
                "FLAMEL_AGENT_DEPTH": "0",
                "FLAMEL_PROVIDER": provider_kind.as_str(),
                "FLAMEL_CODEX_HOME": if provider_kind == ProviderKind::Codex {
                    format!("{}/.codex", user_home.to_string_lossy())
                } else {
                    String::new()
                },
                "FLAMEL_CODEX_REASONING_EFFORT": if provider_kind == ProviderKind::Codex {
                    reasoning_effort.as_deref().unwrap_or("high")
                } else {
                    ""
                },
            }
        }),
    );

    upsert_session(
        &state.db,
        &UpsertSession {
            id: session_id.clone(),
            claude_session_id: None,
            project_path: working_directory.clone(),
            title: title.clone(),
            model: model
                .clone()
                .or_else(|| Some(provider_impl.default_model().to_string())),
            // Preserve the last provider-reported context usage when admitting
            // another turn. New sessions still default to zero in upsert_session.
            input_tokens: None,
            output_tokens: None,
            message_count: Some(1),
            pinned: None,
            provider: Some(provider_kind.as_str().to_string()),
            user_id: user_id_opt.clone(),
            runner_id: runner_id.clone(),
        },
    );
    crate::db::realtime::create_run(
        &state.db,
        &request_id,
        &session_id,
        &user_id,
        &json!({
            "message": message,
            "workingDirectory": working_directory,
            "model": model,
            "reasoningEffort": reasoning_effort,
            "permissionMode": permission_mode,
            "runnerId": runner_id,
            "background": true,
        }),
        "queued",
    );
    crate::db::sessions::set_session_owner(
        &state.db,
        &session_id,
        "goal_worker",
        None,
        None,
        Some("worker"),
    );
    update_session_continuity(
        &state.db,
        &session_id,
        "queued",
        None,
        Some(&request_id),
        true,
        None,
    );

    let agent_defs = runtime_agent_defs(&state.db);
    let mut system_prompt =
        build_system_prompt_with_agents(&working_directory, provider_kind, &agent_defs);
    match crate::db::skill_catalog::project_provider_skill_home(
        &state.db,
        &user_id,
        Some(&working_directory),
        provider_kind.as_str(),
        &user_home,
    ) {
        Ok(_) => crate::db::skill_catalog::append_active_skill_registry(
            &mut system_prompt,
            &state.db,
            &user_id,
            Some(&working_directory),
            provider_kind.as_str(),
            &user_home,
        ),
        Err(error) => tracing::warn!(
            user_id,
            provider = provider_kind.as_str(),
            error,
            "could not refresh the governed skill snapshot for a background turn"
        ),
    }
    append_reasoning_delegation_context(&mut system_prompt, reasoning_effort.as_deref());
    append_durable_context_for_prompt(
        &mut system_prompt,
        &state.db,
        &session_id,
        user_id_opt.as_deref(),
        &working_directory,
    );
    system_prompt.push_str("\n\nFLAMEL JOB WORKER MODE: The user approved the Job objective, repository scope, and autonomy policy when starting this run. Treat that Job snapshot as the authorization boundary. Create, refine, and approve your own internal execution plan, then proceed without requesting a second plan approval. Do not create a separate user-facing Plan-panel plan for routine Job work. Ask only for material scope expansion, unavailable credentials, or destructive/external actions outside the approved autonomy policy. Use Flamel engineering tools first, keep visible progress concise, do not mark work complete without evidence, and end with a concrete completion summary and artifact list. Never batch MCP calls through code-mode Promise.all; use purpose-built batch actions or sequential tool calls so every result is observed.");

    {
        let serialized_mcp = json!({ "mcpServers": &mcp_servers }).to_string();
        if let Some(env) = mcp_servers
            .as_object_mut()
            .and_then(|o| o.get_mut("flamel"))
            .and_then(|v| v.as_object_mut())
            .and_then(|o| o.get_mut("env"))
            .and_then(|v| v.as_object_mut())
        {
            env.insert(
                "FLAMEL_SYSTEM_PROMPT".to_string(),
                Value::String(system_prompt.clone()),
            );
            env.insert(
                "FLAMEL_MCP_SERVERS".to_string(),
                Value::String(serialized_mcp),
            );
        }
    }

    let (abort_tx, abort_rx) = tokio::sync::watch::channel(false);
    state
        .abort_controllers
        .lock()
        .unwrap()
        .insert(request_id.clone(), abort_tx);
    state
        .request_to_session
        .lock()
        .unwrap()
        .insert(request_id.clone(), session_id.clone());

    persist_session_message(
        &state.db,
        &session_id,
        "user",
        &json!({
            "role": "user",
            "content": [{"type": "text", "text": message}],
            "flamel": {"hiddenContinuation": false, "worker": true},
        })
        .to_string(),
    );

    let (live_tx, _live_rx) = broadcast::channel::<String>(16_384);
    state.realtime.attach_legacy_stream(
        session_id.clone(),
        Some(request_id.clone()),
        live_tx.subscribe(),
    );
    {
        let mut streams = state.live_streams.lock().await;
        streams.insert(session_id.clone(), live_tx.clone());
    }
    {
        let mut active = state.active_sessions.lock().await;
        active.insert(session_id.clone());
    }

    let uid = user_id_opt.as_deref().unwrap_or("");
    let needs_auto = false;
    let resolved_runner_id: Option<String> = None;

    let mcp_server_names: Vec<String> = mcp_servers
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let dispatched_to_runner = false;

    let explicit_runner = runner_id
        .as_deref()
        .is_some_and(|runner_id| runner_id != "auto");
    if explicit_runner && !dispatched_to_runner {
        let error = "the selected Worker disconnected or could not accept this provider task";
        update_session_continuity(
            &state.db,
            &session_id,
            "error",
            Some("selected_worker_unavailable"),
            None,
            true,
            Some(error),
        );
        state.active_sessions.lock().await.remove(&session_id);
        state.live_streams.lock().await.remove(&session_id);
        return Err(error.into());
    }

    if !dispatched_to_runner {
        tokio::spawn(run_session(RunSessionInput {
            state: Arc::clone(&state),
            session_id: session_id.clone(),
            user_id: Some(user_id.clone()),
            working_directory: working_directory.clone(),
            message: message.clone(),
            images: None,
            model: model.clone(),
            reasoning_effort: reasoning_effort.clone(),
            permission_mode: permission_mode.clone(),
            request_id: request_id.clone(),
            title: title.clone(),
            credentials: credentials.clone(),
            system_prompt: system_prompt.clone(),
            mcp_servers: mcp_servers.clone(),
            mcp_server_names,
            existing_provider_resume_id: None,
            has_prior_visible_messages: false,
            defer_proactive_compaction: false,
            live_tx,
            abort_rx,
            user_home: user_home.to_string_lossy().to_string(),
            skip_sandbox: local_access,
            provider_kind,
        }));
    }

    Ok(BackgroundChatTask {
        session_id,
        request_id,
    })
}

#[derive(Deserialize)]
pub struct UpdateQueuedMessageRequest {
    message: String,
}

pub async fn handle_update_queued_message(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(request): Json<UpdateQueuedMessageRequest>,
) -> Response {
    let Some(user_id) = crate::routes::admin::get_current_user_id(&state, &headers) else {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))).into_response();
    };
    let message = request.message.trim();
    if message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Queued message cannot be empty"})),
        )
            .into_response();
    }

    match crate::db::realtime::update_queued_run_message(&state.db, &request_id, &user_id, message)
    {
        crate::db::realtime::QueuedRunUpdate::Updated { session_id } => {
            state.realtime.publish_value(
                &session_id,
                Some(&request_id),
                &json!({
                    "type": "queue_updated",
                    "sessionId": session_id,
                    "requestId": request_id,
                    "state": "queued",
                    "message": message,
                }),
            );
            Json(json!({
                "ok": true,
                "requestId": request_id,
                "sessionId": session_id,
                "message": message,
            }))
            .into_response()
        }
        crate::db::realtime::QueuedRunUpdate::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Queued message not found"})),
        )
            .into_response(),
        crate::db::realtime::QueuedRunUpdate::NotQueued => (
            StatusCode::CONFLICT,
            Json(json!({"error": "This message has already started and can no longer be edited"})),
        )
            .into_response(),
    }
}

pub fn handle_chat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> futures::future::BoxFuture<'static, Response> {
    Box::pin(async move {
        tracing::info!("POST /api/chat received");
        if state.is_shutting_down() {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::RETRY_AFTER, "2")],
                Json(json!({"error": "Flamel Server is restarting; this turn was not admitted"})),
            )
                .into_response();
        }
        let message = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let request_id = match body.get("requestId").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "requestId required"})),
                )
                    .into_response();
            }
        };
        let mut working_directory = match body.get("workingDirectory").and_then(|v| v.as_str()) {
            Some(d) => d.to_string(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "workingDirectory required"})),
                )
                    .into_response();
            }
        };
        let session_id = body
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let interrupt_active = body
            .get("interruptActive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let requested_active_plan_id = body
            .get("activePlanId")
            .and_then(|v| v.as_str())
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
            .map(str::to_string);
        let mut title = body
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(|s| s.to_string());
        let requested_model = body
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let requested_reasoning_effort = body
            .get("reasoningEffort")
            .and_then(|v| v.as_str())
            .filter(|value| {
                matches!(
                    *value,
                    "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
                )
            })
            .map(|s| s.to_string());
        let permission_mode = body
            .get("permissionMode")
            .and_then(|v| v.as_str())
            .unwrap_or("bypassPermissions")
            .to_string();
        let ponytail_enabled = body
            .get("ponytailEnabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let hidden_continuation = body
            .get("hiddenContinuation")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let images: Option<Vec<Value>> = body.get("images").and_then(|v| v.as_array()).cloned();
        let planning_mode = body
            .get("planningMode")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let runner_id = body
            .get("runnerId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if runner_id.is_some() {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    json!({"error":"Remote worker dispatch is unavailable in the local workspace"}),
                ),
            )
                .into_response();
        }
        let has_images = images.as_ref().is_some_and(|imgs| !imgs.is_empty());
        if message.is_empty() && !has_images {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "message required"})),
            )
                .into_response();
        }

        // Load user identity and validate working directory is within user's home
        let user_id = match crate::routes::admin::get_current_user_id(&state, &headers) {
            Some(id) => Some(id),
            None => {
                tracing::warn!("chat: no user_id — rejecting request");
                return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})))
                    .into_response();
            }
        };
        let existing_session = get_session_meta(&state.db, &session_id);
        if let Some(existing_session) = existing_session.as_ref() {
            if existing_session
                .session_owner_kind
                .as_deref()
                .is_some_and(|kind| kind != "interactive")
            {
                return (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "error": "internally managed sessions are read-only from chat"
                    })),
                )
                    .into_response();
            }
            if existing_session.user_id.as_deref() != user_id.as_deref() {
                return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})))
                    .into_response();
            }
        }
        let existing_has_title = existing_session
            .as_ref()
            .and_then(|session| session.title.as_deref())
            .is_some_and(|value| !value.trim().is_empty());
        let active_plan_id = requested_active_plan_id.or_else(|| {
            existing_session
                .as_ref()
                .and_then(|session| session.active_plan_id.clone())
        });
        if title.is_none() && !existing_has_title {
            title = Some(crate::db::session_titles::derive_session_title(
                &message, has_images,
            ));
        }

        let local_access = state.local_mode;
        let user_home = state.home_dir.clone();
        let access_root = chat_access_root(&user_home, state.local_mode, local_access);
        if let Err(e) = crate::routes::path_guard::guard_existing(&access_root, &working_directory)
        {
            return e.into_response();
        }
        let routing_settings =
            crate::db::settings::get_user_settings(&state.db, user_id.as_deref().unwrap_or(""));
        let availability =
            crate::routes::providers::provider_availability(&state, user_id.as_deref());
        let plan_reasoning_floor = active_plan_id.as_deref().and_then(|plan_id| {
            let owned = crate::db::plans::get_plan_user_id(&state.db, plan_id).as_deref()
                == user_id.as_deref();
            owned
                .then(|| crate::db::plans::active_plan_reasoning_floor(&state.db, plan_id))
                .flatten()
        });
        let routing = match resolve_model_route(
            requested_model.as_deref(),
            requested_reasoning_effort.as_deref(),
            &message,
            planning_mode.as_deref(),
            plan_reasoning_floor.as_deref(),
            &routing_settings,
            availability,
        ) {
            Ok(routing) => routing,
            Err(error) => {
                return (
                    StatusCode::PRECONDITION_FAILED,
                    Json(json!({"error": error, "code": "provider_unavailable"})),
                )
                    .into_response();
            }
        };
        let model = Some(routing.model.clone());
        let reasoning_effort = routing.reasoning_effort.clone();
        let provider_kind =
            ProviderKind::from_str(&routing.provider).unwrap_or(ProviderKind::Codex);

        let workspace_state = Arc::clone(&state);
        let workspace_session = session_id.clone();
        let workspace_user = user_id.clone().unwrap_or_default();
        let workspace_source = working_directory.clone();
        // Keep generated checkouts in the API-visible per-user home even when an
        // administrator's provider process uses the real system HOME.
        let workspace_home = crate::routes::admin::get_user_home(&state, user_id.as_deref());
        let remote_workspace = false;
        let prepared_workspace =
            match tokio::task::spawn_blocking(move || {
                crate::git_operations::workspaces::prepare(
                    &workspace_state.db,
                    &workspace_session,
                    &workspace_user,
                    &workspace_source,
                    &workspace_home,
                    remote_workspace,
                )
            })
            .await
            {
                Ok(Ok(prepared)) => prepared,
                Ok(Err(error)) => {
                    return (
                StatusCode::CONFLICT,
                Json(json!({"error": error, "code": "workspace_unavailable", "accepted": false})),
            )
                .into_response();
                }
                Err(error) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": error.to_string(), "accepted": false})),
                    )
                        .into_response();
                }
            };
        working_directory = prepared_workspace.path.clone();
        // Canonicalize before admission, so retries sent with either the originally
        // selected folder or the owned worktree match the same durable request.
        body["workingDirectory"] = json!(working_directory);
        let workspace = prepared_workspace.workspace.clone();

        // Persist ownership and the complete request before credential refresh or MCP
        // assembly. A client disconnect from this point onward cannot lose the turn,
        // and a duplicate request id cannot start a second provider process.
        upsert_session(
            &state.db,
            &UpsertSession {
                id: session_id.clone(),
                claude_session_id: None,
                project_path: working_directory.clone(),
                title: title.clone(),
                model: model.clone(),
                // Admission must not erase the usage value inspected by proactive
                // compaction. New sessions still receive zero from the insert path.
                input_tokens: None,
                output_tokens: None,
                message_count: Some(1),
                pinned: None,
                provider: Some(provider_kind.as_str().to_string()),
                user_id: user_id.clone(),
                runner_id: runner_id.clone(),
            },
        );
        set_session_routing(
            &state.db,
            &session_id,
            &routing.requested_model,
            &routing.requested_reasoning_effort,
            &routing.model,
            &routing.provider,
            routing.reasoning_effort.as_deref(),
            &routing.reason,
        );
        let (admission, predecessor_id) = crate::db::realtime::admit_run_with_predecessor(
            &state.db,
            &request_id,
            &session_id,
            user_id.as_deref().unwrap_or(""),
            &body,
        );
        // The repo lock spans selection AND admission. The next first-send now sees
        // this preparing run and cannot choose the same checkout concurrently.
        drop(prepared_workspace);
        match admission {
            crate::db::realtime::RunAdmission::Queued => {
                state.realtime.publish_value(
                    &session_id,
                    Some(&request_id),
                    &json!({
                        "type": "queue_updated",
                        "sessionId": session_id,
                        "requestId": request_id,
                        "state": "queued",
                        "message": message,
                        "hiddenContinuation": hidden_continuation,
                        "interruptActive": interrupt_active,
                    }),
                );
                if interrupt_active {
                    if let Some(predecessor_id) = predecessor_id.clone() {
                        state.realtime.publish_value(
                            &session_id,
                            Some(&predecessor_id),
                            &json!({
                                "type": "interrupt_requested",
                                "sessionId": session_id,
                                "requestId": predecessor_id,
                                "replacementRequestId": request_id,
                            }),
                        );
                        let interrupt_state = Arc::clone(&state);
                        tokio::spawn(async move {
                            // Admission can finish while the predecessor is still
                            // assembling credentials and MCP config. Wait briefly for
                            // its exact controller instead of falling back to a broad
                            // session abort that could race with the successor.
                            for _ in 0..200 {
                                if crate::routes::abort::signal_request_abort_controller(
                                    &interrupt_state,
                                    &predecessor_id,
                                ) {
                                    return;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                            }
                            tracing::warn!(
                                request_id = predecessor_id,
                                "interrupt predecessor finished before its controller was signaled"
                            );
                        });
                    }
                }
                return (
                    StatusCode::ACCEPTED,
                    Json(json!({
                        "queued": true,
                        "requestId": request_id,
                        "sessionId": session_id,
                        "interruptingRequestId": predecessor_id,
                        "projectPath": working_directory,
                        "workspace": workspace,
                    })),
                )
                    .into_response();
            }
            crate::db::realtime::RunAdmission::Duplicate => {
                return (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "error": "request already accepted",
                        "accepted": true,
                        "code": "request_already_accepted",
                        "requestId": request_id,
                        "sessionId": session_id,
                        "projectPath": working_directory,
                        "workspace": workspace,
                    })),
                )
                    .into_response();
            }
            crate::db::realtime::RunAdmission::Conflict => {
                return (StatusCode::CONFLICT, Json(json!({
                "error": "This message ID belongs to a different request. Your new message was not saved.",
                "accepted": false, "code": "request_id_conflict", "requestId": request_id,
            }))).into_response();
            }
            crate::db::realtime::RunAdmission::StorageUnavailable => {
                return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({
                "error": "Your message could not be saved. Please retry; it has not been sent to the agent.",
                "accepted": false, "code": "input_storage_unavailable", "requestId": request_id,
            }))).into_response();
            }
            crate::db::realtime::RunAdmission::Start => {}
        }
        state.realtime.publish_value(
            &session_id,
            Some(&request_id),
            &json!({
                "type": "model_routed",
                "sessionId": session_id,
                "requestId": request_id,
                "routing": routing,
                "projectPath": working_directory,
                "workspace": workspace,
            }),
        );
        // Publish durable ownership before credential refresh and MCP assembly so a
        // reconnect or a second prompt sees this session as active immediately.
        update_session_continuity(
            &state.db,
            &session_id,
            "queued",
            None,
            Some(&request_id),
            true,
            None,
        );
        // Fetch and auto-refresh provider credentials (Claude OAuth or Codex OAuth)
        let credentials: Option<ProvCreds> = if let Some(ref uid) = user_id {
            match provider_kind {
                ProviderKind::Claude => {
                    try_refresh_credentials(&state, uid)
                        .await
                        .map(|c| ProvCreds {
                            user_id: c.user_id,
                            access_token: c.access_token,
                            refresh_token: c.refresh_token,
                            expires_at: c.expires_at,
                            scopes: c.scopes,
                            extra: None,
                        })
                }
                ProviderKind::Codex => {
                    match crate::routes::codex_auth::try_refresh_codex_credentials(&state, uid)
                        .await
                    {
                        Some(pc) => Some(ProvCreds {
                            user_id: pc.user_id,
                            access_token: pc.access_token,
                            refresh_token: pc.refresh_token,
                            expires_at: pc.expires_at,
                            scopes: pc.scopes,
                            extra: pc.id_token.map(|t| serde_json::json!({"id_token": t})),
                        }),
                        None => {
                            tracing::warn!("no codex credentials for user {}", uid);
                            None
                        }
                    }
                }
            }
        } else {
            None
        };

        // Build MCP servers config
        let home = if local_access {
            std::env::var("HOME").unwrap_or_default()
        } else {
            user_home.to_string_lossy().into_owned()
        };
        let mut mcp_servers = read_claude_mcp_servers(&home, &working_directory);

        // Layer UI-configured servers
        let db_configs = list_mcp_configs(&state.db, Some(&working_directory), user_id.as_deref());
        for config in db_configs.iter().filter(|c| c.enabled) {
            mcp_servers.as_object_mut().unwrap().insert(
                config.name.clone(),
                json!({"command": config.command, "args": config.args}),
            );
        }
        if let Some(user_id) = user_id.as_deref() {
            for (name, config) in effective_mcp_servers(&state.db, user_id, &working_directory) {
                mcp_servers.as_object_mut().unwrap().insert(name, config);
            }
        }
        for retired in ["git", "docs", "db", "viz", "browser", "memory"] {
            mcp_servers.as_object_mut().unwrap().remove(retired);
        }

        // Inject flamel planner MCP — uses the compiled or bundled Rust flamel_mcp binary
        let flamel_mcp_path = crate::bundled_binary_path("flamel_mcp");
        let port = state.port;
        let provider_impl = crate::claude::provider::get_provider(provider_kind);
        let claude_cli_path = if provider_kind == ProviderKind::Claude {
            crate::claude::spawn::find_claude_cli()
        } else {
            provider_impl
                .find_binary()
                .unwrap_or_else(|_| "codex".to_string())
        };
        let disallowed_tools_csv = if provider_kind == ProviderKind::Claude {
            crate::claude::spawn::disallowed_tools_csv()
        } else {
            provider_impl.disallowed_tools_csv()
        };
        mcp_servers.as_object_mut().unwrap().insert(
        "flamel".to_string(),
        json!({
            "command": flamel_mcp_path.to_string_lossy(),
            "args": [],
            "tool_timeout_sec": 900,
            "startup_timeout_sec": 30,
            "env": {
                "FLAMEL_URL": crate::callback::endpoint(&state),
                "FLAMEL_TOKEN": crate::mcp_auth::token_for_process(&state, user_id.as_deref().unwrap_or(""), Some(&session_id)),
                "FLAMEL_SESSION_ID": session_id,
                "FLAMEL_RUN_ID": request_id,
                "FLAMEL_PROJECT_PATH": working_directory,
                "GIT_WORK_DIR": working_directory,
                "FLAMEL_USER_ID": user_id.as_deref().unwrap_or(""),
                "FLAMEL_USER_HOME": user_home.to_string_lossy(),
                "FLAMEL_SHELL_UNRESTRICTED": if local_access { "1" } else { "0" },
                // Sub-agent plumbing: flamel_mcp's Agent tool uses these to spawn
                // child claude subprocesses with identical restrictions.
                // FLAMEL_SYSTEM_PROMPT and FLAMEL_MCP_SERVERS are filled in
                // after the system prompt is composed and all MCP servers are
                // registered — see post-fix block below.
                "FLAMEL_PROVIDER_BINARY": claude_cli_path,
                "FLAMEL_DISALLOWED_TOOLS": disallowed_tools_csv,
                "FLAMEL_AGENT_DEPTH": "0",
                "FLAMEL_PROVIDER": provider_kind.as_str(),
                "FLAMEL_CODEX_HOME": if provider_kind == ProviderKind::Codex {
                    format!("{}/.codex", user_home.to_string_lossy())
                } else {
                    String::new()
                },
                "FLAMEL_CODEX_REASONING_EFFORT": if provider_kind == ProviderKind::Codex {
                    reasoning_effort.as_deref().unwrap_or("high")
                } else {
                    ""
                },
            }
        }),
    );

        update_session_continuity(
            &state.db,
            &session_id,
            "queued",
            None,
            Some(&request_id),
            true,
            None,
        );

        let has_prior_visible_messages = has_visible_session_messages(&state.db, &session_id);
        // Persisted provider resume IDs are only valid after Flamel has visible prior
        // chat history for the session. Empty sessions can carry stale provider ids from
        // tab/session reuse and must start as fresh first prompts.
        let existing_provider_resume_id =
            if has_prior_visible_messages && provider_supports_persisted_resume(provider_kind) {
                crate::db::sessions::get_session(&state.db, &session_id).and_then(|s| {
                    if s.provider == provider_kind.as_str() {
                        s.claude_session_id
                    } else {
                        None
                    }
                })
            } else {
                None
            };

        let agent_defs = runtime_agent_defs(&state.db);
        let mut system_prompt =
            build_system_prompt_with_agents(&working_directory, provider_kind, &agent_defs);
        if let Some(skill_user_id) = user_id.as_deref() {
            match crate::db::skill_catalog::project_provider_skill_home(
                &state.db,
                skill_user_id,
                Some(&working_directory),
                provider_kind.as_str(),
                &user_home,
            ) {
                Ok(_) => crate::db::skill_catalog::append_active_skill_registry(
                    &mut system_prompt,
                    &state.db,
                    skill_user_id,
                    Some(&working_directory),
                    provider_kind.as_str(),
                    &user_home,
                ),
                Err(error) => tracing::warn!(
                    user_id = skill_user_id,
                    provider = provider_kind.as_str(),
                    error,
                    "could not refresh the governed skill snapshot for a chat turn"
                ),
            }
        }
        append_reasoning_delegation_context(&mut system_prompt, reasoning_effort.as_deref());
        append_ponytail_context(&mut system_prompt, ponytail_enabled);
        append_active_plan_panel_context(
            &mut system_prompt,
            active_plan_id.as_deref(),
            plan_reasoning_floor.as_deref(),
            reasoning_effort.as_deref(),
            crate::model_routing::automatic_reasoning(requested_reasoning_effort.as_deref()),
        );
        append_durable_context_for_prompt(
            &mut system_prompt,
            &state.db,
            &session_id,
            user_id.as_deref(),
            &working_directory,
        );

        // Inject planning mode addendum
        if let Some(ref mode) = planning_mode {
            system_prompt.push_str(planning_mode_addendum(mode));
        }

        // Post-fix: now that system_prompt is final and every MCP server is registered,
        // inject them into the flamel MCP entry so the Agent tool inside flamel_mcp
        // can spawn child `claude` subprocesses with the same context as the parent.
        // Taking a snapshot without the flamel entry itself would cause children to
        // lose access to mcp__flamel__* tools, so we include the complete config.
        {
            // Wrap in `{"mcpServers": {...}}` — that's the shape claude's
            // --mcp-config expects. flamel_mcp passes this env var straight through
            // to `--mcp-config` when spawning a child, and without the outer key
            // claude rejects it as "Does not adhere to MCP server configuration
            // schema". Matches what spawn.rs:108 does for the parent.
            let serialized_mcp = json!({ "mcpServers": &mcp_servers }).to_string();
            if let Some(flamel_entry) = mcp_servers
                .as_object_mut()
                .and_then(|o| o.get_mut("flamel"))
                .and_then(|v| v.as_object_mut())
            {
                if let Some(env) = flamel_entry.get_mut("env").and_then(|v| v.as_object_mut()) {
                    env.insert(
                        "FLAMEL_SYSTEM_PROMPT".to_string(),
                        Value::String(system_prompt.clone()),
                    );
                    env.insert(
                        "FLAMEL_MCP_SERVERS".to_string(),
                        Value::String(serialized_mcp),
                    );
                }
            }
        }

        // Set up abort controller
        let (abort_tx, abort_rx) = tokio::sync::watch::channel(false);
        state
            .abort_controllers
            .lock()
            .unwrap()
            .insert(request_id.clone(), abort_tx);
        // Track request → session so abort handler can send SIGINT to the right process
        state
            .request_to_session
            .lock()
            .unwrap()
            .insert(request_id.clone(), session_id.clone());

        let mcp_server_names: Vec<String> = mcp_servers
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();

        // Insert user message to DB immediately — before runner starts so reconnecting devices see it
        {
            let user_content = if has_images {
                let mut parts = Vec::new();
                if !message.is_empty() {
                    parts.push(json!({"type": "text", "text": message}));
                }
                if let Some(ref imgs) = images {
                    for img in imgs {
                        parts.push(
                        json!({"type": "image_placeholder", "media_type": img.get("mediaType")}),
                    );
                    }
                }
                json!(parts)
            } else {
                json!([{"type": "text", "text": message}])
            };
            persist_session_message(
                &state.db,
                &session_id,
                "user",
                &json!({
                    "role": "user",
                    "content": user_content,
                    "flamel": {
                        "hiddenContinuation": hidden_continuation,
                        "requestId": request_id,
                    },
                })
                .to_string(),
            );
        }

        // Create live broadcast channel
        let (live_tx, live_rx) = broadcast::channel::<String>(16_384);
        state.realtime.attach_legacy_stream(
            session_id.clone(),
            Some(request_id.clone()),
            live_tx.subscribe(),
        );
        {
            let mut streams = state.live_streams.lock().await;
            streams.insert(session_id.clone(), live_tx.clone());
        }
        {
            let mut active = state.active_sessions.lock().await;
            active.insert(session_id.clone());
        }

        {
            // Local execution — spawn CLI on this server
            tokio::spawn(run_session(RunSessionInput {
                state: Arc::clone(&state),
                session_id: session_id.clone(),
                user_id: user_id.clone(),
                working_directory: working_directory.clone(),
                message: message.clone(),
                images: images.clone(),
                model: model.clone(),
                reasoning_effort: reasoning_effort.clone(),
                permission_mode: permission_mode.clone(),
                request_id: request_id.clone(),
                title: title.clone(),
                credentials: credentials.clone(),
                system_prompt: system_prompt.clone(),
                mcp_servers: mcp_servers.clone(),
                mcp_server_names: mcp_server_names.clone(),
                existing_provider_resume_id,
                has_prior_visible_messages,
                defer_proactive_compaction: interrupt_active,
                live_tx,
                abort_rx,
                user_home: user_home.to_string_lossy().to_string(),
                skip_sandbox: local_access,
                provider_kind,
            }));
        }

        // HTTP response: dumb subscriber — forwards from live broadcast to this client.
        // Dropping this (on disconnect) does NOT affect the runner.
        let session_id_clone = session_id.clone();
        let request_id_clone = request_id.clone();
        let mut live_rx = live_rx;
        let stream = async_stream::stream! {
            // "connected" is per-client (carries requestId) — emit directly, not via runner
            yield Ok::<_, std::convert::Infallible>(
                format!("{}\n", json!({"type": "connected", "requestId": request_id_clone, "sessionId": session_id_clone, "projectPath": working_directory, "workspace": workspace}))
            );

            let mut ping_interval = tokio::time::interval(tokio::time::Duration::from_secs(25));
            ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ping_interval.tick().await; // consume immediate first tick

            loop {
                tokio::select! {
                    result = live_rx.recv() => {
                        match result {
                            Ok(line) => { yield Ok(line); }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!("POST subscriber lagged by {} messages", n);
                            }
                            Err(broadcast::error::RecvError::Closed) => { break; }
                        }
                    }
                    _ = ping_interval.tick() => {
                        yield Ok(format!("{}\n", json!({"type": "ping"})));
                    }
                }
            }
        };

        let body = Body::from_stream(stream);
        Response::builder()
            .header("content-type", "application/x-ndjson")
            .header("transfer-encoding", "chunked")
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no")
            .header("connection", "close")
            .body(body)
            .unwrap()
    })
}

/// Re-enters the normal authenticated chat route for a queued or interrupted run.
/// The response body is intentionally dropped: execution is owned by the server,
/// and all observable output is delivered through the durable event journal.
pub async fn submit_durable_chat_run(
    state: Arc<AppState>,
    run: crate::db::realtime::DurableChatRun,
) {
    let mut request = run.request;
    let Some(body) = request.as_object_mut() else {
        crate::db::realtime::update_run_state(
            &state.db,
            &run.id,
            "failed",
            Some("invalid durable request"),
        );
        return;
    };
    body.insert("requestId".to_string(), json!(run.id));
    body.insert("sessionId".to_string(), json!(run.session_id));
    let input_recovery = body.get("inputRecovery").and_then(|recovery| {
        Some((
            recovery.get("requestId")?.as_str()?.to_string(),
            recovery.get("deliveryOwner")?.as_str()?.to_string(),
        ))
    });
    if run.state == "recovering" && input_recovery.is_none() {
        body.insert(
            "message".to_string(),
            json!(crate::db::realtime::RECOVERY_MESSAGE),
        );
        body.insert("hiddenContinuation".to_string(), json!(true));
    }

    let mut headers = HeaderMap::new();
    let Ok(identity) = run.user_id.parse() else {
        return;
    };
    headers.insert("x-flamel-user-id", identity);
    let response = handle_chat(State(state.clone()), headers, Json(request)).await;
    let status = response.status();
    let accepted = status.is_success()
        || (status == StatusCode::CONFLICT
            && axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .ok()
                .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
                .is_some_and(|body| accepted_durable_duplicate(&body, &run.id, &run.session_id)));
    let dispatch_error = (!accepted).then(|| format!("durable run dispatch returned {status}"));
    if let Some(error) = dispatch_error {
        tracing::error!(run_id = run.id, %error, "durable run dispatch failed");
        if let Err(storage_error) = crate::db::realtime::record_dispatch_error(
            &state.db,
            &run.id,
            &run.session_id,
            &run.user_id,
            &error,
        ) {
            tracing::error!(run_id=run.id,%storage_error,"durable dispatch error could not be saved");
        }
    } else if let Some((request_id, owner)) = input_recovery {
        if let Err(error) =
            crate::db::input_requests::mark_consumed(&state.db, &request_id, &run.user_id, &owner)
        {
            tracing::error!(
                run_id = run.id,
                request_id,
                "durable input recovery acknowledgement failed: {error}"
            );
        }
    }
}

fn accepted_durable_duplicate(body: &Value, request_id: &str, session_id: &str) -> bool {
    body.get("accepted").and_then(Value::as_bool) == Some(true)
        && body.get("code").and_then(Value::as_str) == Some("request_already_accepted")
        && body.get("requestId").and_then(Value::as_str) == Some(request_id)
        && body.get("sessionId").and_then(Value::as_str) == Some(session_id)
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    #[test]
    fn only_the_exact_accepted_duplicate_acknowledges_dispatch() {
        let response = json!({"accepted":true,"code":"request_already_accepted",
            "requestId":"run","sessionId":"session"});
        assert!(accepted_durable_duplicate(&response, "run", "session"));
        assert!(!accepted_durable_duplicate(&response, "other", "session"));
        assert!(!accepted_durable_duplicate(&response, "run", "other"));
        for (key, value) in [
            ("accepted", json!(false)),
            ("code", json!("request_id_conflict")),
        ] {
            let mut rejected = response.clone();
            rejected[key] = value;
            assert!(!accepted_durable_duplicate(&rejected, "run", "session"));
        }
        assert!(!accepted_durable_duplicate(&Value::Null, "run", "session"));
    }
}

pub fn read_claude_mcp_servers(home: &str, project_path: &str) -> Value {
    // Skip ~/.claude.json MCP servers entirely — flamel manages its own MCP servers
    // through built-ins (flamel, git, docs, db, browser) and the UI config.
    // Reading from .claude.json causes stale servers (serena, sequential-thinking)
    // to get injected, bloating the system prompt with their instructions.
    let _ = (home, project_path);
    json!({})
}
