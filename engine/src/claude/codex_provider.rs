//! OpenAI Codex CLI provider — persistent `codex app-server` over stdio.
//!
//! Spawns `codex app-server` (JSON-RPC over newline-delimited JSON on stdio)
//! and normalizes its events into the same format the frontend expects.
//! The process stays alive between turns — same warm-session model as Claude.

use super::credential_files::{
    CredentialMaterializationError, ensure_private_directory, write_private_file,
};
use super::provider::*;
use crate::mcp_lifecycle::{collect_mcp_process_env, is_sensitive_mcp_env_key};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Codex app-server's provider-native compactor must stay disabled. Flamel
/// owns the inspectable checkpoint and fresh-thread handoff instead.
pub const CODEX_NATIVE_COMPACTION_LIMIT_CONFIG: &str = "model_auto_compact_token_limit=1000000000";
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct CodexProvider {
    content_blocks: Mutex<Vec<Value>>,
}

impl CodexProvider {
    pub fn new() -> Self {
        Self {
            content_blocks: Mutex::new(Vec::new()),
        }
    }

    fn append_thinking(&self, text: &str) -> Vec<Value> {
        let mut blocks = self.content_blocks.lock().unwrap();
        if let Some(block) = blocks
            .last_mut()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("thinking"))
        {
            let existing = block.get("thinking").and_then(Value::as_str).unwrap_or("");
            let combined = merge_streamed_reasoning(existing, text);
            *block = json!({
                "type": "thinking",
                "thinking": combined,
            });
        } else {
            blocks.push(json!({
                "type": "thinking",
                "thinking": text,
            }));
        }
        blocks.clone()
    }

    fn upsert_tool_use_block(&self, id: &str, name: &str, input: Value) -> Vec<Value> {
        let mut blocks = self.content_blocks.lock().unwrap();
        let replacement = json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": input,
        });

        if let Some(block) = blocks.iter_mut().find(|block| {
            block.get("type").and_then(|v| v.as_str()) == Some("tool_use")
                && block.get("id").and_then(|v| v.as_str()) == Some(id)
        }) {
            *block = replacement;
        } else {
            blocks.push(replacement);
        }
        blocks.clone()
    }

    fn clear_blocks(&self) {
        self.content_blocks.lock().unwrap().clear();
    }

    fn snapshot(content: Vec<Value>) -> Value {
        json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": content,
            }
        })
    }
}

fn merge_streamed_reasoning(existing: &str, incoming: &str) -> String {
    if existing.is_empty() {
        return incoming.to_string();
    }
    if incoming.is_empty() {
        return existing.to_string();
    }

    // App-server versions differ here: some emit true deltas while others emit
    // the cumulative summary. A summary-part boundary can also arrive before a
    // replay of the current cumulative value. Treat prefixes and identical
    // values as snapshots so a reconnect or boundary never duplicates text.
    let existing_without_boundary = existing.trim_end_matches(char::is_whitespace);
    let incoming_without_boundary = incoming.trim_end_matches(char::is_whitespace);
    if existing_without_boundary == incoming_without_boundary {
        return existing_without_boundary.to_string();
    }
    if incoming.starts_with(existing) {
        return incoming.to_string();
    }
    if existing.starts_with(incoming) {
        return existing.to_string();
    }
    if incoming.starts_with(existing_without_boundary) {
        return incoming.to_string();
    }

    format!("{existing}{incoming}")
}

const CODEX_DISALLOWED_TOOLS: &[&str] = super::spawn::DISALLOWED_BUILTIN_TOOLS;

fn sanitize_reasoning_effort(value: Option<&str>) -> Option<&str> {
    match value {
        Some("low" | "medium" | "high" | "xhigh" | "max" | "ultra") => value,
        _ => None,
    }
}

fn codex_approval_policy(permission_mode: &str) -> Option<&'static str> {
    match permission_mode {
        "bypassPermissions" | "never" => Some("never"),
        "default" | "acceptEdits" | "plan" | "on-request" => Some("on-request"),
        "on-failure" => Some("on-failure"),
        "untrusted" => Some("untrusted"),
        _ => None,
    }
}

fn codex_sandbox_mode(permission_mode: &str) -> Option<&'static str> {
    match permission_mode {
        "bypassPermissions" => Some("danger-full-access"),
        "acceptEdits" | "plan" => Some("workspace-write"),
        _ => None,
    }
}

