/// Background agent executor — polls for pending agent runs and executes them with Claude CLI.
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::Duration;

use super::codex_provider;
use super::git_ops;
use super::provider::{
    NormalizedEvent, ProviderCredentials, ProviderKind, SpawnOpts, get_provider,
};
use crate::AppState;
use crate::db::{agents, capabilities::effective_mcp_servers, git_accounts, mcp::list_mcp_configs};

const POLL_INTERVAL: Duration = Duration::from_secs(15);
/// Hard cap: kill agent Claude process if it hasn't finished in 30 minutes.
const AGENT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Spawn the background agent runner. Call once at startup.
pub fn start_agent_runner(state: Arc<AppState>) {
    tokio::spawn(async move {
        // Initial delay so server is fully up before first poll
        tokio::time::sleep(Duration::from_secs(5)).await;

        loop {
            if state.is_shutting_down() {
                break;
            }
            poll_and_run(&state).await;
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

async fn poll_and_run(state: &Arc<AppState>) {
    let pending = agents::find_pending_runs(&state.db);
    if pending.is_empty() {
        return;
    }

    tracing::info!("agent_runner: {} pending run(s) found", pending.len());

    for run in pending {
        // Atomically claim the run so a concurrent instance can't double-execute
        if !agents::try_claim_run(&state.db, &run.id) {
            tracing::debug!("agent_runner: run {} already claimed, skipping", run.id);
            continue;
        }

        // Broadcast that the run is now claimed (running)
        state.agent_pool.emit(serde_json::json!({
            "type": "agent_status",
            "runId": run.id,
            "status": "running",
        }));

        let state_clone = Arc::clone(state);
        let sem = Arc::clone(&state.agent_pool.semaphore);
        let run_id = run.id.clone();

        tokio::spawn(async move {
            // Acquire concurrency slot — blocks if MAX_CONCURRENT_AGENTS already running
            let _permit = match sem.acquire().await {
                Ok(p) => p,
                Err(_) => return, // semaphore closed
            };
            tracing::info!("agent_runner: executing run {}", run_id);
            execute_run(state_clone, run).await;
            // _permit drops here, releasing the slot
        });
    }
}

async fn execute_run(state: Arc<AppState>, run: agents::AgentRun) {
    let config = match agents::get_agent_config(&state.db, &run.agent_config_id) {
        Some(c) => c,
        None => {
            agents::update_agent_run_status(
                &state.db,
                &run.id,
                "failed",
                Some("Agent config not found"),
            );
            state.agent_pool.emit(serde_json::json!({"type": "agent_status", "runId": run.id, "status": "failed", "error": "Agent config not found"}));
            return;
        }
    };

    // Extract issue number from payload for lifecycle comments
    let issue_number = run
        .trigger_payload
        .get("issue")
        .and_then(|i| i.get("number"))
        .and_then(|n| n.as_i64());

    // Look up git account for API calls (comments, close)
    let account = git_accounts::get_git_account_with_token(
        &state.db,
        &state.credential_vault,
        &config.git_account_id,
    );

    // Post "working on it" comment
    if config.comment_on_start {
        if let (Some(number), Some(acct)) = (issue_number, &account) {
            git_ops::post_issue_comment(
                acct,
                &config.repo_owner,
                &config.repo_name,
                number,
                "🤖 **flamel** is working on this...",
            )
            .await;
            git_ops::add_label(
                acct,
                &config.repo_owner,
                &config.repo_name,
                number,
                "in-progress",
            )
            .await;
        }
    }

    // Build a descriptive prompt from the trigger payload
    let prompt = build_prompt(&config, &run);

    let provider_kind = ProviderKind::from_model(&config.model);
    let provider = get_provider(provider_kind);
    let agent_user_id = if !config.user_id.is_empty() {
        config.user_id.clone()
    } else {
        account
            .as_ref()
            .and_then(|acct| acct.user_id.clone())
            .unwrap_or_default()
    };
    if !state.local_mode && agent_user_id.is_empty() {
        let msg = "Hosted agent has no account owner; refusing to use the server home";
        agents::update_agent_run_status(&state.db, &run.id, "failed", Some(msg));
        state.agent_pool.emit(serde_json::json!({"type": "agent_status", "runId": run.id, "status": "failed", "error": msg}));
        post_failure_comment(&config, &account, issue_number, msg).await;
        return;
    }
    let sandbox_required = false;
    let user_home = state.home_dir.clone();
    let user_home_str = user_home.to_string_lossy().into_owned();
    let provider_creds =
        load_agent_provider_credentials(&state, provider_kind, &agent_user_id).await;
    let system_prompt = crate::routes::chat::system_prompt_with_codex_startup_memory_reminder(
        &crate::routes::chat::build_system_prompt(&config.project_path, provider_kind),
        provider_kind,
        "persisted_context",
    );
    let mcp_servers = build_agent_mcp_servers(
        state.as_ref(),
        &run.id,
        &config.project_path,
        &agent_user_id,
        &user_home,
        provider_kind,
        &system_prompt,
    );

    let cli = match provider.find_binary() {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("Failed to find {} CLI: {e}", provider_kind);
            tracing::error!("agent_runner [{}]: {}", run.id, msg);
            agents::update_agent_run_status(&state.db, &run.id, "failed", Some(&msg));
            state.agent_pool.emit(serde_json::json!({"type": "agent_status", "runId": run.id, "status": "failed", "error": msg}));
            post_failure_comment(&config, &account, issue_number, &msg).await;
            return;
        }
    };
    let opts = SpawnOpts {
        model: Some(&config.model),
        reasoning_effort: Some("high"),
        permission_mode: &config.permission_mode,
        system_prompt_append: Some(&system_prompt),
        resume_session_id: None,
        mcp_servers: Some(&mcp_servers),
        user_home: &user_home_str,
        session_id: &run.id,
        provider_base_url: None,
        message: Some(&prompt),
    };
    let args = provider.build_args(&opts);
    let mut env_vars = provider.build_env(
        provider_creds.as_ref(),
        Some(state.db.as_ref()),
        &user_home_str,
        Some(&mcp_servers),
        opts.reasoning_effort,
    );

    let mut command = tokio::process::Command::new(&cli);
    command.args(&args);
    crate::mcp_parent_guard::configure_tokio_child(&mut command);
    let mut child = match command
        .current_dir(&config.project_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .envs(env_vars)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Failed to spawn {} CLI: {e}", provider_kind);
            tracing::error!("agent_runner [{}]: {}", run.id, msg);
            agents::update_agent_run_status(&state.db, &run.id, "failed", Some(&msg));
            state.agent_pool.emit(serde_json::json!({"type": "agent_status", "runId": run.id, "status": "failed", "error": msg}));
            post_failure_comment(&config, &account, issue_number, &msg).await;
            return;
        }
    };

    // Register PID for kill support
    if let Some(pid) = child.id() {
        state.agent_pool.register(&run.id, pid).await;
    }

    if let Some(stderr) = child.stderr.take() {
        let run_id_for_stderr = run.id.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    tracing::debug!("agent_runner [{}] stderr: {}", run_id_for_stderr, trimmed);
                }
            }
        });
    }

    let mut stdin = child.stdin.take();

    if provider_kind == ProviderKind::Claude {
        if let Some(stdin) = stdin.as_mut() {
            let msg = serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": prompt }
            });
            let mut bytes = serde_json::to_vec(&msg).unwrap_or_default();
            bytes.push(b'\n');
            let _ = stdin.write_all(&bytes).await;
            let _ = stdin.flush().await;
        }
        stdin = None;
    } else if let Some(stdin) = stdin.as_mut() {
        let init_bytes = codex_provider::build_init_sequence();
        let thread_start_bytes = codex_provider::build_thread_start(
            Some(&config.model),
            &config.project_path,
            Some(&system_prompt),
            opts.reasoning_effort,
            &config.permission_mode,
        );
        let _ = stdin.write_all(&init_bytes).await;
        let _ = stdin.write_all(&thread_start_bytes).await;
        let _ = stdin.flush().await;
    }

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            agents::update_agent_run_status(&state.db, &run.id, "failed", Some("No stdout pipe"));
            state.agent_pool.emit(serde_json::json!({"type": "agent_status", "runId": run.id, "status": "failed", "error": "No stdout pipe"}));
            state.agent_pool.unregister(&run.id).await;
            post_failure_comment(
                &config,
                &account,
                issue_number,
                "Agent process had no stdout pipe",
            )
            .await;
            return;
        }
    };

    let mut pr_url: Option<String> = None;
    let mut error: Option<String> = None;
    let mut finished = false;
    let mut codex_thread_id: Option<String> = None;
    let mut codex_first_turn_started = false;
    let run_id_for_read = run.id.clone();
    let pool_for_read = &state.agent_pool;

    let read_fut = async {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let msg: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if provider_kind == ProviderKind::Codex {
                let Some(event) = provider.parse_event(trimmed) else {
                    continue;
                };
                match event {
                    NormalizedEvent::Init { session_id } => {
                        codex_thread_id = Some(session_id.clone());
                        if !codex_first_turn_started {
                            if let Some(stdin) = stdin.as_mut() {
                                let turn_bytes = codex_provider::build_turn_start(
                                    &session_id,
                                    &prompt,
                                    &[],
                                    Some(&config.model),
                                    opts.reasoning_effort,
                                    &config.permission_mode,
                                );
                                let _ = stdin.write_all(&turn_bytes).await;
                                let _ = stdin.flush().await;
                                codex_first_turn_started = true;
                            }
                        }
                    }
                    NormalizedEvent::AssistantText { text, complete } => {
                        if complete {
                            if pr_url.is_none() {
                                pr_url = extract_pr_url(&text);
                            }
                            for text_line in text.lines().take(20) {
                                if !text_line.trim().is_empty() {
                                    pool_for_read.emit(serde_json::json!({
                                        "type": "agent_output",
                                        "runId": run_id_for_read,
                                        "line": if text_line.len() > 500 { &text_line[..500] } else { text_line },
                                    }));
                                }
                            }
                        }
                    }
                    NormalizedEvent::ToolUse { name, .. } => {
                        pool_for_read.emit(serde_json::json!({
                            "type": "agent_tool",
                            "runId": run_id_for_read,
                            "tool": name,
                        }));
                    }
                    NormalizedEvent::McpToolCall { server, tool, .. } => {
                        let tool_name = if server.is_empty() {
                            tool
                        } else {
                            format!("{server}.{tool}")
                        };
                        pool_for_read.emit(serde_json::json!({
                            "type": "agent_tool",
                            "runId": run_id_for_read,
                            "tool": tool_name,
                        }));
                    }
                    NormalizedEvent::Result {
                        is_error,
                        error_message,
                        ..
                    } => {
                        finished = true;
                        if is_error {
                            error = Some(
                                error_message.unwrap_or_else(|| "Codex turn failed".to_string()),
                            );
                        }
                        break;
                    }
                    NormalizedEvent::Error { message } => {
                        finished = true;
                        error = Some(message);
                        break;
                    }
                    NormalizedEvent::Raw { data } => {
                        if data
                            .get("__codex_approval_request")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        {
                            if let (Some(method), Some(request_id), Some(params)) = (
                                data.get("method").and_then(|v| v.as_str()),
                                data.get("request_id"),
                                data.get("params"),
                            ) {
                                let approved = matches!(
                                    config.permission_mode.as_str(),
                                    "bypassPermissions" | "never"
                                );
                                if let Some(bytes) = codex_provider::build_approval_response(
                                    method, request_id, params, approved,
                                ) {
                                    if let Some(stdin) = stdin.as_mut() {
                                        let _ = stdin.write_all(&bytes).await;
                                        let _ = stdin.flush().await;
                                    }
                                }
                            }
                        } else if data
                            .get("__codex_thread_closed")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        {
                            finished = true;
                            let thread_label = data
                                .get("thread_id")
                                .and_then(|v| v.as_str())
                                .or(codex_thread_id.as_deref())
                                .unwrap_or("unknown");
                            error =
                                Some(format!("Codex thread closed unexpectedly ({thread_label})"));
                            break;
                        }
                    }
                    _ => {}
                }
            } else {
                let ev_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                // Claude stream-json events
                match ev_type {
                    "assistant" => {
                        if let Some(content) = msg
                            .get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_array())
                        {
                            for block in content {
                                match block.get("type").and_then(|t| t.as_str()) {
                                    Some("text") => {
                                        if let Some(text) =
                                            block.get("text").and_then(|t| t.as_str())
                                        {
                                            if pr_url.is_none() {
                                                pr_url = extract_pr_url(text);
                                            }
                                            for text_line in text.lines().take(20) {
                                                if !text_line.trim().is_empty() {
                                                    pool_for_read.emit(serde_json::json!({
                                                        "type": "agent_output",
                                                        "runId": run_id_for_read,
                                                        "line": if text_line.len() > 500 { &text_line[..500] } else { text_line },
                                                    }));
                                                }
                                            }
                                        }
                                    }
                                    Some("tool_use") => {
                                        let tool_name = block
                                            .get("name")
                                            .and_then(|n| n.as_str())
                                            .unwrap_or("unknown");
                                        pool_for_read.emit(serde_json::json!({
                                            "type": "agent_tool",
                                            "runId": run_id_for_read,
                                            "tool": tool_name,
                                        }));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    "result" => {
                        finished = true;
                        let is_err = msg
                            .get("is_error")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if is_err {
                            error = Some(
                                msg.get("error")
                                    .and_then(|v| v.as_str())
                                    .or_else(|| msg.get("result").and_then(|v| v.as_str()))
                                    .unwrap_or("Claude returned an error")
                                    .to_string(),
                            );
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
    };

    if tokio::time::timeout(AGENT_TIMEOUT, read_fut).await.is_err() {
        tracing::warn!(
            "agent_runner [{}]: timed out after {:?}",
            run.id,
            AGENT_TIMEOUT
        );
        let _ = child.kill().await;
        state.agent_pool.unregister(&run.id).await;
        agents::update_agent_run_status(
            &state.db,
            &run.id,
            "failed",
            Some("Agent timed out (30 min limit)"),
        );
        state.agent_pool.emit(serde_json::json!({"type": "agent_status", "runId": run.id, "status": "failed", "error": "Agent timed out (30 min limit)"}));
        post_failure_comment(
            &config,
            &account,
            issue_number,
            "Agent timed out after 30 minutes.",
        )
        .await;
        return;
    }

    if provider_kind == ProviderKind::Codex {
        let _ = child.kill().await;
    }
    let _ = child.wait().await;
    state.agent_pool.unregister(&run.id).await;

    if !finished {
        let msg = if provider_kind == ProviderKind::Codex {
            "Codex process ended without a completed turn"
        } else {
            "Claude process ended without a result message"
        };
        agents::update_agent_run_status(&state.db, &run.id, "failed", Some(msg));
        state.agent_pool.emit(serde_json::json!({"type": "agent_status", "runId": run.id, "status": "failed", "error": msg}));
        post_failure_comment(&config, &account, issue_number, msg).await;
        return;
    }

    if let Some(ref err) = error {
        tracing::warn!(
            "agent_runner [{}]: {} error — {}",
            run.id,
            provider_kind,
            err
        );
        agents::update_agent_run_status(&state.db, &run.id, "failed", Some(err));
        state.agent_pool.emit(serde_json::json!({"type": "agent_status", "runId": run.id, "status": "failed", "error": err}));
        post_failure_comment(&config, &account, issue_number, err).await;
    } else {
        tracing::info!("agent_runner [{}]: done (pr_url={:?})", run.id, pr_url);
        agents::update_agent_run_with_pr(&state.db, &run.id, "done", pr_url.as_deref());
        state.agent_pool.emit(serde_json::json!({"type": "agent_status", "runId": run.id, "status": "done", "prUrl": pr_url}));

        // Post success comment and optionally close the issue
        if let (Some(number), Some(acct)) = (issue_number, &account) {
            let comment = if let Some(ref url) = pr_url {
                format!("✅ **flamel** finished working on this. PR opened: {url}")
            } else {
                "✅ **flamel** finished working on this. Changes have been committed.".to_string()
            };
            git_ops::post_issue_comment(
                acct,
                &config.repo_owner,
                &config.repo_name,
                number,
                &comment,
            )
            .await;

            if config.auto_close {
                git_ops::close_issue(acct, &config.repo_owner, &config.repo_name, number).await;
            }
        }
    }
}

pub(crate) async fn load_agent_provider_credentials(
    state: &Arc<AppState>,
    provider_kind: ProviderKind,
    user_id: &str,
) -> Option<ProviderCredentials> {
    if user_id.is_empty() {
        return None;
    }

    match provider_kind {
        ProviderKind::Claude => crate::routes::claude_auth::try_refresh_credentials(state, user_id)
            .await
            .map(|c| ProviderCredentials {
                user_id: c.user_id,
                access_token: c.access_token,
                refresh_token: c.refresh_token,
                expires_at: c.expires_at,
                scopes: c.scopes,
                extra: None,
            }),
        ProviderKind::Codex => {
            crate::routes::codex_auth::try_refresh_codex_credentials(state, user_id)
                .await
                .map(|pc| ProviderCredentials {
                    user_id: pc.user_id,
                    access_token: pc.access_token,
                    refresh_token: pc.refresh_token,
                    expires_at: pc.expires_at,
                    scopes: pc.scopes,
                    extra: pc.id_token.map(|t| json!({"id_token": t})),
                })
        }
    }
}

fn mcp_binary(name: &str) -> String {
    crate::bundled_binary_path(name)
        .to_string_lossy()
        .into_owned()
}

fn build_agent_mcp_servers(
    state: &AppState,
    run_id: &str,
    working_directory: &str,
    user_id: &str,
    user_home: &Path,
    provider_kind: ProviderKind,
    system_prompt: &str,
) -> Value {
    let unrestricted_shell = true;
    let home = if unrestricted_shell {
        std::env::var("HOME").unwrap_or_default()
    } else {
        user_home.to_string_lossy().into_owned()
    };
    let mut mcp_servers = crate::routes::chat::read_claude_mcp_servers(&home, working_directory);

    for config in list_mcp_configs(state.db.as_ref(), Some(working_directory), Some(user_id))
        .iter()
        .filter(|c| c.enabled)
    {
        mcp_servers.as_object_mut().unwrap().insert(
            config.name.clone(),
            json!({"command": config.command, "args": config.args}),
        );
    }
    for (name, config) in effective_mcp_servers(&state.db, user_id, working_directory) {
        mcp_servers.as_object_mut().unwrap().insert(name, config);
    }
    for retired in ["git", "docs", "db", "viz", "browser", "memory"] {
        mcp_servers.as_object_mut().unwrap().remove(retired);
    }

    let port = state.port;
    let provider_impl = get_provider(provider_kind);
    let provider_cli_path = if provider_kind == ProviderKind::Claude {
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
            "command": mcp_binary("flamel_mcp"),
            "args": [],
            "tool_timeout_sec": 900,
            "startup_timeout_sec": 30,
            "env": {
                "FLAMEL_URL": crate::callback::endpoint(&state),
                "FLAMEL_TOKEN": crate::mcp_auth::token_for_process(state, user_id, Some(run_id)),
                "FLAMEL_SESSION_ID": run_id,
                "FLAMEL_PROJECT_PATH": working_directory,
                "GIT_WORK_DIR": working_directory,
                "FLAMEL_USER_ID": user_id,
                "FLAMEL_USER_HOME": user_home.to_string_lossy(),
                "FLAMEL_SHELL_UNRESTRICTED": if unrestricted_shell { "1" } else { "0" },
                "FLAMEL_PROVIDER_BINARY": provider_cli_path,
                "FLAMEL_DISALLOWED_TOOLS": disallowed_tools_csv,
                "FLAMEL_AGENT_DEPTH": "0",
                "FLAMEL_PROVIDER": provider_kind.as_str(),
                "FLAMEL_CODEX_HOME": if provider_kind == ProviderKind::Codex {
                    user_home.join(".codex").to_string_lossy().into_owned()
                } else {
                    String::new()
                },
            }
        }),
    );

    let serialized_mcp = json!({"mcpServers": &mcp_servers}).to_string();
    if let Some(env) = mcp_servers
        .as_object_mut()
        .and_then(|servers| servers.get_mut("flamel"))
        .and_then(|entry| entry.get_mut("env"))
        .and_then(|env| env.as_object_mut())
    {
        env.insert(
            "FLAMEL_SYSTEM_PROMPT".to_string(),
            Value::String(system_prompt.to_string()),
        );
        env.insert(
            "FLAMEL_MCP_SERVERS".to_string(),
            Value::String(serialized_mcp),
        );
    }

    mcp_servers
}

async fn post_failure_comment(
    config: &agents::AgentConfig,
    account: &Option<git_accounts::GitAccount>,
    issue_number: Option<i64>,
    reason: &str,
) {
    if let (Some(number), Some(acct)) = (issue_number, account) {
        let body = format!("❌ **flamel** encountered an error:\n\n```\n{reason}\n```");
        git_ops::post_issue_comment(acct, &config.repo_owner, &config.repo_name, number, &body)
            .await;
    }
}

fn build_prompt(config: &agents::AgentConfig, run: &agents::AgentRun) -> String {
    let p = &run.trigger_payload;

    let action = p.get("action").and_then(|v| v.as_str()).unwrap_or("opened");

    let issue_number = p
        .get("issue")
        .and_then(|i| i.get("number"))
        .and_then(|n| n.as_i64());
    let issue_title = p
        .get("issue")
        .and_then(|i| i.get("title"))
        .and_then(|t| t.as_str())
        .unwrap_or("(no title)");
    let issue_body = p
        .get("issue")
        .and_then(|i| i.get("body"))
        .and_then(|b| b.as_str())
        .unwrap_or("(no description)");
    let issue_url = p
        .get("issue")
        .and_then(|i| i.get("html_url"))
        .and_then(|u| u.as_str())
        .unwrap_or("");

    let repo_full = format!("{}/{}", config.repo_owner, config.repo_name);

    let pr_instruction = if config.auto_pr {
        "\n\nWhen you have finished making changes:\n\
         1. Create a git commit with a clear message describing what you did.\n\
         2. Push the branch and open a pull request. Print the PR URL on its own line so it can be tracked."
    } else {
        "\n\nWhen you have finished making changes, commit them with a descriptive message."
    };

    format!(
        "You are an autonomous software engineering agent triggered by a GitHub event.\n\n\
         Repository: {repo_full}\n\
         Event: issue {action}\n\
         {issue_ref}Issue: {issue_title}\n\n\
         {issue_body}\n\n\
         {url_line}\
         The codebase is already checked out at: {project_path}\n\n\
         Your task:\n\
         1. Read and fully understand the issue.\n\
         2. Explore the codebase to understand the relevant code.\n\
         3. Implement a clean, correct solution.\n\
         4. Verify your changes make sense.\n\
         {pr_instruction}",
        repo_full = repo_full,
        action = action,
        issue_ref = issue_number.map(|n| format!("#{n} ")).unwrap_or_default(),
        issue_title = issue_title,
        issue_body = issue_body,
        url_line = if !issue_url.is_empty() {
            format!("Issue URL: {issue_url}\n\n")
        } else {
            String::new()
        },
        project_path = config.project_path,
        pr_instruction = pr_instruction,
    )
}

/// Scan text for a GitHub/Gitea PR URL (e.g. https://github.com/owner/repo/pull/42)
fn extract_pr_url(text: &str) -> Option<String> {
    for line in text.lines() {
        for word in line.split_whitespace() {
            let w = word.trim_matches(|c: char| {
                !c.is_alphanumeric() && c != '/' && c != ':' && c != '.' && c != '-' && c != '_'
            });
            if (w.starts_with("https://github.com/") || w.contains("/pull/"))
                && w.contains("/pull/")
            {
                return Some(w.to_string());
            }
        }
    }
    None
}