impl Provider for CodexProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Codex
    }

    fn find_binary(&self) -> Result<String, String> {
        if let Ok(p) = std::env::var("CODEX_CLI_PATH") {
            return crate::provider_cli::resolve_executable(&p)
                .ok_or_else(|| format!("Codex CLI override is not executable: {p}"));
        }

        if let Some(path) = crate::provider_cli::managed_binary("codex") {
            return Ok(path);
        }

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let candidates = [
            PathBuf::from("/usr/local/bin/codex"),
            PathBuf::from("/usr/bin/codex"),
            home.join(".flamel").join("bin").join("codex"),
            home.join(".flamel")
                .join("npm-global")
                .join("bin")
                .join("codex"),
            home.join(".npm-global").join("bin").join("codex"),
        ];

        for c in &candidates {
            if let Some(path) = crate::provider_cli::resolve_executable(&c.to_string_lossy()) {
                tracing::debug!("found codex CLI at: {}", path);
                return Ok(path);
            }
        }

        crate::provider_cli::resolve_executable("codex")
            .ok_or_else(|| "Codex CLI is not installed; Flamel is installing the latest version. Retry shortly.".to_string())
    }

    fn build_args(&self, opts: &SpawnOpts) -> Vec<String> {
        let mut args = vec![
            "app-server".to_string(),
            "-c".to_string(),
            // Provider-native search is the efficient path for public research.
            // The Browser MCP remains available for rendered-page interaction,
            // authenticated workflows, and runtime debugging.
            "web_search=\"live\"".to_string(),
            "-c".to_string(),
            "features.image_generation=true".to_string(),
            "-c".to_string(),
            // Flamel checkpoints context between turns and can recover from a
            // context-limit error with an inspectable summary. Keep Codex's
            // opaque automatic compactor out of that ownership path.
            CODEX_NATIVE_COMPACTION_LIMIT_CONFIG.to_string(),
        ];

        if let Some(base_url) = opts.provider_base_url {
            let base_url = escape_toml_str(base_url);
            args.extend([
                "-c".to_string(),
                "model_provider=\"flamel_openai\"".to_string(),
                "-c".to_string(),
                "model_providers.flamel_openai.name=\"Flamel OpenAI\"".to_string(),
                "-c".to_string(),
                format!("model_providers.flamel_openai.base_url=\"{base_url}\""),
                "-c".to_string(),
                "model_providers.flamel_openai.env_key=\"OPENAI_API_KEY\"".to_string(),
                "-c".to_string(),
                "model_providers.flamel_openai.wire_api=\"responses\"".to_string(),
                "-c".to_string(),
                "model_providers.flamel_openai.supports_websockets=false".to_string(),
            ]);
        }

        args
    }

    fn build_env(
        &self,
        creds: Option<&ProviderCredentials>,
        _db: Option<&crate::db::Db>,
        user_home: &str,
        mcp_servers: Option<&Value>,
        reasoning_effort: Option<&str>,
    ) -> Vec<(String, String)> {
        let mut env: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| k != "CODEX_HOME" && k != "NODE_OPTIONS" && k != "CLAUDECODE")
            .collect();
        crate::claude::provider::add_browser_automation_env(&mut env);

        let codex_home = if let Some(c) = creds {
            match write_codex_credentials(c, user_home) {
                Ok(home) => Some(home),
                Err(error) => {
                    tracing::error!("failed to materialize Codex credentials: {error}");
                    None
                }
            }
        } else if !user_home.is_empty() {
            let dir = std::path::PathBuf::from(user_home).join(".codex");
            std::fs::create_dir_all(&dir).ok();
            Some(dir.to_string_lossy().into_owned())
        } else {
            None
        };

        if let Some(ref home) = codex_home {
            if let Some(servers) = mcp_servers {
                for (key, value) in collect_mcp_process_env(servers) {
                    if let Some((_, existing_value)) = env
                        .iter_mut()
                        .find(|(existing_key, _)| existing_key == &key)
                    {
                        *existing_value = value;
                    } else {
                        env.push((key, value));
                    }
                }
                write_codex_mcp_config(servers, home, reasoning_effort);
            }
            env.push(("CODEX_HOME".to_string(), home.clone()));
        }

        env
    }

    fn format_input(&self, _message: &str, _images: &[ImageBlock]) -> Vec<u8> {
        // Turn/start messages are constructed by session_pool using build_turn_start()
        Vec::new()
    }

    fn parse_event(&self, line: &str) -> Option<NormalizedEvent> {
        let event = parse_jsonrpc_message(line)?;
        if matches!(
            &event,
            NormalizedEvent::Raw { data }
                if data
                    .get("__codex_turn_started")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        ) {
            // A resumed app-server may replay items from prior turns before the new
            // turn begins. Never let those blocks carry into the current snapshot.
            self.clear_blocks();
        }
        Some(event)
    }

    fn serialize_for_frontend(&self, event: &NormalizedEvent) -> Option<Value> {
        match event {
            NormalizedEvent::Init { session_id } => Some(json!({
                "type": "system",
                "subtype": "init",
                "session_id": session_id,
            })),

            NormalizedEvent::AssistantText { text, complete } => {
                let mut blocks = self.content_blocks.lock().unwrap();
                let last_is_text = blocks
                    .last()
                    .and_then(|b| b.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("text");

                if *complete {
                    if last_is_text {
                        let existing = blocks
                            .last()
                            .unwrap()
                            .get("text")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        if text.starts_with(existing) || existing.is_empty() {
                            *blocks.last_mut().unwrap() = json!({"type": "text", "text": text});
                        } else {
                            blocks.push(json!({"type": "text", "text": text}));
                        }
                    } else {
                        blocks.push(json!({"type": "text", "text": text}));
                    }
                } else if last_is_text {
                    let existing = blocks
                        .last()
                        .unwrap()
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("");
                    let combined = format!("{}{}", existing, text);
                    *blocks.last_mut().unwrap() = json!({"type": "text", "text": combined});
                } else {
                    blocks.push(json!({"type": "text", "text": text}));
                }

                let snap = blocks.clone();
                drop(blocks);
                Some(Self::snapshot(snap))
            }

            NormalizedEvent::Thinking { text } => {
                let blocks = self.append_thinking(text);
                Some(Self::snapshot(blocks))
            }

            NormalizedEvent::ThinkingRedacted => {
                let mut blocks = self.content_blocks.lock().unwrap();
                let last_is_redacted_thinking = blocks.last().is_some_and(|block| {
                    block.get("type").and_then(|value| value.as_str()) == Some("thinking")
                        && block
                            .get("redacted")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false)
                        && block
                            .get("thinking")
                            .and_then(|value| value.as_str())
                            .unwrap_or("")
                            .is_empty()
                });

                if !last_is_redacted_thinking {
                    blocks.push(json!({
                        "type": "thinking",
                        "thinking": "",
                        "redacted": true,
                    }));
                }

                let snap = blocks.clone();
                drop(blocks);
                Some(Self::snapshot(snap))
            }

            NormalizedEvent::ToolUse { id, name, input } => {
                let blocks = self.upsert_tool_use_block(id, name, input.clone());
                Some(Self::snapshot(blocks))
            }

            NormalizedEvent::ToolResult {
                tool_use_id,
                output,
                presentation,
                is_error,
            } => {
                let mut result = json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": output,
                    "is_error": is_error,
                });
                if let Some(presentation) = presentation {
                    result["presentation"] = json!(presentation);
                }
                Some(result)
            }

            NormalizedEvent::ToolOutputDelta {
                tool_use_id,
                output,
            } => Some(json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": output,
                "is_error": false,
                "same_turn": true,
                "is_delta": true,
            })),

            NormalizedEvent::McpToolCall {
                server,
                tool,
                arguments,
                result,
                error,
                status,
                id: call_id,
            } => {
                let name = format!("mcp__{}__{}", server, tool);
                let id = call_id.clone().unwrap_or_else(|| {
                    format!(
                        "codex_mcp_{}_{}_{}",
                        server,
                        tool,
                        chrono::Utc::now().timestamp_millis()
                    )
                });

                if status == "in_progress" || status == "inProgress" {
                    let blocks = self.upsert_tool_use_block(&id, &name, arguments.clone());
                    Some(Self::snapshot(blocks))
                } else {
                    if !arguments.is_null() {
                        self.upsert_tool_use_block(&id, &name, arguments.clone());
                    }
                    let (output, presentation) = if let Some(err) = error {
                        normalize_tool_result(&Value::String(err.clone()))
                    } else if let Some(result) = result {
                        normalize_tool_result(result)
                    } else {
                        normalize_tool_result(&Value::String(String::new()))
                    };
                    Some(json!({
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": output,
                        "presentation": presentation,
                        "is_error": error.is_some(),
                    }))
                }
            }

            NormalizedEvent::Result {
                session_id,
                usage,
                stop_reason,
                is_error,
                error_message,
            } => {
                self.clear_blocks();
                let mut result = json!({
                    "type": "result",
                    "is_error": is_error,
                });
                if let Some(sid) = session_id {
                    result["session_id"] = json!(sid);
                }
                if let Some(u) = usage {
                    result["usage"] = json!({
                        "input_tokens": u.input_tokens,
                        "output_tokens": u.output_tokens,
                    });
                    if let Some(cached) = u.cached_input_tokens {
                        result["usage"]["cache_read_input_tokens"] = json!(cached);
                    }
                }
                if let Some(reason) = stop_reason {
                    result["stop_reason"] = json!(reason);
                }
                if let Some(msg) = error_message {
                    result["result"] = json!(msg);
                }
                Some(result)
            }

            NormalizedEvent::Usage {
                usage,
                context_window,
            } => Some(json!({
                "type": "usage",
                "usage": {
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                    "cache_read_input_tokens": usage.cached_input_tokens.unwrap_or(0),
                },
                "context_window": context_window,
            })),

            NormalizedEvent::Error { message } => Some(json!({
                "type": "result",
                "is_error": true,
                "stop_reason": "error",
                "result": message,
            })),

            NormalizedEvent::Compacting => Some(json!({
                "type": "system_status",
                "status": "compacting",
            })),

            NormalizedEvent::StreamEvent { raw } => Some(raw.clone()),
            NormalizedEvent::Raw { data } => Some(data.clone()),
        }
    }

    fn disallowed_tools_csv(&self) -> String {
        CODEX_DISALLOWED_TOOLS.join(",")
    }

    fn default_model(&self) -> &str {
        "gpt-5.6-sol"
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC message builders
// ---------------------------------------------------------------------------

/// Thread-safe request ID counter shared across all Codex sessions.
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

fn next_request_id() -> u64 {
    REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

fn jsonrpc_request(method: &str, params: Value) -> Vec<u8> {
    let id = next_request_id();
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let mut bytes = serde_json::to_vec(&msg).unwrap();
    bytes.push(b'\n');
    bytes
}

fn jsonrpc_notification(method: &str, params: Option<Value>) -> Vec<u8> {
    let mut msg = json!({
        "jsonrpc": "2.0",
        "method": method,
    });
    if let Some(p) = params {
        msg["params"] = p;
    }
    let mut bytes = serde_json::to_vec(&msg).unwrap();
    bytes.push(b'\n');
    bytes
}

fn jsonrpc_response(id: &Value, result: Value) -> Vec<u8> {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    let mut bytes = serde_json::to_vec(&msg).unwrap();
    bytes.push(b'\n');
    bytes
}

/// Build the initialization sequence: initialize request + initialized notification.
/// Returns the bytes to write to stdin.
pub fn build_init_sequence() -> Vec<u8> {
    let mut bytes = Vec::new();

    // 1. initialize request
    bytes.extend(jsonrpc_request(
        "initialize",
        json!({
            "clientInfo": {
                "name": "flamel",
                "version": "1.0.0",
            },
        }),
    ));

    // 2. initialized notification (sent immediately, don't wait for response)
    bytes.extend(jsonrpc_notification("initialized", None));

    bytes
}

/// Build a thread/start request.
pub fn build_thread_start(
    model: Option<&str>,
    cwd: &str,
    system_prompt: Option<&str>,
    reasoning_effort: Option<&str>,
    permission_mode: &str,
) -> Vec<u8> {
    let reasoning_effort = sanitize_reasoning_effort(reasoning_effort).unwrap_or("high");
    let mut params = json!({
        "cwd": cwd,
        "modelReasoningEffort": reasoning_effort,
    });

    if let Some(policy) = codex_approval_policy(permission_mode) {
        params["approvalPolicy"] = json!(policy);
    }
    if let Some(sandbox) = codex_sandbox_mode(permission_mode) {
        params["sandbox"] = json!(sandbox);
    }
    if let Some(m) = model {
        params["model"] = json!(m);
    }

    if let Some(sp) = system_prompt {
        if !sp.is_empty() {
            params["developerInstructions"] = json!(sp);
        }
    }

    jsonrpc_request("thread/start", params)
}

/// Build a thread/resume request.
pub fn build_thread_resume(
    thread_id: &str,
    model: Option<&str>,
    cwd: &str,
    system_prompt: Option<&str>,
    reasoning_effort: Option<&str>,
    permission_mode: &str,
) -> Vec<u8> {
    let mut params = json!({
        "threadId": thread_id,
        "cwd": cwd,
    });

    if let Some(policy) = codex_approval_policy(permission_mode) {
        params["approvalPolicy"] = json!(policy);
    }
    if let Some(sandbox) = codex_sandbox_mode(permission_mode) {
        params["sandbox"] = json!(sandbox);
    }
    if let Some(effort) = sanitize_reasoning_effort(reasoning_effort) {
        params["modelReasoningEffort"] = json!(effort);
    }
    if let Some(m) = model {
        params["model"] = json!(m);
    }

    if let Some(sp) = system_prompt {
        if !sp.is_empty() {
            params["developerInstructions"] = json!(sp);
        }
    }

    jsonrpc_request("thread/resume", params)
}

/// Build a turn/start request for sending a message on an existing thread.
pub fn build_turn_start(
    thread_id: &str,
    message: &str,
    images: &[ImageBlock],
    model: Option<&str>,
    reasoning_effort: Option<&str>,
    permission_mode: &str,
) -> Vec<u8> {
    let mut input = Vec::new();

    if !message.is_empty() {
        input.push(json!({
            "type": "text",
            "text": message,
            "text_elements": [],
        }));
    }

    for image in images {
        if image.data.trim().is_empty() {
            continue;
        }
        input.push(json!({
            "type": "image",
            "url": format!("data:{};base64,{}", image.media_type, image.data),
        }));
    }

    let mut params = json!({
        "threadId": thread_id,
        "input": input,
        "summary": "detailed",
    });

    if let Some(m) = model {
        params["model"] = json!(m);
    }
    if let Some(effort) = sanitize_reasoning_effort(reasoning_effort) {
        params["effort"] = json!(effort);
    }
    if let Some(policy) = codex_approval_policy(permission_mode) {
        params["approvalPolicy"] = json!(policy);
    }

    jsonrpc_request("turn/start", params)
}

/// Build a turn/interrupt request to cancel a running turn.
pub fn build_turn_interrupt(thread_id: &str, turn_id: &str) -> Vec<u8> {
    jsonrpc_request(
        "turn/interrupt",
        json!({
            "threadId": thread_id,
            "turnId": turn_id,
        }),
    )
}

/// Build a response for an approval request from the server.
pub fn build_approval_response(
    method: &str,
    request_id: &Value,
    params: &Value,
    approved: bool,
) -> Option<Vec<u8>> {
    let result = match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            json!({ "decision": if approved { "accept" } else { "decline" } })
        }
        "item/applyPatch/requestApproval" | "applyPatchApproval" => {
            json!({ "decision": if approved { "approved" } else { "denied" } })
        }
        "execCommandApproval" => {
            json!({ "decision": if approved { "approved" } else { "denied" } })
        }
        "item/permissions/requestApproval" | "permissions/requestApproval" => json!({
            "permissions": if approved {
                params.get("permissions").cloned().unwrap_or_else(|| json!({}))
            } else {
                json!({})
            },
            "scope": if approved { "session" } else { "turn" },
        }),
        _ => return None,
    };

    Some(jsonrpc_response(request_id, result))
}

// ---------------------------------------------------------------------------
// JSON-RPC event parsing
// ---------------------------------------------------------------------------

/// Parse a single stdout line as a JSON-RPC message.
/// Returns NormalizedEvent for notifications, None for responses we handle internally.
fn parse_jsonrpc_message(line: &str) -> Option<NormalizedEvent> {
    let v: Value = serde_json::from_str(line).ok()?;

    if let Some(event) = parse_raw_codex_event(&v) {
        return Some(event);
    }

    // JSON-RPC response (to one of our requests)
    if v.get("id").is_some() && v.get("result").is_some() {
        return parse_response(&v);
    }

    // JSON-RPC error response
    if v.get("id").is_some() && v.get("error").is_some() {
        let err = extract_error_message(&v["error"])
            .unwrap_or_else(|| "Unknown JSON-RPC error".to_string());
        if is_missing_turn_id_error(&err) {
            tracing::warn!("codex app-server recoverable protocol error: {}", err);
            return None;
        }
        if is_resume_rejection_error(&err) {
            tracing::warn!("codex app-server rejected thread/resume: {}", err);
            return Some(NormalizedEvent::Raw {
                data: json!({
                    "__codex_resume_rejected": true,
                    "error": err,
                }),
            });
        }
        tracing::error!("codex app-server error: {}", err);
        return Some(NormalizedEvent::Error { message: err });
    }

    // JSON-RPC notification or server request
    let method = v.get("method")?.as_str()?;
    let params = v.get("params").cloned().unwrap_or(json!({}));

    // Server requests (have "id" + "method") — need approval response
    if v.get("id").is_some() {
        return parse_server_request(method, &v["id"], &params);
    }

    // Notifications
    parse_notification(method, &params)
}

fn parse_response(v: &Value) -> Option<NormalizedEvent> {
    let result = v.get("result")?;

    // thread/start response — extract thread ID
    if let Some(thread) = result.get("thread") {
        if let Some(id) = thread.get("id").and_then(|i| i.as_str()) {
            tracing::info!("codex thread started: {}", id);
            return Some(NormalizedEvent::Init {
                session_id: id.to_string(),
            });
        }
    }

    // turn/start response — keep the turn ID for later turn/interrupt.
    if let Some(turn) = result.get("turn") {
        if let Some(turn_id) = turn_id_from(turn) {
            return Some(NormalizedEvent::Raw {
                data: json!({
                    "__codex_turn_started": true,
                    "thread_id": thread_id_from(turn)
                        .or_else(|| thread_id_from(result))
                        .unwrap_or_default(),
                    "turn_id": turn_id,
                }),
            });
        }
        return None;
    }

    // initialize response — swallow
    if result.get("serverInfo").is_some() || result.get("capabilities").is_some() {
        tracing::info!("codex app-server initialized");
        return None;
    }

    // turn/interrupt response — swallow
    None
}

fn parse_server_request(method: &str, id: &Value, params: &Value) -> Option<NormalizedEvent> {
    match method {
        "item/commandExecution/requestApproval"
        | "item/fileChange/requestApproval"
        | "item/applyPatch/requestApproval"
        | "applyPatchApproval"
        | "execCommandApproval"
        | "item/permissions/requestApproval"
        | "permissions/requestApproval" => Some(NormalizedEvent::Raw {
            data: json!({
                "__codex_approval_request": true,
                "method": method,
                "request_id": id,
                "params": params,
            }),
        }),
        _ => {
            tracing::debug!("unhandled codex server request: {}", method);
            None
        }
    }
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(key).and_then(|v| v.as_str()))
        .map(str::to_string)
}

fn turn_id_from(value: &Value) -> Option<String> {
    string_field(value, &["id", "turnId", "turn_id"])
}

fn thread_id_from(value: &Value) -> Option<String> {
    string_field(value, &["threadId", "thread_id"]).or_else(|| {
        value
            .get("thread")
            .and_then(|thread| string_field(thread, &["id"]))
    })
}

fn extract_turn_stop_reason(turn: &Value) -> Option<String> {
    const STOP_REASON_KEYS: &[&str] = &[
        "stop_reason",
        "stopReason",
        "completion_reason",
        "completionReason",
        "finish_reason",
        "finishReason",
    ];

    STOP_REASON_KEYS
        .iter()
        .find_map(|key| turn.get(key).and_then(|v| v.as_str()))
        .map(|value| value.to_string())
        .or_else(|| {
            turn.get("result")
                .and_then(|result| {
                    STOP_REASON_KEYS
                        .iter()
                        .find_map(|key| result.get(key).and_then(|v| v.as_str()))
                })
                .map(|value| value.to_string())
        })
}

fn is_missing_turn_id_error(message: &str) -> bool {
    message.contains("missing field `turnId`") || message.contains("missing field turnId")
}

fn is_resume_rejection_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("invalid thread id")
        || normalized.contains("no rollout found for thread id")
}

fn extract_error_message(value: &Value) -> Option<String> {
    fn extract(value: &Value, depth: usize) -> Option<String> {
        if depth == 0 {
            return None;
        }

        match value {
            Value::String(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }
            Value::Array(items) => items.iter().find_map(|item| extract(item, depth - 1)),
            Value::Object(map) => {
                for key in [
                    "message",
                    "error",
                    "details",
                    "detail",
                    "cause",
                    "reason",
                    "description",
                    "text",
                ] {
                    if let Some(found) = map.get(key).and_then(|item| extract(item, depth - 1)) {
                        return Some(found);
                    }
                }

                map.values().find_map(|item| extract(item, depth - 1))
            }
            _ => None,
        }
    }

    extract(value, 6)
}

fn parse_raw_codex_event(value: &Value) -> Option<NormalizedEvent> {
    match value.get("type").and_then(|t| t.as_str()) {
        Some("response_item") | Some("responseItem") => {
            parse_codex_response_item(value.get("payload")?)
        }
        _ => None,
    }
}

fn parse_codex_response_item(item: &Value) -> Option<NormalizedEvent> {
    match item.get("type").and_then(|t| t.as_str())? {
        "message" => parse_assistant_message_item(item, true),
        "agent_message" | "agentMessage" => parse_assistant_message_item(item, false),
        "reasoning" => extract_reasoning_summary(item)
            .map(|text| NormalizedEvent::Thinking { text })
            .or(Some(NormalizedEvent::ThinkingRedacted)),
        _ => None,
    }
}

fn extract_reasoning_summary(item: &Value) -> Option<String> {
    let summary = item.get("summary")?;
    let text = match summary {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.as_str()
                    .or_else(|| part.get("text").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    };
    (!text.trim().is_empty()).then_some(text)
}

fn parse_assistant_message_item(
    item: &Value,
    require_assistant_role: bool,
) -> Option<NormalizedEvent> {
    if require_assistant_role
        && item.get("role").and_then(|role| role.as_str()) != Some("assistant")
    {
        return None;
    }

    let text = extract_message_text(item)?;
    Some(NormalizedEvent::AssistantText {
        text,
        complete: true,
    })
}

fn extract_message_text(item: &Value) -> Option<String> {
    if let Some(text) = item
        .get("text")
        .and_then(|text| text.as_str())
        .filter(|text| !text.trim().is_empty())
    {
        return Some(text.to_string());
    }

    match item.get("content")? {
        Value::String(text) if !text.trim().is_empty() => Some(text.to_string()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter_map(extract_message_part_text)
                .collect::<Vec<_>>()
                .join("");
            if text.trim().is_empty() {
                None
            } else {
                Some(text)
            }
        }
        _ => None,
    }
}

fn extract_message_part_text(part: &Value) -> Option<String> {
    if let Some(text) = part.as_str() {
        return (!text.is_empty()).then(|| text.to_string());
    }

    let part_type = part.get("type").and_then(|t| t.as_str());
    if matches!(part_type, Some("input_text" | "input_image" | "image")) {
        return None;
    }

    part.get("text")
        .or_else(|| part.get("content"))
        .and_then(|text| text.as_str())
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn parse_notification(method: &str, params: &Value) -> Option<NormalizedEvent> {
    match method {
        "agent_message/delta" | "item/agentMessage/delta" => {
            let delta = params.get("delta")?.as_str()?.to_string();
            Some(NormalizedEvent::AssistantText {
                text: delta,
                complete: false,
            })
        }

        // Reasoning summaries are explicitly model-generated for display. Raw reasoning
        // text remains private and is represented only by an activity marker.
        "reasoning_summary_text/delta"
        | "item/reasoningSummaryText/delta"
        | "item/reasoning/summaryTextDelta" => {
            let text = params.get("delta")?.as_str()?.to_string();
            Some(NormalizedEvent::Thinking { text })
        }

        "reasoning_summary_part_added" | "item/reasoning/summaryPartAdded" => {
            let summary_index = params
                .get("summaryIndex")
                .or_else(|| params.get("summary_index"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            (summary_index > 0).then(|| NormalizedEvent::Thinking {
                text: "\n\n".to_string(),
            })
        }

        // Raw private reasoning deltas can be interleaved with displayable summary
        // deltas. The reasoning item/start event already supplies the activity marker;
        // emitting another marker for every private delta splits one summary into many
        // cumulative blocks in the UI.
        "reasoning_text/delta" | "item/reasoningText/delta" | "item/reasoning/textDelta" => None,

        // Thread lifecycle
        "thread/started" => {
            let thread_id = params
                .get("thread")
                .and_then(|t| t.get("id"))
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            if !thread_id.is_empty() {
                Some(NormalizedEvent::Init {
                    session_id: thread_id,
                })
            } else {
                None
            }
        }

        "thread/closed" => {
            let thread_id = params
                .get("thread")
                .and_then(|t| t.get("id"))
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            tracing::info!(
                "codex thread closed: {}",
                if thread_id.is_empty() {
                    "<unknown>"
                } else {
                    &thread_id
                }
            );
            Some(NormalizedEvent::Raw {
                data: json!({
                    "__codex_thread_closed": true,
                    "thread_id": thread_id,
                }),
            })
        }

        // Turn lifecycle
        "turn/started" => {
            let turn = params.get("turn").unwrap_or(params);
            turn_id_from(turn).map(|turn_id| NormalizedEvent::Raw {
                data: json!({
                    "__codex_turn_started": true,
                    "thread_id": thread_id_from(turn)
                        .or_else(|| thread_id_from(params))
                        .unwrap_or_default(),
                    "turn_id": turn_id,
                }),
            })
        }

        "turn/plan/updated" => {
            let explanation = params
                .get("explanation")
                .and_then(Value::as_str)
                .unwrap_or("");
            let item_count = params
                .get("plan")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            tracing::warn!(
                "rejected codex-native update_plan event for turn {} ({} items)",
                params
                    .get("turnId")
                    .and_then(|value| value.as_str())
                    .unwrap_or("<unknown>"),
                item_count
            );
            Some(NormalizedEvent::Raw {
                data: json!({
                    "type": "workflow_event",
                    "event": {
                        "kind": "recovery.notice",
                        "source": "codex_native_plan_guard",
                        "payload": {
                            "nativePlanRejected": true,
                            "clearStalePlan": true,
                            "discardAssistantText": true,
                            "note": "Codex tried its native update_plan helper. Flamel rejected it because it would not persist to the Plan panel; retrying with the Flamel Plan MCP.",
                            "summary": explanation,
                            "threadId": params.get("threadId").cloned().unwrap_or(Value::Null),
                            "turnId": params.get("turnId").cloned().unwrap_or(Value::Null),
                            "itemCount": item_count,
                        }
                    }
                }),
            })
        }

        "turn/failed" => {
            let err_msg = extract_error_message(params.get("error").unwrap_or(params))
                .unwrap_or_else(|| "Turn failed (unknown reason)".to_string());
            tracing::error!("codex turn/failed: {}", err_msg);
            Some(NormalizedEvent::Result {
                session_id: None,
                usage: None,
                stop_reason: extract_turn_stop_reason(params),
                is_error: true,
                error_message: Some(err_msg),
            })
        }

        "turn/completed" => {
            let turn = params.get("turn").unwrap_or(params);
            let usage = turn.get("usage").map(|u| TokenUsage {
                input_tokens: u
                    .get("input_tokens")
                    .or_else(|| u.get("inputTokens"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                output_tokens: u
                    .get("output_tokens")
                    .or_else(|| u.get("outputTokens"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                cached_input_tokens: u
                    .get("cached_input_tokens")
                    .or_else(|| u.get("cachedInputTokens"))
                    .and_then(|v| v.as_i64()),
            });
            Some(NormalizedEvent::Result {
                session_id: None,
                usage,
                stop_reason: extract_turn_stop_reason(turn)
                    .or_else(|| extract_turn_stop_reason(params)),
                is_error: false,
                error_message: None,
            })
        }

        // Item lifecycle — tool use / command execution / file changes
        "item/started" | "item/completed" => parse_item_notification(method, params),

        // Raw Responses API items carry Codex assistant messages in current app-server builds.
        "rawResponseItem/completed" => params.get("item").and_then(parse_codex_response_item),

        // Streaming command output (app-server uses item/ prefix)
        "command_execution/output/delta"
        | "item/commandExecution/output/delta"
        | "item/commandExecution/outputDelta" => {
            let delta = params.get("delta")?.as_str()?.to_string();
            let item_id = params
                .get("itemId")
                .and_then(|i| i.as_str())
                .unwrap_or("codex_cmd")
                .to_string();
            Some(NormalizedEvent::ToolOutputDelta {
                tool_use_id: item_id,
                output: delta,
            })
        }

        // Streaming file change output
        "file_change/output/delta"
        | "item/fileChange/output/delta"
        | "item/fileChange/outputDelta" => {
            let delta = params.get("delta")?.as_str()?.to_string();
            let item_id = params
                .get("itemId")
                .and_then(|i| i.as_str())
                .unwrap_or("codex_file")
                .to_string();
            Some(NormalizedEvent::ToolOutputDelta {
                tool_use_id: item_id,
                output: delta,
            })
        }

        // Error notification
        "error" => {
            let msg = extract_error_message(params).unwrap_or_else(|| "Unknown error".to_string());
            Some(NormalizedEvent::Error { message: msg })
        }

        // Context compaction
        "context/compacted" => Some(NormalizedEvent::Raw {
            data: json!({
                "type": "system_status",
                "status": "compact_complete",
            }),
        }),

        // Thread status changes
        "thread/status/changed" => {
            let status = params
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");
            tracing::debug!("codex thread status: {}", status);
            None
        }

        "thread/tokenUsage/updated" => {
            let token_usage = params
                .get("tokenUsage")
                .or_else(|| params.get("token_usage"))?;
            // `total` is cumulative across the thread and can grow to millions
            // of tokens. `last` is the latest request and therefore reflects
            // the active context that competes with the model window.
            let current = token_usage
                .get("last")
                .or_else(|| token_usage.get("total"))?;
            Some(NormalizedEvent::Usage {
                usage: TokenUsage {
                    input_tokens: current
                        .get("totalTokens")
                        .or_else(|| current.get("total_tokens"))
                        .and_then(Value::as_i64)
                        .unwrap_or_else(|| {
                            current
                                .get("inputTokens")
                                .or_else(|| current.get("input_tokens"))
                                .and_then(Value::as_i64)
                                .unwrap_or(0)
                        }),
                    output_tokens: current
                        .get("outputTokens")
                        .or_else(|| current.get("output_tokens"))
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    cached_input_tokens: current
                        .get("cachedInputTokens")
                        .or_else(|| current.get("cached_input_tokens"))
                        .and_then(Value::as_i64),
                },
                context_window: token_usage
                    .get("modelContextWindow")
                    .or_else(|| token_usage.get("model_context_window"))
                    .and_then(Value::as_i64),
            })
        }

        // Informational — swallow silently
        "thread/name/updated"
        | "account/rateLimits/updated"
        | "mcpServer/startupStatus/updated" => None,

        _ => {
            tracing::debug!("unknown codex notification: {}", method);
            None
        }
    }
}

fn mcp_tool_arguments(item: &Value) -> Value {
    item.get("arguments")
        .or_else(|| item.get("input"))
        .or_else(|| item.get("args"))
        .cloned()
        .unwrap_or(Value::Null)
}

fn parse_item_notification(event_type: &str, params: &Value) -> Option<NormalizedEvent> {
    let item = params.get("item")?;
    let item_type = item.get("type")?.as_str()?;
    let item_id = item
        .get("id")
        .and_then(|i| i.as_str())
        .unwrap_or("codex_item")
        .to_string();

    match item_type {
        "agent_message" | "agentMessage" => {
            if event_type == "item/completed" {
                parse_assistant_message_item(item, false)
            } else {
                None
            }
        }

        "message" => {
            if event_type == "item/completed" {
                parse_assistant_message_item(item, true)
            } else {
                None
            }
        }

        "reasoning" => {
            if event_type == "item/completed" {
                extract_reasoning_summary(item)
                    .map(|text| NormalizedEvent::Thinking { text })
                    .or(Some(NormalizedEvent::ThinkingRedacted))
            } else if event_type == "item/started" {
                Some(NormalizedEvent::ThinkingRedacted)
            } else {
                None
            }
        }

        "webSearch" | "web_search" => {
            let query = item.get("query").and_then(Value::as_str).unwrap_or("");
            let action = item.get("action").cloned().unwrap_or(Value::Null);
            if event_type == "item/started" {
                Some(NormalizedEvent::ToolUse {
                    id: item_id,
                    name: "web_search".to_string(),
                    input: json!({
                        "query": query,
                        "action": action,
                    }),
                })
            } else if event_type == "item/completed" {
                let status = item
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("completed");
                Some(NormalizedEvent::ToolResult {
                    tool_use_id: item_id,
                    output: json!({
                        "query": query,
                        "action": action,
                        "status": status,
                    })
                    .to_string(),
                    presentation: None,
                    is_error: status.eq_ignore_ascii_case("failed"),
                })
            } else {
                None
            }
        }

        "mcp_tool_call" | "mcpToolCall" => {
            let server = item
                .get("server")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let tool = item
                .get("tool")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let arguments = mcp_tool_arguments(item);
            let result = item.get("result").cloned();
            let error = item.get("error").and_then(extract_error_message);
            let status = item
                .get("status")
                .and_then(|s| s.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if event_type == "item/completed" || result.is_some() || error.is_some() {
                        "completed".to_string()
                    } else {
                        "in_progress".to_string()
                    }
                });

            Some(NormalizedEvent::McpToolCall {
                server,
                tool,
                arguments,
                result,
                error,
                status,
                id: Some(item_id),
            })
        }

        "command_execution" | "commandExecution" => {
            let command = item
                .get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();

            if event_type == "item/started" {
                Some(NormalizedEvent::ToolUse {
                    id: item_id,
                    name: "shell".to_string(),
                    input: json!({"command": command}),
                })
            } else if event_type == "item/completed" {
                let output = item
                    .get("aggregated_output")
                    .or_else(|| item.get("aggregatedOutput"))
                    .or_else(|| item.get("output"))
                    .and_then(|o| o.as_str())
                    .unwrap_or("")
                    .to_string();
                let exit_code = item
                    .get("exit_code")
                    .or_else(|| item.get("exitCode"))
                    .and_then(|c| c.as_i64());
                let status = item.get("status").and_then(|s| s.as_str()).unwrap_or("");
                let is_error = status == "failed" || exit_code.map(|c| c != 0).unwrap_or(false);
                Some(NormalizedEvent::ToolResult {
                    tool_use_id: item_id,
                    output,
                    presentation: None,
                    is_error,
                })
            } else {
                None
            }
        }

        "file_change" | "fileChange" => {
            let changes = item.get("changes").cloned().unwrap_or(json!([]));

            if event_type == "item/started" {
                Some(NormalizedEvent::ToolUse {
                    id: item_id,
                    name: "file_edit".to_string(),
                    input: json!({"changes": changes}),
                })
            } else if event_type == "item/completed" {
                let status = item.get("status").and_then(|s| s.as_str()).unwrap_or("");
                let summary = if let Some(arr) = changes.as_array() {
                    arr.iter()
                        .filter_map(|c| {
                            let path = c.get("path")?.as_str()?;
                            let kind = c.get("kind")?.as_str()?;
                            Some(format!("{}: {}", kind, path))
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    String::new()
                };
                Some(NormalizedEvent::ToolResult {
                    tool_use_id: item_id,
                    output: summary,
                    presentation: None,
                    is_error: status == "failed",
                })
            } else {
                None
            }
        }

        "imageGeneration" | "image_generation" => {
            if event_type == "item/started" {
                let prompt = item
                    .get("revisedPrompt")
                    .or_else(|| item.get("revised_prompt"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                Some(NormalizedEvent::ToolUse {
                    id: item_id,
                    name: "image_gen".to_string(),
                    input: json!({"prompt": prompt}),
                })
            } else if event_type == "item/completed" {
                let status = item
                    .get("status")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let saved_path = item
                    .get("savedPath")
                    .or_else(|| item.get("saved_path"))
                    .and_then(|value| value.as_str());
                let output = saved_path
                    .map(|path| format!("Generated image saved to {path}"))
                    .unwrap_or_else(|| "Image generation completed".to_string());
                Some(NormalizedEvent::ToolResult {
                    tool_use_id: item_id,
                    output,
                    presentation: None,
                    is_error: status.eq_ignore_ascii_case("failed"),
                })
            } else {
                None
            }
        }

        // userMessage items — swallow (just echoes what we sent)
        "userMessage" | "user_message" => None,

        // Codex may emit this as an item during automatic compaction.
        "contextCompaction" | "context_compaction" => {
            if event_type == "item/completed" {
                Some(NormalizedEvent::Raw {
                    data: json!({
                        "type": "system_status",
                        "status": "compact_complete",
                    }),
                })
            } else {
                Some(NormalizedEvent::Compacting)
            }
        }

        "error" => {
            let msg = extract_error_message(item)
                .or_else(|| extract_error_message(params))
                .unwrap_or_else(|| "Unknown error".to_string());
            Some(NormalizedEvent::Error { message: msg })
        }

        _ => {
            tracing::info!("unknown codex item type: {}", item_type);
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Codex credential writing
// ---------------------------------------------------------------------------

pub(crate) fn write_codex_credentials(
    creds: &ProviderCredentials,
    user_home: &str,
) -> Result<String, CredentialMaterializationError> {
    let codex_home = if !user_home.is_empty() {
        PathBuf::from(user_home).join(".codex")
    } else {
        let data_dir = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        data_dir
            .join("flamel")
            .join("users")
            .join(&creds.user_id)
            .join("codex")
    };

    ensure_private_directory(&codex_home)?;

    let auth = json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "access_token": creds.access_token,
            "refresh_token": creds.refresh_token,
            "id_token": creds.extra.as_ref()
                .and_then(|e| e.get("id_token"))
                .and_then(|t| t.as_str())
                .unwrap_or(""),
            "account_id": creds.extra.as_ref()
                .and_then(|e| e.get("account_id"))
                .cloned()
                .unwrap_or(Value::Null),
        },
        "last_refresh": chrono::Utc::now().to_rfc3339(),
    });

    let serialized = serde_json::to_vec_pretty(&auth).map_err(|error| {
        CredentialMaterializationError::new("failed to serialize Codex credentials", None, error)
    })?;
    let auth_path = codex_home.join("auth.json");
    write_private_file(&auth_path, &serialized)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::chown;

        let _ = chown(&codex_home, Some(1000), Some(1000));
        let _ = chown(&auth_path, Some(1000), Some(1000));
    }

    tracing::info!("wrote Codex credentials to {}", auth_path.display());
    Ok(codex_home.to_string_lossy().into_owned())
}

#[cfg(test)]
mod credential_tests {
    use super::*;

    #[test]
    fn materializes_native_codex_oauth_with_private_permissions() {
        let user_home =
            std::env::temp_dir().join(format!("flamel-codex-credentials-{}", uuid::Uuid::new_v4()));
        let codex_home = user_home.join(".codex");
        let auth_path = codex_home.join("auth.json");
        std::fs::create_dir_all(&codex_home).expect("create Codex test directory");
        std::fs::write(&auth_path, b"stale").expect("create existing Codex auth file");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(&codex_home, std::fs::Permissions::from_mode(0o755))
                .expect("set broad Codex test directory mode");
            std::fs::set_permissions(&auth_path, std::fs::Permissions::from_mode(0o644))
                .expect("set broad Codex test file mode");
        }

        let credentials = ProviderCredentials {
            user_id: "test-user".to_string(),
            access_token: "synthetic-access-token".to_string(),
            refresh_token: Some("synthetic-refresh-token".to_string()),
            expires_at: 1_900_000_000,
            scopes: "openid profile".to_string(),
            extra: Some(json!({
                "id_token": "synthetic-id-token",
                "account_id": "synthetic-account",
            })),
        };

        let materialized_home = write_codex_credentials(
            &credentials,
            user_home.to_str().expect("UTF-8 Codex test path"),
        )
        .expect("materialize Codex credentials");
        assert_eq!(PathBuf::from(materialized_home), codex_home);

        let auth: Value = serde_json::from_slice(
            &std::fs::read(&auth_path).expect("read materialized Codex credentials"),
        )
        .expect("parse materialized Codex credentials");
        assert_eq!(auth["auth_mode"], json!("chatgpt"));
        assert!(auth["OPENAI_API_KEY"].is_null());
        assert!(auth["tokens"]["access_token"].is_string());
        assert!(auth["tokens"]["refresh_token"].is_string());
        assert!(auth["tokens"]["id_token"].is_string());
        assert!(auth.get("apiKey").is_none());

        let env = CodexProvider::new().build_env(
            Some(&credentials),
            None,
            user_home.to_str().expect("UTF-8 Codex test path"),
            None,
            None,
        );
        assert_eq!(
            env.iter()
                .find(|(key, _)| key == "CODEX_HOME")
                .map(|(_, value)| value.as_str()),
            codex_home.to_str()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                std::fs::metadata(&codex_home)
                    .expect("read Codex directory metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&auth_path)
                    .expect("read Codex auth metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        std::fs::remove_dir_all(&user_home).expect("remove Codex test directory");
    }
}

fn write_codex_mcp_config(mcp_servers: &Value, codex_home: &str, reasoning_effort: Option<&str>) {
    let Some(servers) = mcp_servers.as_object() else {
        return;
    };
    let reasoning_effort = sanitize_reasoning_effort(reasoning_effort).unwrap_or("high");
    let mut managed_toml = String::new();

    managed_toml.push_str(&format!(
        "model_reasoning_effort = \"{}\"\n\n",
        reasoning_effort
    ));

    for (name, config) in servers {
        let command = config.get("command").and_then(|v| v.as_str()).unwrap_or("");
        let url = config.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if command.is_empty() && url.is_empty() {
            continue;
        }

        managed_toml.push_str(&format!("[mcp_servers.{name}]\n"));
        if !url.is_empty() {
            managed_toml.push_str(&format!("url = \"{}\"\n", escape_toml_str(url)));
        } else {
            managed_toml.push_str(&format!("command = \"{}\"\n", escape_toml_str(command)));
        }

        if !command.is_empty() {
            if let Some(args) = config.get("args").and_then(|v| v.as_array()) {
                let args_str: Vec<String> = args
                    .iter()
                    .filter_map(|a| a.as_str())
                    .map(|a| format!("\"{}\"", escape_toml_str(a)))
                    .collect();
                managed_toml.push_str(&format!("args = [{}]\n", args_str.join(", ")));
            } else {
                managed_toml.push_str("args = []\n");
            }
        }

        let tool_timeout_sec = config
            .get("tool_timeout_sec")
            .and_then(|v| v.as_u64())
            .or_else(|| (name == "git").then_some(600));
        if let Some(timeout) = tool_timeout_sec {
            managed_toml.push_str(&format!("tool_timeout_sec = {timeout}\n"));
        }

        let startup_timeout_sec = config.get("startup_timeout_sec").and_then(|v| v.as_u64());
        if let Some(timeout) = startup_timeout_sec {
            managed_toml.push_str(&format!("startup_timeout_sec = {timeout}\n"));
        }

        if let Some(env) = config.get("env").and_then(|v| v.as_object()) {
            let mut inherited_keys = env.keys().cloned().collect::<Vec<_>>();
            inherited_keys.sort();
            let inherited_keys = inherited_keys
                .iter()
                .map(|key| format!("\"{}\"", escape_toml_str(key)))
                .collect::<Vec<_>>()
                .join(", ");
            managed_toml.push_str(&format!("env_vars = [{inherited_keys}]\n"));
            let secret_count = env
                .keys()
                .filter(|key| is_sensitive_mcp_env_key(key))
                .count();
            managed_toml.push_str(&format!(
                "# {} env value(s) inherited from Flamel at process startup; {} sensitive value(s) omitted from persisted config.\n",
                env.len(),
                secret_count
            ));
        }

        managed_toml.push('\n');
    }

    let config_path = std::path::Path::new(codex_home).join("config.toml");
    let managed = match managed_toml.parse::<toml_edit::DocumentMut>() {
        Ok(doc) => doc,
        Err(e) => {
            tracing::error!("failed to construct codex config: {e}");
            return;
        }
    };
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut merged = if existing.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        match existing.parse::<toml_edit::DocumentMut>() {
            Ok(doc) => doc,
            Err(e) => {
                let backup_path = config_path.with_extension("toml.flamel-invalid");
                let _ = std::fs::copy(&config_path, &backup_path);
                tracing::warn!(
                    "existing codex config was invalid ({e}); backed it up to {}",
                    backup_path.display()
                );
                toml_edit::DocumentMut::new()
            }
        }
    };

    merged["model_reasoning_effort"] = toml_edit::value(reasoning_effort);
    if merged
        .get("mcp_servers")
        .and_then(toml_edit::Item::as_table)
        .is_none()
    {
        merged["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let target_servers = merged["mcp_servers"]
        .as_table_mut()
        .expect("mcp_servers was initialized as a table");
    for name in ["flamel", "git", "docs", "db", "memory", "browser", "viz"] {
        target_servers.remove(name);
    }
    if let Some(source_servers) = managed
        .get("mcp_servers")
        .and_then(toml_edit::Item::as_table)
    {
        for (name, value) in source_servers {
            target_servers.insert(name, value.clone());
        }
    }

    if let Err(e) = std::fs::write(&config_path, merged.to_string()) {
        tracing::error!("failed to write codex config.toml: {e}");
    } else {
        tracing::info!(
            "merged flamel MCP config into {} without replacing native codex settings",
            config_path.display()
        );
    }
}

fn escape_toml_str(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_server_defers_automatic_compaction_to_flamel() {
        let provider = CodexProvider::new();
        let args = provider.build_args(&SpawnOpts {
            model: Some("gpt-5.6-sol"),
            reasoning_effort: Some("high"),
            permission_mode: "bypassPermissions",
            system_prompt_append: None,
            resume_session_id: None,
            mcp_servers: None,
            user_home: "/tmp/flamel-user",
            session_id: "session-1",
            provider_base_url: None,
            message: Some("test"),
        });

        assert!(
            args.iter()
                .any(|arg| { arg == CODEX_NATIVE_COMPACTION_LIMIT_CONFIG })
        );
        assert!(args.iter().any(|arg| arg == "web_search=\"live\""));
        assert!(!args.iter().any(|arg| arg.starts_with("model_provider=")));
    }

    #[test]
    fn brokered_app_server_uses_loopback_responses_without_websockets() {
        let provider = CodexProvider::new();
        let args = provider.build_args(&SpawnOpts {
            model: Some("gpt-5.6-sol"),
            reasoning_effort: Some("high"),
            permission_mode: "bypassPermissions",
            system_prompt_append: None,
            resume_session_id: None,
            mcp_servers: None,
            user_home: "/tmp/flamel-job",
            session_id: "job-session-1",
            provider_base_url: Some("http://127.0.0.1:43123/v1"),
            message: Some("test"),
        });
        let rendered = args.join("\n");

        assert!(rendered.contains("model_provider=\"flamel_openai\""));
        assert!(
            rendered
                .contains("model_providers.flamel_openai.base_url=\"http://127.0.0.1:43123/v1\"")
        );
        assert!(rendered.contains("model_providers.flamel_openai.env_key=\"OPENAI_API_KEY\""));
        assert!(rendered.contains("model_providers.flamel_openai.wire_api=\"responses\""));
        assert!(rendered.contains("model_providers.flamel_openai.supports_websockets=false"));
        assert!(!rendered.contains("fjp_"));
        assert!(!rendered.contains("OPENAI_API_KEY="));
    }

    #[test]
    fn native_web_search_surfaces_as_a_research_tool() {
        let provider = CodexProvider::new();
        let started = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/started","params":{"item":{"id":"search-1","type":"webSearch","status":"inProgress","query":"Flamel documentation","action":{"type":"search","query":"Flamel documentation"}}}}"#,
            )
            .expect("web search start");
        match started {
            NormalizedEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "search-1");
                assert_eq!(name, "web_search");
                assert_eq!(input["query"], json!("Flamel documentation"));
                assert_eq!(input["action"]["type"], json!("search"));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let completed = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"id":"search-1","type":"webSearch","status":"completed","query":"Flamel documentation","action":{"type":"openPage","url":"https://flamel.dev"}}}}"#,
            )
            .expect("web search completion");
        match completed {
            NormalizedEvent::ToolResult {
                tool_use_id,
                output,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "search-1");
                assert!(!is_error);
                assert!(output.contains("openPage"));
                assert!(output.contains("https://flamel.dev"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn thread_resume_request_includes_thread_id_and_overrides() {
        let bytes = build_thread_resume(
            "thread_123",
            Some("gpt-5.5"),
            "/repo",
            Some("Continue the existing conversation."),
            Some("medium"),
            "bypassPermissions",
        );
        let value: Value = serde_json::from_slice(&bytes[..bytes.len() - 1]).expect("jsonrpc");

        assert_eq!(value["method"], json!("thread/resume"));
        assert_eq!(value["params"]["threadId"], json!("thread_123"));
        assert_eq!(value["params"]["cwd"], json!("/repo"));
        assert_eq!(value["params"]["model"], json!("gpt-5.5"));
        assert_eq!(value["params"]["modelReasoningEffort"], json!("medium"));
        assert_eq!(value["params"]["approvalPolicy"], json!("never"));
        assert_eq!(
            value["params"]["developerInstructions"],
            json!("Continue the existing conversation.")
        );
    }

    #[test]
    fn thread_start_default_permission_does_not_force_bypass() {
        let bytes =
            build_thread_start(Some("gpt-5.6-sol"), "/repo", None, Some("ultra"), "default");
        let value: Value = serde_json::from_slice(&bytes[..bytes.len() - 1]).expect("jsonrpc");

        assert_eq!(value["method"], json!("thread/start"));
        assert_eq!(value["params"]["model"], json!("gpt-5.6-sol"));
        assert_eq!(value["params"]["modelReasoningEffort"], json!("ultra"));
        assert_eq!(value["params"]["approvalPolicy"], json!("on-request"));
        assert!(value["params"].get("sandbox").is_none());
    }

    #[test]
    fn thread_start_accept_edits_uses_workspace_sandbox() {
        let bytes = build_thread_start(None, "/repo", None, None, "acceptEdits");
        let value: Value = serde_json::from_slice(&bytes[..bytes.len() - 1]).expect("jsonrpc");

        assert_eq!(value["params"]["approvalPolicy"], json!("on-request"));
        assert_eq!(value["params"]["sandbox"], json!("workspace-write"));
    }

    #[test]
    fn turn_start_request_includes_text_and_images() {
        let bytes = build_turn_start(
            "thread_123",
            "Explain this screenshot",
            &[ImageBlock {
                media_type: "image/png".to_string(),
                data: "AAA".to_string(),
            }],
            Some("gpt-5.5"),
            Some("high"),
            "bypassPermissions",
        );
        let value: Value = serde_json::from_slice(&bytes[..bytes.len() - 1]).expect("jsonrpc");

        assert_eq!(value["method"], json!("turn/start"));
        assert_eq!(value["params"]["threadId"], json!("thread_123"));
        assert_eq!(value["params"]["model"], json!("gpt-5.5"));
        assert_eq!(value["params"]["effort"], json!("high"));
        assert_eq!(value["params"]["summary"], json!("detailed"));
        assert_eq!(value["params"]["approvalPolicy"], json!("never"));
        assert_eq!(
            value["params"]["input"],
            json!([
                { "type": "text", "text": "Explain this screenshot", "text_elements": [] },
                { "type": "image", "url": "data:image/png;base64,AAA" }
            ])
        );
    }

    #[test]
    fn turn_start_accepts_gpt_5_6_max_reasoning() {
        let bytes = build_turn_start(
            "thread_123",
            "Solve it",
            &[],
            Some("gpt-5.6-luna"),
            Some("max"),
            "default",
        );
        let value: Value = serde_json::from_slice(&bytes[..bytes.len() - 1]).expect("jsonrpc");

        assert_eq!(value["params"]["model"], json!("gpt-5.6-luna"));
        assert_eq!(value["params"]["effort"], json!("max"));
    }

    #[test]
    fn turn_interrupt_request_includes_thread_and_turn_ids() {
        let bytes = build_turn_interrupt("thread_123", "turn_456");
        let value: Value = serde_json::from_slice(&bytes[..bytes.len() - 1]).expect("jsonrpc");

        assert_eq!(value["method"], json!("turn/interrupt"));
        assert_eq!(value["params"]["threadId"], json!("thread_123"));
        assert_eq!(value["params"]["turnId"], json!("turn_456"));
    }

    #[test]
    fn turn_start_response_tracks_turn_id() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","id":3,"result":{"turn":{"id":"turn_456","threadId":"thread_123"}}}"#,
            )
            .expect("turn/start response should track turn ID");

        match event {
            NormalizedEvent::Raw { data } => {
                assert_eq!(
                    data.get("__codex_turn_started").and_then(|v| v.as_bool()),
                    Some(true)
                );
                assert_eq!(
                    data.get("turn_id").and_then(|v| v.as_str()),
                    Some("turn_456")
                );
                assert_eq!(
                    data.get("thread_id").and_then(|v| v.as_str()),
                    Some("thread_123")
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn turn_start_discards_blocks_replayed_during_resume() {
        let provider = CodexProvider::new();
        provider
            .serialize_for_frontend(&NormalizedEvent::AssistantText {
                text: "old assistant output".to_string(),
                complete: true,
            })
            .expect("historical snapshot");

        provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"turn/started","params":{"turn":{"id":"turn-new","threadId":"thread-1"}}}"#,
            )
            .expect("turn start");

        let snapshot = provider
            .serialize_for_frontend(&NormalizedEvent::AssistantText {
                text: "current assistant output".to_string(),
                complete: true,
            })
            .expect("current snapshot");
        assert_eq!(
            snapshot["message"]["content"],
            json!([{"type": "text", "text": "current assistant output"}])
        );
    }

    #[test]
    fn command_and_file_output_deltas_stay_in_the_running_tool() {
        let provider = CodexProvider::new();

        let command = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/commandExecution/outputDelta","params":{"itemId":"cmd-1","delta":"stdout"}}"#,
            )
            .expect("command output delta");
        match &command {
            NormalizedEvent::ToolOutputDelta {
                tool_use_id,
                output,
            } => {
                assert_eq!(tool_use_id, "cmd-1");
                assert_eq!(output, "stdout");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let file = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/fileChange/outputDelta","params":{"itemId":"file-1","delta":"patched"}}"#,
            )
            .expect("file output delta");
        match file {
            NormalizedEvent::ToolOutputDelta {
                tool_use_id,
                output,
            } => {
                assert_eq!(tool_use_id, "file-1");
                assert_eq!(output, "patched");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let serialized = provider
            .serialize_for_frontend(&command)
            .expect("serialized command delta");
        assert_eq!(serialized["type"], json!("tool_result"));
        assert_eq!(serialized["same_turn"], json!(true));
        assert_eq!(serialized["is_delta"], json!(true));
    }

    #[test]
    fn native_image_generation_surfaces_as_a_previewable_tool_result() {
        let provider = CodexProvider::new();
        let started = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/started","params":{"item":{"id":"img-1","type":"imageGeneration","status":"inProgress","revisedPrompt":"A precise product render"}}}"#,
            )
            .expect("image generation start");
        match started {
            NormalizedEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "img-1");
                assert_eq!(name, "image_gen");
                assert_eq!(input["prompt"], json!("A precise product render"));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let completed = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"id":"img-1","type":"imageGeneration","status":"completed","result":"base64-is-not-forwarded","savedPath":"/tmp/generated/product.png"}}}"#,
            )
            .expect("image generation completion");
        match completed {
            NormalizedEvent::ToolResult {
                tool_use_id,
                output,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "img-1");
                assert_eq!(
                    output,
                    "Generated image saved to /tmp/generated/product.png"
                );
                assert!(!is_error);
                assert!(!output.contains("base64-is-not-forwarded"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn raw_response_item_completed_notification_emits_text() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"rawResponseItem/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Visible app-server note"}]}}}"#,
            )
            .expect("rawResponseItem/completed should emit assistant text");

        match event {
            NormalizedEvent::AssistantText { text, complete } => {
                assert_eq!(text, "Visible app-server note");
                assert!(complete);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn raw_response_item_assistant_message_emits_text() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Visible progress note"}],"phase":"commentary"}}"#,
            )
            .expect("assistant response_item should emit text");

        match event {
            NormalizedEvent::AssistantText { text, complete } => {
                assert_eq!(text, "Visible progress note");
                assert!(complete);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn item_completed_message_emits_assistant_text() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"id":"msg-1","type":"message","role":"assistant","content":[{"type":"output_text","text":"Final answer text"}]}}}"#,
            )
            .expect("completed message item should emit text");

        match event {
            NormalizedEvent::AssistantText { text, complete } => {
                assert_eq!(text, "Final answer text");
                assert!(complete);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn response_item_user_message_is_ignored() {
        let provider = CodexProvider::new();
        let event = provider.parse_event(
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Do not echo me"}]}}"#,
        );

        assert!(event.is_none());
    }

    #[test]
    fn permission_request_emits_approval_marker() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","id":9,"method":"item/permissions/requestApproval","params":{"permissions":{"sandbox":"workspace-write"},"reason":"edit files"}}"#,
            )
            .expect("permission request");

        match event {
            NormalizedEvent::Raw { data } => {
                assert_eq!(
                    data.get("__codex_approval_request")
                        .and_then(|v| v.as_bool()),
                    Some(true)
                );
                assert_eq!(
                    data.get("method").and_then(|v| v.as_str()),
                    Some("item/permissions/requestApproval")
                );
                assert_eq!(data.get("request_id"), Some(&json!(9)));
                assert_eq!(
                    data["params"]["permissions"]["sandbox"],
                    json!("workspace-write")
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn approval_responses_match_request_protocols() {
        let command_bytes = build_approval_response(
            "item/commandExecution/requestApproval",
            &json!(11),
            &json!({}),
            false,
        )
        .expect("command response");
        let command: Value =
            serde_json::from_slice(&command_bytes[..command_bytes.len() - 1]).expect("json");
        assert_eq!(command["id"], json!(11));
        assert_eq!(command["result"]["decision"], json!("decline"));

        let patch_bytes =
            build_approval_response("applyPatchApproval", &json!(12), &json!({}), true)
                .expect("patch response");
        let patch: Value =
            serde_json::from_slice(&patch_bytes[..patch_bytes.len() - 1]).expect("json");
        assert_eq!(patch["id"], json!(12));
        assert_eq!(patch["result"]["decision"], json!("approved"));

        let permissions_bytes = build_approval_response(
            "item/permissions/requestApproval",
            &json!(13),
            &json!({"permissions":{"network":true}}),
            true,
        )
        .expect("permissions response");
        let permissions: Value =
            serde_json::from_slice(&permissions_bytes[..permissions_bytes.len() - 1])
                .expect("json");
        assert_eq!(permissions["id"], json!(13));
        assert_eq!(permissions["result"]["permissions"]["network"], json!(true));
        assert_eq!(permissions["result"]["scope"], json!("session"));
    }

    #[test]
    fn normalized_error_serializes_as_final_result() {
        let provider = CodexProvider::new();
        let value = provider
            .serialize_for_frontend(&NormalizedEvent::Error {
                message: "Model switch rejected".to_string(),
            })
            .expect("serialized error");

        assert_eq!(value["type"], json!("result"));
        assert_eq!(value["is_error"], json!(true));
        assert_eq!(value["stop_reason"], json!("error"));
        assert_eq!(value["result"], json!("Model switch rejected"));
    }

    #[test]
    fn assistant_snapshots_keep_prior_text_and_tools() {
        let provider = CodexProvider::new();

        let first = provider
            .serialize_for_frontend(&NormalizedEvent::AssistantText {
                text: "First note".to_string(),
                complete: false,
            })
            .unwrap();
        assert_eq!(
            first["message"]["content"],
            json!([
                { "type": "text", "text": "First note" }
            ])
        );

        let with_tool = provider
            .serialize_for_frontend(&NormalizedEvent::ToolUse {
                id: "tool-1".to_string(),
                name: "shell".to_string(),
                input: json!({ "command": "pwd" }),
            })
            .unwrap();
        assert_eq!(
            with_tool["message"]["content"],
            json!([
                { "type": "text", "text": "First note" },
                { "type": "tool_use", "id": "tool-1", "name": "shell", "input": { "command": "pwd" } }
            ])
        );

        let second = provider
            .serialize_for_frontend(&NormalizedEvent::AssistantText {
                text: "Second note".to_string(),
                complete: true,
            })
            .unwrap();
        assert_eq!(
            second["message"]["content"],
            json!([
                { "type": "text", "text": "First note" },
                { "type": "tool_use", "id": "tool-1", "name": "shell", "input": { "command": "pwd" } },
                { "type": "text", "text": "Second note" }
            ])
        );
    }

    #[test]
    fn complete_assistant_text_appends_new_message_instead_of_replacing_previous_one() {
        let provider = CodexProvider::new();

        provider
            .serialize_for_frontend(&NormalizedEvent::AssistantText {
                text: "First complete note".to_string(),
                complete: true,
            })
            .unwrap();
        let second = provider
            .serialize_for_frontend(&NormalizedEvent::AssistantText {
                text: "Second complete note".to_string(),
                complete: true,
            })
            .unwrap();

        assert_eq!(
            second["message"]["content"],
            json!([
                { "type": "text", "text": "First complete note" },
                { "type": "text", "text": "Second complete note" }
            ])
        );
    }

    #[test]
    fn rich_tool_images_are_not_copied_into_frontend_history() {
        let encoded = "sensitive-base64-payload";
        let output = normalize_tool_result(&json!({
            "content": [
                {"type": "text", "text": "capture saved at /private/capture.webp"},
                {"type": "image", "mimeType": "image/webp", "data": encoded}
            ]
        }))
        .0;
        assert!(output.contains("capture saved"));
        assert!(output.contains("delivered_to_model"));
        assert!(!output.contains(encoded));
    }

    #[test]
    fn frontend_tool_results_serialize_optional_presentation_compatibly() {
        let provider = CodexProvider::new();
        let legacy = provider
            .serialize_for_frontend(&NormalizedEvent::ToolResult {
                tool_use_id: "legacy-1".to_string(),
                output: "legacy output".to_string(),
                presentation: None,
                is_error: false,
            })
            .expect("legacy result");
        assert_eq!(legacy["content"], json!("legacy output"));
        assert!(legacy.get("presentation").is_none());

        let (output, presentation) = normalize_tool_result(&json!({
            "content": [{"type": "text", "text": "structured output"}],
            "structuredContent": {"rows": 2}
        }));
        let structured = provider
            .serialize_for_frontend(&NormalizedEvent::ToolResult {
                tool_use_id: "structured-1".to_string(),
                output,
                presentation: Some(presentation),
                is_error: false,
            })
            .expect("structured result");
        assert_eq!(structured["content"], json!("structured output"));
        assert_eq!(structured["presentation"]["version"], json!(1));
        assert_eq!(
            structured["presentation"]["structuredContent"]["rows"],
            json!(2)
        );
    }

    #[test]
    fn codex_tool_use_snapshot_updates_existing_tool_by_id() {
        let provider = CodexProvider::new();

        provider
            .serialize_for_frontend(&NormalizedEvent::McpToolCall {
                server: "flamel".to_string(),
                tool: "plan".to_string(),
                arguments: Value::Null,
                result: None,
                error: None,
                status: "in_progress".to_string(),
                id: Some("plan-1".to_string()),
            })
            .unwrap();

        let updated = provider
            .serialize_for_frontend(&NormalizedEvent::McpToolCall {
                server: "flamel".to_string(),
                tool: "plan".to_string(),
                arguments: json!({ "action": "create", "title": "Plan", "items": [] }),
                result: None,
                error: None,
                status: "in_progress".to_string(),
                id: Some("plan-1".to_string()),
            })
            .unwrap();

        assert_eq!(
            updated["message"]["content"],
            json!([
                { "type": "tool_use", "id": "plan-1", "name": "mcp__flamel__plan", "input": { "action": "create", "title": "Plan", "items": [] } }
            ])
        );
    }

    #[test]
    fn mcp_tool_call_parses_input_alias_as_arguments() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/started","params":{"item":{"id":"plan-1","type":"mcp_tool_call","server":"flamel","tool":"plan","input":{"action":"create"}}}}"#,
            )
            .expect("mcp tool call should parse input alias");

        match event {
            NormalizedEvent::McpToolCall { arguments, .. } => {
                assert_eq!(arguments["action"], json!("create"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn codex_mcp_config_extends_git_tool_timeout() {
        let temp_dir =
            std::env::temp_dir().join(format!("flamel_codex_config_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("temp dir");

        write_codex_mcp_config(
            &json!({
                "flamel": {
                    "command": "/tmp/flamel_mcp",
                    "args": [],
                    "tool_timeout_sec": 900,
                    "startup_timeout_sec": 30
                },
                "git": {"command": "/tmp/git_mcp", "args": []},
                "docs": {"command": "/tmp/docs_mcp", "args": []},
                "grafana": {"type": "http", "url": "https://mcp.grafana.com/mcp"}
            }),
            temp_dir.to_str().expect("utf8 temp dir"),
            Some("medium"),
        );

        let config = std::fs::read_to_string(temp_dir.join("config.toml")).expect("config");
        assert!(config.contains("model_reasoning_effort = \"medium\""));
        assert!(config.contains("[mcp_servers.flamel]\n"));
        assert!(config.contains("tool_timeout_sec = 900"));
        assert!(config.contains("startup_timeout_sec = 30"));
        assert!(config.contains("[mcp_servers.git]\n"));
        assert!(config.contains("tool_timeout_sec = 600"));
        assert!(config.contains("[mcp_servers.docs]\n"));
        assert!(config.contains("[mcp_servers.grafana]\n"));
        assert!(config.contains("url = \"https://mcp.grafana.com/mcp\""));
        assert_eq!(config.matches("tool_timeout_sec = 600").count(), 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn codex_mcp_config_preserves_native_settings_and_replaces_managed_servers() {
        let temp_dir = std::env::temp_dir().join(format!(
            "flamel_codex_merge_config_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("temp dir");
        std::fs::write(
            temp_dir.join("config.toml"),
            r#"model = "gpt-5.6-sol"

[[skills.config]]
path = "/tmp/example/SKILL.md"
enabled = false

[plugins.example]
enabled = true

[mcp_servers.custom]
command = "/tmp/custom_mcp"

[mcp_servers.flamel]
command = "/tmp/stale_flamel_mcp"
[mcp_servers.flamel.env]
FLAMEL_TOKEN = "must-be-removed"
"#,
        )
        .expect("seed config");

        write_codex_mcp_config(
            &json!({
                "flamel": {
                    "command": "/tmp/current_flamel_mcp",
                    "args": [],
                    "env": {"FLAMEL_TOKEN": "new-secret"}
                },
                "git": {"command": "/tmp/git_mcp", "args": []}
            }),
            temp_dir.to_str().expect("utf8 temp dir"),
            Some("max"),
        );

        let config = std::fs::read_to_string(temp_dir.join("config.toml")).expect("config");
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(config.contains("[[skills.config]]"));
        assert!(config.contains("[plugins.example]"));
        assert!(config.contains("[mcp_servers.custom]"));
        assert!(config.contains("command = \"/tmp/current_flamel_mcp\""));
        assert!(config.contains("model_reasoning_effort = \"max\""));
        assert!(!config.contains("stale_flamel_mcp"));
        assert!(!config.contains("must-be-removed"));
        assert!(!config.contains("new-secret"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn codex_mcp_config_omits_persisted_env_values() {
        let temp_dir = std::env::temp_dir().join(format!(
            "flamel_codex_secret_config_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("temp dir");

        write_codex_mcp_config(
            &json!({
                "flamel": {
                    "command": "/tmp/flamel_mcp",
                    "args": [],
                    "env": {
                        "FLAMEL_TOKEN": "secret-token",
                        "FLAMEL_SYSTEM_PROMPT": "sensitive prompt",
                        "FLAMEL_URL": "http://localhost:8181"
                    }
                },
                "git": {
                    "command": "/tmp/git_mcp",
                    "args": [],
                    "env": {"GIT_WORK_DIR": "/repo"}
                }
            }),
            temp_dir.to_str().expect("utf8 temp dir"),
            None,
        );

        let config = std::fs::read_to_string(temp_dir.join("config.toml")).expect("config");
        assert!(config.contains("env value(s) inherited from Flamel"));
        assert!(
            config.contains(
                "env_vars = [\"FLAMEL_SYSTEM_PROMPT\", \"FLAMEL_TOKEN\", \"FLAMEL_URL\"]"
            )
        );
        assert!(config.contains("env_vars = [\"GIT_WORK_DIR\"]"));
        assert!(!config.contains("secret-token"));
        assert!(!config.contains("sensitive prompt"));
        assert!(!config.contains("/repo"));
        assert!(!config.contains("[mcp_servers.flamel.env]"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn thread_closed_emits_internal_reset_marker() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(r#"{"jsonrpc":"2.0","method":"thread/closed","params":{"thread":{"id":"thread_123"}}}"#)
            .expect("thread/closed should produce a reset marker");

        match event {
            NormalizedEvent::Raw { data } => {
                assert_eq!(
                    data.get("__codex_thread_closed").and_then(|v| v.as_bool()),
                    Some(true)
                );
                assert_eq!(
                    data.get("thread_id").and_then(|v| v.as_str()),
                    Some("thread_123")
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn jsonrpc_error_response_extracts_nested_message() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"data":{"details":{"message":"Model gpt-5.5 is unavailable for this account"}}}}"#,
            )
            .expect("json-rpc error should surface a message");

        match event {
            NormalizedEvent::Error { message } => {
                assert_eq!(message, "Model gpt-5.5 is unavailable for this account");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn missing_turn_id_jsonrpc_error_is_nonfatal() {
        let provider = CodexProvider::new();
        let event = provider.parse_event(
            r#"{"jsonrpc":"2.0","id":42,"error":{"code":-32600,"message":"Invalid request: missing field `turnId`"}}"#,
        );

        assert!(event.is_none());
    }

    #[test]
    fn resume_rejection_jsonrpc_error_emits_internal_reset_marker() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","id":43,"error":{"code":-32600,"message":"no rollout found for thread id 11111111-1111-1111-1111-111111111111"}}"#,
            )
            .expect("resume rejection should produce an internal reset marker");

        match event {
            NormalizedEvent::Raw { data } => {
                assert_eq!(
                    data.get("__codex_resume_rejected")
                        .and_then(|v| v.as_bool()),
                    Some(true)
                );
                assert_eq!(
                    data.get("error").and_then(|v| v.as_str()),
                    Some("no rollout found for thread id 11111111-1111-1111-1111-111111111111")
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn completed_context_compaction_item_emits_terminal_status() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"id":"compact-1","type":"contextCompaction"}}}"#,
            )
            .expect("contextCompaction should produce a status event");

        assert!(matches!(
            event,
            NormalizedEvent::Raw { ref data }
                if data["type"] == "system_status"
                    && data["status"] == "compact_complete"
        ));
    }

    #[test]
    fn started_context_compaction_item_emits_live_status() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/started","params":{"item":{"id":"compact-1","type":"contextCompaction"}}}"#,
            )
            .expect("contextCompaction should produce a live status event");

        assert!(matches!(event, NormalizedEvent::Compacting));
    }

    #[test]
    fn token_usage_notification_preserves_context_fill_and_window() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"thread/tokenUsage/updated","params":{"threadId":"thread-1","turnId":"turn-1","tokenUsage":{"total":{"totalTokens":224000,"inputTokens":210000,"cachedInputTokens":180000,"outputTokens":14000,"reasoningOutputTokens":9000},"last":{"totalTokens":1200,"inputTokens":1000,"cachedInputTokens":800,"outputTokens":200,"reasoningOutputTokens":100},"modelContextWindow":272000}}}"#,
            )
            .expect("usage notification should be forwarded");

        match event {
            NormalizedEvent::Usage {
                usage,
                context_window,
            } => {
                assert_eq!(usage.input_tokens, 1_200);
                assert_eq!(usage.output_tokens, 200);
                assert_eq!(usage.cached_input_tokens, Some(800));
                assert_eq!(context_window, Some(272_000));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn reasoning_activity_surfaces_redacted_thinking_block() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/started","params":{"item":{"id":"reason-1","type":"reasoning"}}}"#,
            )
            .expect("reasoning start should produce a thinking event");

        assert!(matches!(event, NormalizedEvent::ThinkingRedacted));

        let value = provider
            .serialize_for_frontend(&event)
            .expect("serialized thinking marker");
        assert_eq!(
            value["message"]["content"],
            json!([{ "type": "thinking", "thinking": "", "redacted": true }])
        );
    }

    #[test]
    fn reasoning_summary_delta_surfaces_displayable_thinking() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/reasoning/summaryTextDelta","params":{"itemId":"reason-1","summaryIndex":0,"delta":"I am checking the request.","threadId":"thread-1","turnId":"turn-1"}}"#,
            )
            .expect("reasoning summary should produce a thinking event");

        assert!(matches!(
            event,
            NormalizedEvent::Thinking { ref text }
                if text == "I am checking the request."
        ));
        let value = provider.serialize_for_frontend(&event).unwrap();
        assert_eq!(
            value["message"]["content"],
            json!([{ "type": "thinking", "thinking": "I am checking the request." }])
        );
    }

    #[test]
    fn adjacent_redacted_reasoning_markers_are_deduped() {
        let provider = CodexProvider::new();
        provider
            .serialize_for_frontend(&NormalizedEvent::ThinkingRedacted)
            .unwrap();
        let value = provider
            .serialize_for_frontend(&NormalizedEvent::ThinkingRedacted)
            .unwrap();

        assert_eq!(value["message"]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn reasoning_summary_replaces_redacted_marker_and_appends_deltas() {
        let provider = CodexProvider::new();
        provider
            .serialize_for_frontend(&NormalizedEvent::ThinkingRedacted)
            .unwrap();
        provider
            .serialize_for_frontend(&NormalizedEvent::Thinking {
                text: "Checking ".to_string(),
            })
            .unwrap();
        let value = provider
            .serialize_for_frontend(&NormalizedEvent::Thinking {
                text: "the code.".to_string(),
            })
            .unwrap();

        assert_eq!(
            value["message"]["content"],
            json!([{ "type": "thinking", "thinking": "Checking the code." }])
        );
    }

    #[test]
    fn private_reasoning_deltas_do_not_split_cumulative_summary_snapshots() {
        let provider = CodexProvider::new();
        let start = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/started","params":{"item":{"id":"reason-1","type":"reasoning"}}}"#,
            )
            .unwrap();
        provider.serialize_for_frontend(&start).unwrap();

        let first = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/reasoning/summaryTextDelta","params":{"itemId":"reason-1","delta":"**Considering security and plans**\n\nI"}}"#,
            )
            .unwrap();
        provider.serialize_for_frontend(&first).unwrap();

        assert!(provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/reasoning/textDelta","params":{"itemId":"reason-1","delta":"private"}}"#,
            )
            .is_none());

        let cumulative = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/reasoning/summaryTextDelta","params":{"itemId":"reason-1","delta":"**Considering security and plans**\n\nI need to provide advice"}}"#,
            )
            .unwrap();
        let value = provider.serialize_for_frontend(&cumulative).unwrap();

        assert_eq!(
            value["message"]["content"],
            json!([{
                "type": "thinking",
                "thinking": "**Considering security and plans**\n\nI need to provide advice"
            }])
        );
    }

    #[test]
    fn repeated_reasoning_snapshot_after_part_boundary_is_not_duplicated() {
        let provider = CodexProvider::new();
        let repeated = "Preparing to push current changes for testing";

        provider
            .serialize_for_frontend(&NormalizedEvent::Thinking {
                text: repeated.to_string(),
            })
            .unwrap();
        provider
            .serialize_for_frontend(&NormalizedEvent::Thinking {
                text: "\n\n".to_string(),
            })
            .unwrap();
        let value = provider
            .serialize_for_frontend(&NormalizedEvent::Thinking {
                text: repeated.to_string(),
            })
            .unwrap();

        assert_eq!(
            value["message"]["content"],
            json!([{ "type": "thinking", "thinking": repeated }])
        );
    }

    #[test]
    fn distinct_reasoning_parts_remain_separated() {
        assert_eq!(
            merge_streamed_reasoning("Inspecting changes\n\n", "Running tests"),
            "Inspecting changes\n\nRunning tests"
        );
    }

    #[test]
    fn reasoning_phases_remain_in_tool_order() {
        let provider = CodexProvider::new();
        provider
            .serialize_for_frontend(&NormalizedEvent::ThinkingRedacted)
            .unwrap();
        provider
            .serialize_for_frontend(&NormalizedEvent::ToolUse {
                id: "tool-1".to_string(),
                name: "shell".to_string(),
                input: json!({ "command": "pwd" }),
            })
            .unwrap();
        let value = provider
            .serialize_for_frontend(&NormalizedEvent::ThinkingRedacted)
            .unwrap();

        let content = value["message"]["content"].as_array().unwrap();
        let redacted_count = content
            .iter()
            .filter(|block| {
                block.get("type").and_then(|value| value.as_str()) == Some("thinking")
                    && block
                        .get("redacted")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
            })
            .count();
        assert_eq!(redacted_count, 2);
    }

    #[test]
    fn turn_failed_extracts_nested_error_message() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"turn/failed","params":{"error":{"details":{"message":"Rate limited by provider"}}}}"#,
            )
            .expect("turn/failed should produce a result");

        match event {
            NormalizedEvent::Result {
                is_error,
                error_message,
                ..
            } => {
                assert!(is_error);
                assert_eq!(error_message.as_deref(), Some("Rate limited by provider"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn native_plan_update_is_rejected_as_a_recovery_event() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"turn/plan/updated","params":{"threadId":"thread-1","turnId":"turn-1","explanation":"Build plan","plan":[{"step":"Alpha","status":"pending"},{"step":"Beta","status":"pending"}]}}"#,
            )
            .expect("native plan update should produce a guard event");

        match event {
            NormalizedEvent::Raw { data } => {
                assert_eq!(data["type"], json!("workflow_event"));
                assert_eq!(data["event"]["kind"], json!("recovery.notice"));
                assert_eq!(data["event"]["source"], json!("codex_native_plan_guard"));
                assert_eq!(data["event"]["payload"]["nativePlanRejected"], json!(true));
                assert_eq!(data["event"]["payload"]["itemCount"], json!(2));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn item_error_extracts_nested_message() {
        let provider = CodexProvider::new();
        let event = provider
            .parse_event(
                r#"{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"error","error":{"message":"Model switch rejected"}}}}"#,
            )
            .expect("item error should surface a message");

        match event {
            NormalizedEvent::Error { message } => {
                assert_eq!(message, "Model switch rejected");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
