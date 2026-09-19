#![recursion_limit = "512"]
//! Flamel's supervised stdio MCP gateway for planning, coding, and connected capabilities.

use flamel::mcp_args::normalize_tool_arguments;
use flamel::mcp_tool_catalog::apply_tool_capability_metadata;
use serde_json::{Value, json};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, Write};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};

/// Interactive shell session — holds a running child process with piped I/O.
struct ShellSession {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    output_buf: Arc<Mutex<String>>,
    started_at: std::time::Instant,
    last_read: std::time::Instant,
    cwd: String,
    command: String,
    command_event: ShellCommandEventContext,
    event_closed: bool,
}

fn shell_sessions() -> &'static Mutex<HashMap<String, ShellSession>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, ShellSession>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parse_automation_capability_grants(
    encoded: &str,
) -> Result<Option<HashMap<String, HashSet<String>>>, String> {
    if encoded.len() > 64 * 1024 {
        return Err("automation capability grant file is too large".into());
    }
    let payload: Value = serde_json::from_str(encoded)
        .map_err(|error| format!("automation capability grants are invalid: {error}"))?;
    match payload.get("mode").and_then(Value::as_str) {
        Some("unrestricted") => return Ok(None),
        Some("restricted") => {}
        _ => return Err("automation capability grant mode is invalid".into()),
    }
    let grants = payload
        .get("grants")
        .and_then(Value::as_array)
        .ok_or_else(|| "automation capability grants are missing".to_owned())?;
    if grants.len() > 16 {
        return Err("automation capability grant count exceeds the runtime limit".into());
    }
    let mut result = HashMap::new();
    for grant in grants {
        let object = grant
            .as_object()
            .ok_or_else(|| "automation capability grant must be an object".to_owned())?;
        if object
            .keys()
            .any(|field| !matches!(field.as_str(), "capability_id" | "tools"))
        {
            return Err("automation capability grant contains unsupported fields".into());
        }
        let capability_id = object
            .get("capability_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 160)
            .ok_or_else(|| "automation capability_id is invalid".to_owned())?;
        let tools = object
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| "automation capability tools are missing".to_owned())?;
        if tools.is_empty() || tools.len() > 32 || result.contains_key(capability_id) {
            return Err("automation capability tools are invalid".into());
        }
        let mut exact_tools = HashSet::new();
        for tool in tools {
            let tool = tool
                .as_str()
                .filter(|value| !value.is_empty() && *value != "*" && value.len() <= 160)
                .ok_or_else(|| "automation capability tool is invalid".to_owned())?;
            if !exact_tools.insert(tool.to_owned()) {
                return Err("automation capability tools contain a duplicate".into());
            }
        }
        result.insert(capability_id.to_owned(), exact_tools);
    }
    Ok(Some(result))
}

fn automation_capability_grants() -> Result<Option<HashMap<String, HashSet<String>>>, String> {
    let Ok(path) = std::env::var("FLAMEL_AUTOMATION_GRANT_FILE") else {
        return Ok(None);
    };
    let encoded = std::fs::read_to_string(&path)
        .map_err(|error| format!("automation capability grants are unavailable: {error}"))?;
    parse_automation_capability_grants(&encoded)
}

#[derive(Debug, Clone)]
struct ShellCommandEventContext {
    session_id: String,
    run_id: String,
    block_id: String,
    command: String,
    cwd: String,
    started_at: String,
}

fn mcp_command_sequence() -> &'static Mutex<u64> {
    static SEQUENCE: OnceLock<Mutex<u64>> = OnceLock::new();
    SEQUENCE.get_or_init(|| Mutex::new(1))
}

fn next_mcp_command_event_metadata() -> (u64, String) {
    let sequence = {
        let mut sequence = mcp_command_sequence().lock().unwrap();
        let current = *sequence;
        *sequence = sequence.saturating_add(1);
        current
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string());
    (sequence, timestamp)
}

fn send_mcp_command_event(event: Value) {
    let msg = json!({
        "jsonrpc": "2.0",
        "method": "notifications/message",
        "params": {
            "level": "info",
            "logger": "command_event",
            "data": event,
        }
    });
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = serde_json::to_writer(&mut out, &msg);
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

fn shell_command_origin() -> Value {
    json!({
        "kind": "mcp_shell",
        "label": "MCP shell",
    })
}

fn start_shell_command_event(
    session_id: Option<String>,
    command: &str,
    cwd: &str,
) -> ShellCommandEventContext {
    let (sequence, timestamp) = next_mcp_command_event_metadata();
    let session_id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let run_id = uuid::Uuid::new_v4().to_string();
    let block_id = uuid::Uuid::new_v4().to_string();
    let context = ShellCommandEventContext {
        session_id: session_id.clone(),
        run_id: run_id.clone(),
        block_id: block_id.clone(),
        command: command.to_string(),
        cwd: cwd.to_string(),
        started_at: timestamp.clone(),
    };
    let origin = shell_command_origin();

    send_mcp_command_event(json!({
        "event_id": uuid::Uuid::new_v4().to_string(),
        "kind": "session_started",
        "session_id": session_id,
        "sequence": sequence,
        "timestamp": timestamp,
        "session": {
            "id": context.session_id,
            "origin": origin,
            "status": "running",
            "created_at": context.started_at,
            "cwd": cwd,
            "metadata": {},
        }
    }));

    let (sequence, timestamp) = next_mcp_command_event_metadata();
    let origin = shell_command_origin();
    send_mcp_command_event(json!({
        "event_id": uuid::Uuid::new_v4().to_string(),
        "kind": "command_started",
        "session_id": context.session_id,
        "run_id": context.run_id,
        "block_id": context.block_id,
        "sequence": sequence,
        "timestamp": timestamp,
        "run": {
            "id": context.run_id,
            "session_id": context.session_id,
            "origin": origin,
            "status": "running",
            "started_at": context.started_at,
            "command": command,
            "cwd": cwd,
            "metadata": {},
        },
        "block": {
            "id": context.block_id,
            "session_id": context.session_id,
            "run_id": context.run_id,
            "origin": shell_command_origin(),
            "status": "running",
            "started_at": context.started_at,
            "title": command,
            "command": command,
            "metadata": {},
        }
    }));

    context
}

fn shell_line_stream_and_text(line: &str) -> (&'static str, &str) {
    if let Some(stderr) = line.strip_prefix("[stderr] ") {
        ("stderr", stderr)
    } else {
        ("stdout", line)
    }
}

fn emit_shell_output_line(context: &ShellCommandEventContext, line: &str) {
    let (stream, text) = shell_line_stream_and_text(line);
    emit_shell_output_event(context, stream, text);
}

fn emit_shell_output_event(context: &ShellCommandEventContext, stream: &str, text: &str) {
    if text.is_empty() {
        return;
    }
    let (sequence, timestamp) = next_mcp_command_event_metadata();
    let text = cap_chars(text, 16_000, "...[truncated]");
    send_mcp_command_event(json!({
        "event_id": uuid::Uuid::new_v4().to_string(),
        "kind": "output_chunk",
        "session_id": context.session_id,
        "run_id": context.run_id,
        "block_id": context.block_id,
        "sequence": sequence,
        "timestamp": timestamp,
        "chunk": {
            "id": uuid::Uuid::new_v4().to_string(),
            "session_id": context.session_id,
            "run_id": context.run_id,
            "block_id": context.block_id,
            "sequence": sequence,
            "stream": stream,
            "text": text,
            "received_at": timestamp,
            "byte_len": text.len(),
            "metadata": {},
        }
    }));
}

fn emit_shell_error_event(context: &ShellCommandEventContext, error: &str) {
    let (sequence, timestamp) = next_mcp_command_event_metadata();
    send_mcp_command_event(json!({
        "event_id": uuid::Uuid::new_v4().to_string(),
        "kind": "error",
        "session_id": context.session_id,
        "run_id": context.run_id,
        "block_id": context.block_id,
        "sequence": sequence,
        "timestamp": timestamp,
        "error": error,
    }));
}

fn finish_shell_command_event(
    context: &ShellCommandEventContext,
    status: &str,
    exit_code: Option<i32>,
) {
    let (sequence, timestamp) = next_mcp_command_event_metadata();
    let origin = shell_command_origin();
    send_mcp_command_event(json!({
        "event_id": uuid::Uuid::new_v4().to_string(),
        "kind": "command_finished",
        "session_id": context.session_id,
        "run_id": context.run_id,
        "block_id": context.block_id,
        "sequence": sequence,
        "timestamp": timestamp,
        "run": {
            "id": context.run_id,
            "session_id": context.session_id,
            "origin": origin,
            "status": status,
            "started_at": context.started_at,
            "finished_at": timestamp,
            "command": context.command,
            "cwd": context.cwd,
            "exit_code": exit_code,
            "metadata": {},
        },
        "block": {
            "id": context.block_id,
            "session_id": context.session_id,
            "run_id": context.run_id,
            "origin": shell_command_origin(),
            "status": status,
            "started_at": context.started_at,
            "finished_at": timestamp,
            "title": context.command,
            "command": context.command,
            "exit_code": exit_code,
            "metadata": {},
        }
    }));
}

fn close_shell_session_event(context: &ShellCommandEventContext, status: &str) {
    let (sequence, timestamp) = next_mcp_command_event_metadata();
    send_mcp_command_event(json!({
        "event_id": uuid::Uuid::new_v4().to_string(),
        "kind": "session_closed",
        "session_id": context.session_id,
        "sequence": sequence,
        "timestamp": timestamp,
        "session": {
            "id": context.session_id,
            "origin": shell_command_origin(),
            "status": status,
            "created_at": context.started_at,
            "closed_at": timestamp,
            "cwd": context.cwd,
            "metadata": {},
        }
    }));
}

fn finish_and_close_shell_command_event(
    context: &ShellCommandEventContext,
    status: &str,
    exit_code: Option<i32>,
) {
    finish_shell_command_event(context, status, exit_code);
    close_shell_session_event(context, "closed");
}

fn shell_status_from_exit_code(exit_code: i32) -> &'static str {
    if exit_code == 0 {
        "succeeded"
    } else {
        "failed"
    }
}

fn enabled_env_flag(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn unrestricted_shell_enabled() -> bool {
    enabled_env_flag(std::env::var("FLAMEL_SHELL_UNRESTRICTED").ok().as_deref())
}

struct AgentBatchRegistry {
    order: VecDeque<String>,
    batches: HashMap<String, AgentBatchTracked>,
}

struct AgentBatchTracked {
    batch_id: String,
    description: String,
    total: usize,
    idle_timeout_secs: u64,
    started_at: std::time::Instant,
    last_update: std::time::Instant,
    status: String,
    agents: Vec<AgentBatchTrackedAgent>,
}

struct AgentBatchTrackedAgent {
    index: usize,
    description: String,
    subagent_type: String,
    status: String,
    run_id: String,
    duration_ms: Option<u64>,
    output: Option<String>,
    output_truncated: bool,
    error: Option<String>,
}

fn agent_batch_registry() -> &'static Mutex<AgentBatchRegistry> {
    static REGISTRY: OnceLock<Mutex<AgentBatchRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(AgentBatchRegistry {
            order: VecDeque::new(),
            batches: HashMap::new(),
        })
    })
}

fn register_agent_batch(
    batch_id: &str,
    description: String,
    idle_timeout_secs: u64,
    agents: Vec<AgentBatchTrackedAgent>,
) {
    if let Ok(mut registry) = agent_batch_registry().lock() {
        registry.order.push_back(batch_id.to_string());
        registry.batches.insert(
            batch_id.to_string(),
            AgentBatchTracked {
                batch_id: batch_id.to_string(),
                description,
                total: agents.len(),
                idle_timeout_secs,
                started_at: std::time::Instant::now(),
                last_update: std::time::Instant::now(),
                status: "running".to_string(),
                agents,
            },
        );

        while registry.order.len() > 20 {
            let Some(index) = registry.order.iter().position(|id| {
                registry
                    .batches
                    .get(id)
                    .is_some_and(|batch| batch.status != "running")
            }) else {
                break;
            };
            if let Some(oldest) = registry.order.remove(index) {
                registry.batches.remove(&oldest);
            }
        }
    }
}

fn update_agent_batch_agent(batch_id: &str, idx: usize, result: &AgentBatchResult) {
    if let Ok(mut registry) = agent_batch_registry().lock() {
        if let Some(batch) = registry.batches.get_mut(batch_id) {
            if let Some(agent) = batch.agents.get_mut(idx) {
                agent.status = result.status.clone();
                agent.run_id = result.run_id.clone();
                agent.duration_ms = Some(result.duration_ms);
                let output_limit = 15 * 1024;
                let output_truncated = result.output.len() > output_limit;
                let capped_output = cap_output(
                    &result.output,
                    output_limit,
                    "\n\n[... agent output truncated at 15KB in AgentBatchStatus ...]",
                );
                agent.output_truncated = output_truncated;
                agent.output = (!capped_output.is_empty()).then_some(capped_output);
                agent.error = result.error.clone();
            }
            batch.last_update = std::time::Instant::now();
        }
    }
}

fn finish_agent_batch(batch_id: &str, status: &str) {
    if let Ok(mut registry) = agent_batch_registry().lock() {
        if let Some(batch) = registry.batches.get_mut(batch_id) {
            batch.status = status.to_string();
            batch.last_update = std::time::Instant::now();
        }
    }
}

fn agent_batch_status_value(batch: &AgentBatchTracked) -> Value {
    let completed = batch
        .agents
        .iter()
        .filter(|agent| agent.status != "running")
        .count();
    let failed = batch
        .agents
        .iter()
        .filter(|agent| agent.status == "failed")
        .count();
    let timed_out = batch
        .agents
        .iter()
        .filter(|agent| agent.status == "timeout")
        .count();
    let skipped = batch
        .agents
        .iter()
        .filter(|agent| agent.status == "skipped")
        .count();

    json!({
        "batch_id": batch.batch_id.clone(),
        "description": batch.description.clone(),
        "status": batch.status.clone(),
        "completed": completed,
        "total": batch.total,
        "running": batch.total.saturating_sub(completed),
        "failed": failed,
        "timed_out": timed_out,
        "skipped": skipped,
        "elapsed_ms": batch.started_at.elapsed().as_millis() as u64,
        "last_update_ms_ago": batch.last_update.elapsed().as_millis() as u64,
        "idle_timeout_secs": batch.idle_timeout_secs,
        "agents": batch.agents.iter().map(|agent| json!({
            "index": agent.index,
            "description": agent.description.clone(),
            "subagent_type": agent.subagent_type.clone(),
            "status": agent.status.clone(),
            "run_id": agent.run_id.clone(),
            "duration_ms": agent.duration_ms,
            "output": agent.output.clone(),
            "output_truncated": agent.output_truncated,
            "error": agent.error.clone(),
        })).collect::<Vec<_>>(),
    })
}

fn run_agent_batch_status(args: &Value) -> Result<String, String> {
    if let Some(status) = mcp_agents::durable_batch_status(args)? {
        return Ok(status);
    }
    let requested_id = args
        .get("batch_id")
        .and_then(|v| v.as_str())
        .map(|id| id.to_string());
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(3_600_000);
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let mut observed_last_update: Option<std::time::Instant> = None;

    loop {
        let (status, running, last_update) = {
            let registry = agent_batch_registry()
                .lock()
                .map_err(|_| "agent batch status lock error".to_string())?;

            let batch_id = if let Some(id) = requested_id.as_deref() {
                Some(id.to_string())
            } else {
                registry
                    .order
                    .iter()
                    .rev()
                    .find(|id| {
                        registry
                            .batches
                            .get(*id)
                            .map(|batch| batch.status == "running")
                            .unwrap_or(false)
                    })
                    .or_else(|| registry.order.back())
                    .cloned()
            };

            let Some(batch_id) = batch_id else {
                return Ok(
                    json!({"status":"not_found","total":0,"completed":0,"running":0,"agents":[],
                    "message":"No recorded agent batch was found. Completion is unconfirmed."})
                    .to_string(),
                );
            };
            let Some(batch) = registry.batches.get(&batch_id) else {
                return Err(format!("AgentBatch run not found: {batch_id}"));
            };

            let running = batch
                .agents
                .iter()
                .filter(|agent| agent.status == "running")
                .count();
            let status = serde_json::to_string_pretty(&agent_batch_status_value(batch))
                .map_err(|e| format!("failed to format AgentBatch status: {e}"))?;
            (status, running, batch.last_update)
        };

        if running == 0 || timeout_ms == 0 {
            return Ok(status);
        }

        if let Some(previous) = observed_last_update {
            if last_update != previous {
                return Ok(status);
            }
        } else {
            observed_last_update = Some(last_update);
        }

        let now = std::time::Instant::now();
        if now >= deadline {
            return Ok(status);
        }

        std::thread::sleep(std::cmp::min(
            std::time::Duration::from_millis(1000),
            deadline.saturating_duration_since(now),
        ));
    }
}

/// Track last-read mtime (ms since epoch) per absolute file path for this session.
/// Used to detect stale edits: if a file is edited without being read first, or
/// if the file was modified externally since the last read, we reject the edit.
fn file_read_times() -> &'static Mutex<HashMap<String, u64>> {
    static READ_TIMES: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    READ_TIMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_mtime_ms(path: &str) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

/// Record that `path` was just read. Called from flamel_read after a successful read.
fn record_file_read(path: &str) {
    if let Some(mtime) = get_mtime_ms(path) {
        if let Ok(mut map) = file_read_times().lock() {
            map.insert(path.to_string(), mtime);
        }
    }
}

/// Update the read record after a successful write (so the next edit doesn't
/// trigger a false "file modified externally" error).
fn record_file_written(path: &str) {
    record_file_read(path);
}

/// Remove any cached read-tracking after a successful delete.
fn record_file_deleted(path: &str) {
    if let Ok(mut map) = file_read_times().lock() {
        map.remove(path);
    }
}

/// Check whether `path` has been read this session and hasn't changed since.
/// Returns Ok(()) if safe to edit, Err(message) if not.
fn check_read_before_edit(path: &str) -> Result<(), String> {
    // Only check files that exist (new files don't need prior reads)
    if !std::path::Path::new(path).exists() {
        return Ok(());
    }
    let map = file_read_times()
        .lock()
        .map_err(|_| "lock error".to_string())?;
    match map.get(path) {
        None => Err(format!(
            "File '{path}' has not been read this session. \
             Read it first with file_read(action=\"read\", file=\"{path}\") or mcp__flamel__file_read, then retry the edit."
        )),
        Some(&read_mtime) => match get_mtime_ms(path) {
            Some(current_mtime) if current_mtime != read_mtime => Err(format!(
                "File '{path}' was modified externally since last read (timestamps differ). \
                     Re-read it with file_read(action=\"read\", file=\"{path}\") or mcp__flamel__file_read to get the current content, then retry."
            )),
            _ => Ok(()),
        },
    }
}

// ── Auto-memory: session tool call tracking ────────────────────────────────────

// ── File history ──────────────────────────────────────────────────────────────

/// Save a backup of `path` content to `/tmp/flamel-history/{session}/{sanitized}/{timestamp}.bak`
/// before any destructive edit. Silently ignores errors — backup is best-effort.
fn backup_file(path: &str, session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    let Ok(content) = std::fs::read(path) else {
        return;
    };
    // Sanitize path for use as a directory name
    let sanitized = path.replace(['/', '\\', ':'], "_");
    let dir = format!("/tmp/flamel-history/{session_id}/{sanitized}");
    let _ = std::fs::create_dir_all(&dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = std::fs::write(format!("{dir}/{ts}.bak"), content);
    // Keep only last 10 backups per file
    if let Ok(mut entries) = std::fs::read_dir(&dir) {
        let mut files: Vec<_> = entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
            .collect();
        files.sort_by_key(|e| e.file_name());
        if files.len() > 10 {
            for f in &files[..files.len() - 10] {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
}

#[path = "../mcp_modules/fs.rs"]
mod mcp_fs;
#[path = "../mcp_modules/symbols.rs"]
mod mcp_symbols;
// LSP is disabled. Keep the module source in-tree for possible future re-enable.
// #[path = "../mcp_modules/lsp.rs"]  mod mcp_lsp;

const SHELL_RUN_OUTPUT_CAP: usize = 256 * 1024;
const SHELL_RUN_TIMEOUT_DRAIN_MS: u64 = 500;
const SHELL_RUN_TERM_GRACE_MS: u64 = 1500;
const MCP_LOG_LINE_CHAR_CAP: usize = 8_000;
const SHELL_RUN_LIVE_LOG_CHAR_CAP: usize = 128_000;

fn cap_chars(input: &str, max_chars: usize, marker: &str) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }

    let keep = max_chars.saturating_sub(marker.chars().count()).max(1);
    let mut capped: String = input.chars().take(keep).collect();
    capped.push_str(marker);
    capped
}

fn send_shell_run_log(line: &str, live_chars: &mut usize, omitted_lines: &mut usize) {
    let line_chars = line.chars().count().saturating_add(1);
    if *live_chars >= SHELL_RUN_LIVE_LOG_CHAR_CAP {
        *omitted_lines += 1;
        return;
    }

    if (*live_chars).saturating_add(line_chars) <= SHELL_RUN_LIVE_LOG_CHAR_CAP {
        send_mcp_log(line);
        *live_chars += line_chars;
        return;
    }

    let remaining = SHELL_RUN_LIVE_LOG_CHAR_CAP.saturating_sub(*live_chars);
    let capped = cap_chars(
        line,
        remaining,
        "\n[... live shell output truncated; final tool result contains capped output ...]",
    );
    send_mcp_log(&capped);
    *live_chars = SHELL_RUN_LIVE_LOG_CHAR_CAP;
    *omitted_lines += 1;
}

fn send_shell_run_omitted_notice(omitted_lines: &mut usize) {
    if *omitted_lines == 0 {
        return;
    }
    send_mcp_log(&format!(
        "[... {} shell output lines omitted from live stream; final tool result contains capped output ...]",
        *omitted_lines
    ));
    *omitted_lines = 0;
}

fn append_capped_shell_output(output: &mut String, line: &str) {
    const MARKER: &str = "\n[... shell output truncated to last 256KB ...]\n";

    output.push_str(line);
    output.push('\n');

    if output.len() <= SHELL_RUN_OUTPUT_CAP {
        return;
    }

    let keep = SHELL_RUN_OUTPUT_CAP.saturating_sub(MARKER.len()).max(1);
    let mut start = output.len().saturating_sub(keep);
    while start < output.len() && !output.is_char_boundary(start) {
        start += 1;
    }

    let tail = output[start..].to_string();
    output.clear();
    output.push_str(MARKER);
    output.push_str(&tail);
}

fn drain_shell_output_for(
    rx: &std::sync::mpsc::Receiver<String>,
    output: &mut String,
    duration: std::time::Duration,
    live_chars: &mut usize,
    omitted_lines: &mut usize,
    command_event: Option<&ShellCommandEventContext>,
) {
    let deadline = std::time::Instant::now() + duration;
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(25)) {
            Ok(line) => {
                if let Some(context) = command_event {
                    emit_shell_output_line(context, &line);
                }
                send_shell_run_log(&line, live_chars, omitted_lines);
                append_capped_shell_output(output, &line);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if std::time::Instant::now() >= deadline {
                    break;
                }
            }
        }
    }
}

fn configure_shell_child(cmd: &mut std::process::Command) {
    flamel::mcp_parent_guard::configure_child(cmd);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Put the shell and its children in their own process group so timeout
        // cleanup can kill the whole command tree, not only the parent shell.
        cmd.process_group(0);
    }
}

const BROWSER_AUTOMATION_ENV: [(&str, &str); 6] = [
    ("PUPPETEER_SKIP_DOWNLOAD", "true"),
    ("PUPPETEER_SKIP_CHROMIUM_DOWNLOAD", "true"),
    ("PUPPETEER_EXECUTABLE_PATH", "/usr/local/bin/google-chrome"),
    ("CHROME_BIN", "/usr/local/bin/google-chrome"),
    ("PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD", "1"),
    ("NODE_PATH", "/usr/lib/node_modules"),
];

fn inject_browser_automation_env(cmd: &mut std::process::Command) {
    for (key, value) in BROWSER_AUTOMATION_ENV {
        if std::env::var(key).map(|v| v.is_empty()).unwrap_or(true) {
            cmd.env(key, value);
        }
    }
}

fn inject_shell_auth_env(cmd: &mut std::process::Command, auth_env: &[(String, String)]) {
    inject_browser_automation_env(cmd);
    for (key, value) in auth_env {
        cmd.env(key, value);
    }
}

fn github_cli_auth_env(base_url: &str, token: &str) -> Vec<(String, String)> {
    if token.is_empty() {
        return Vec::new();
    }

    let user_id = flamel_user_id();
    if user_id.is_empty() {
        return Vec::new();
    }

    let accounts = match call_api_with_user(
        "GET",
        "/api/internal/git-credentials",
        None,
        base_url,
        token,
        &user_id,
    ) {
        Ok(Value::Array(accounts)) => accounts,
        _ => return Vec::new(),
    };

    let Some(token) = select_github_cli_token(&accounts) else {
        return Vec::new();
    };

    vec![
        ("GH_TOKEN".to_string(), token.clone()),
        ("GITHUB_TOKEN".to_string(), token),
    ]
}

fn select_github_cli_token(accounts: &[Value]) -> Option<String> {
    for account in accounts {
        let account_type = account.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let base_url = account
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let token = account.get("token").and_then(|v| v.as_str()).unwrap_or("");

        if token.is_empty() || token == "***" {
            continue;
        }

        if account_type == "github" || github_cli_host(base_url).as_deref() == Some("github.com") {
            return Some(token.to_string());
        }
    }

    None
}

fn github_cli_host(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let authority = without_scheme.split('/').next().unwrap_or("");
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    let host = host_port
        .split(':')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    match host.as_str() {
        "" => None,
        "api.github.com" => Some("github.com".to_string()),
        _ => Some(host),
    }
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: &str) {
    let pgid = format!("-{pid}");
    let _ = flamel::mcp_parent_guard::command("kill")
        .arg(signal)
        .arg("--")
        .arg(pgid)
        .status();
}

fn terminate_shell_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id();
        signal_process_group(pid, "-TERM");
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(SHELL_RUN_TERM_GRACE_MS);
        while std::time::Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
                Err(_) => break,
            }
        }
        signal_process_group(pid, "-KILL");
        let _ = child.wait();
    }

    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn panic_payload_to_string(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn panic_response(req: &Value, payload: &(dyn std::any::Any + Send)) -> Option<Value> {
    let id = req.get("id")?;
    let tool = req
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let result = tool_error_result(
        tool,
        &format!(
            "MCP tool handler panicked: {}",
            panic_payload_to_string(payload)
        ),
    );
    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
}

fn tool_error_result(tool: &str, error: &str) -> Value {
    flamel::mcp_gateway::bound_result(
        tool,
        json!({
            "content": [{"type": "text", "text": format!("Error: {error}")}],
            "isError": true
        }),
    )
}

/// Write an MCP log notification to stdout (no id, no response expected).
/// Used by shell_run to stream output lines to the client in real time.
/// Safe to call from inside handle_tool_call because the main loop does not
/// hold the stdout lock while handle_request is executing.
fn send_mcp_log(message: &str) {
    use io::Write as _;
    let message = cap_chars(
        message,
        MCP_LOG_LINE_CHAR_CAP,
        "\n[... MCP log line truncated ...]",
    );
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "notifications/message",
        "params": {
            "level": "info",
            "logger": "shell_run",
            "data": message
        }
    });
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    let _ = serde_json::to_writer(&mut lock, &notif);
    let _ = lock.write_all(b"\n");
    let _ = lock.flush();
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum McpProfile {
    #[default]
    Default,
    PersonalAssistant,
    Invalid(String),
}

impl McpProfile {
    fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None => Self::Default,
            Some(flamel::cli_mcp::PERSONAL_ASSISTANT_PROFILE) => Self::PersonalAssistant,
            Some(value) => Self::Invalid(value.to_owned()),
        }
    }

    fn label(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::PersonalAssistant => flamel::cli_mcp::PERSONAL_ASSISTANT_PROFILE,
            Self::Invalid(value) => value,
        }
    }

    fn allows_tool(&self, name: &str) -> bool {
        match self {
            Self::Default | Self::PersonalAssistant => true,
            Self::Invalid(_) => false,
        }
    }

    fn instructions(&self) -> &'static str {
        match self {
            Self::Default => {
                "Use Flamel tools as the primary software-engineering toolchain when available. Prefer Flamel file reading, structured editing, search, symbols, code navigation, shell, Git, planning, memory, and browser tools over overlapping built-ins. Read targets before editing, use symbol-aware operations for code, update approved plans as work progresses, and fall back to native tools only when the corresponding Flamel tool is unavailable or fails."
            }
            Self::PersonalAssistant => {
                "You are the native Flamel Assistant. Use the canonical Flamel MCP tools for files, code navigation, edits, shell commands, Git and hosted repository work, plans, memory, connected services, Jobs, and persistent browser sessions. Treat tool schemas, approvals, and authoritative service state as binding; never claim a mutation completed until its tool result confirms it."
            }
            Self::Invalid(_) => {
                "This MCP process has an invalid FLAMEL_MCP_PROFILE value. No tools are available until the managed profile is corrected."
            }
        }
    }
}

fn main() {
    flamel::mcp_parent_guard::install();
    let runtime = flamel::cli_mcp::discover_local_cli_runtime();
    let flamel_url = std::env::var("FLAMEL_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| runtime.as_ref().map(|runtime| runtime.api_base.clone()))
        .unwrap_or_else(|| "http://localhost:8181".to_string());
    let flamel_token = std::env::var("FLAMEL_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| runtime.as_ref().map(|runtime| runtime.mcp_token.clone()))
        .unwrap_or_default();
    let session_id = std::env::var("FLAMEL_SESSION_ID").unwrap_or_default();
    let project_path = flamel::cli_mcp::current_project_path();
    let mcp_profile = McpProfile::parse(
        std::env::var(flamel::cli_mcp::MCP_PROFILE_ENV)
            .ok()
            .as_deref(),
    );
    initialize_persona_registry(&flamel_url, &flamel_token);
    let stdin = io::stdin();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Dispatch each request in its own thread so parallel Agent tool calls
        // (and other slow tools) execute concurrently rather than queuing up.
        // Responses are written back via io::stdout().lock() which serialises
        // individual writes while allowing interleaving between requests.
        let url = flamel_url.clone();
        let tok = flamel_token.clone();
        let sid = session_id.clone();
        let pp = project_path.clone();
        let profile = mcp_profile.clone();
        std::thread::spawn(move || {
            let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handle_request(&req, &url, &tok, &sid, &pp, &profile)
            }))
            .unwrap_or_else(|payload| panic_response(&req, payload.as_ref()));

            if let Some(resp) = response {
                let stdout = io::stdout();
                let mut out = stdout.lock();
                let _ = serde_json::to_writer(&mut out, &resp);
                let _ = out.write_all(b"\n");
                let _ = out.flush();
            }
        });
    }
}

/// Coerce a Value into an array — handles the case where Claude encodes
/// array parameters as a JSON string instead of a proper JSON array.
fn coerce_array(v: Option<&Value>) -> Value {
    match v {
        None => json!([]),
        Some(Value::Array(_)) => v.unwrap().clone(),
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(json!([])),
        Some(Value::Null) => json!([]),
        _ => json!([]),
    }
}

fn call_api(
    method: &str,
    path: &str,
    body: Option<&Value>,
    base_url: &str,
    token: &str,
) -> Result<Value, String> {
    // Global user_id — set once at startup, threaded through via env var
    let user_id = flamel_user_id();
    call_api_with_user(method, path, body, base_url, token, &user_id)
}

fn flamel_user_id() -> String {
    std::env::var("FLAMEL_USER_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| flamel::cli_mcp::discover_local_cli_runtime().map(|runtime| runtime.user_id))
        .unwrap_or_else(|| flamel::FLAMEL_LOCAL_USER_ID.to_string())
}

fn call_local_secret_api(body: &Value, base_url: &str, token: &str) -> Result<Value, String> {
    if base_url.starts_with("ipc:") {
        return flamel::callback::call(
            "POST",
            "/api/internal/local-secret",
            Some(body),
            base_url,
            token,
            &flamel_user_id(),
        );
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| format!("Could not initialize local secret transport: {error}"))?;
    let mut request = client
        .post(format!("{base_url}/api/internal/local-secret"))
        .header("X-Flamel-Token", token)
        .header("x-flamel-user-id", flamel_user_id())
        .json(body);
    if let Ok(session_id) = std::env::var("FLAMEL_SESSION_ID")
        && !session_id.trim().is_empty()
    {
        request = request.header("x-flamel-session-id", session_id.trim());
    }
    let response = request
        .send()
        .map_err(|error| format!("Local secret request failed: {error}"))?;
    let status = response.status();
    let payload = response
        .json::<Value>()
        .map_err(|_| "Local secret response was invalid".to_string())?;
    if !status.is_success() {
        let error = payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Local secret request failed");
        return Err(format!("Local secret request failed ({status}): {error}"));
    }
    Ok(payload)
}

fn call_api_with_user(
    method: &str,
    path: &str,
    body: Option<&Value>,
    base_url: &str,
    token: &str,
    user_id: &str,
) -> Result<Value, String> {
    if base_url.starts_with("ipc:") {
        return flamel::callback::call(method, path, body, base_url, token, user_id);
    }
    let url = format!("{base_url}{path}");

    // Use a simple blocking HTTP client via std — no tokio needed for this tiny binary
    let mut cmd = flamel::mcp_parent_guard::command("curl");
    if matches!(
        path,
        "/api/internal/nested-event" | "/api/internal/agent-batch-status"
    ) {
        cmd.arg("--connect-timeout")
            .arg("2")
            .arg("--max-time")
            .arg("15");
    } else if path == "/api/runtime-catalog/personas" {
        cmd.arg("--connect-timeout")
            .arg("1")
            .arg("--max-time")
            .arg("3");
    }
    cmd.arg("-s")
        .arg("-X")
        .arg(method)
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-H")
        .arg(format!("X-Flamel-Token: {token}"))
        .arg("-w")
        .arg("\n%{http_code}"); // append HTTP status code on final line

    // Forward identity so server-side ownership checks work. The session header lets
    // the server recover the owner when older MCP processes lack FLAMEL_USER_ID.
    if !user_id.is_empty() {
        cmd.arg("-H").arg(format!("x-flamel-user-id: {user_id}"));
    }
    if let Ok(session_id) = std::env::var("FLAMEL_SESSION_ID") {
        let session_id = session_id.trim();
        if !session_id.is_empty() {
            cmd.arg("-H")
                .arg(format!("x-flamel-session-id: {session_id}"));
        }
    }
    for (variable, header) in [
        ("FLAMEL_ASSISTANT_ID", "x-flamel-assistant-id"),
        ("FLAMEL_DELEGATION_TASK_ID", "x-flamel-delegation-id"),
    ] {
        if let Ok(value) = std::env::var(variable) {
            let value = value.trim();
            if !value.is_empty() {
                uuid::Uuid::parse_str(value)
                    .map_err(|_| format!("{variable} must contain a UUID"))?;
                cmd.arg("-H").arg(format!("{header}: {value}"));
            }
        }
    }

    if let Some(b) = body {
        cmd.arg("-d").arg(b.to_string());
    }

    cmd.arg(&url);

    let output = cmd.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("HTTP request failed: {stderr}"));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    // Split off the trailing status line written by -w "%{http_code}"
    let (body_str, status_code) = raw.rsplit_once('\n').unwrap_or((&raw, ""));
    let status: u16 = status_code.trim().parse().unwrap_or(0);
    if status >= 400 {
        if status == 401
            && (token.trim().is_empty()
                || body_str.contains("Internal MCP authentication failed")
                || body_str.contains("sign in via flamel.dev"))
        {
            let cause = if token.trim().is_empty() {
                "missing"
            } else {
                "stale or rejected"
            };
            return Err(format!(
                "API error 401: Internal MCP authentication {cause}; restart this provider/MCP session so Flamel can inject a fresh token."
            ));
        }
        return Err(format!("API error {status}: {body_str}"));
    }

    serde_json::from_str(body_str).map_err(|e| format!("JSON parse error: {e}: {body_str}"))
}

#[derive(Clone, Debug)]
struct RuntimePersonaDefinition {
    markdown: String,
    body: String,
}

static PERSONA_REGISTRY: OnceLock<HashMap<String, RuntimePersonaDefinition>> = OnceLock::new();

/// Strip YAML frontmatter (between `---` markers) from a markdown persona file,
/// returning just the body — which is the actual persona system prompt.
fn strip_frontmatter(md: &str) -> &str {
    let Some(after_open) = md
        .strip_prefix("---\n")
        .or_else(|| md.strip_prefix("---\r\n"))
    else {
        return md;
    };
    // Find the closing `---` on its own line.
    let mut idx = 0usize;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            let body_start = idx + line.len();
            return after_open[body_start..]
                .trim_start_matches(['\r', '\n'])
                .trim_start();
        }
        idx += line.len();
    }
    md
}

fn parse_persona_catalog(
    value: Value,
) -> Result<HashMap<String, RuntimePersonaDefinition>, String> {
    let entries = value
        .as_array()
        .ok_or_else(|| "Runtime Catalog personas response must be an array".to_string())?;
    if entries.len() > 64 {
        return Err("Runtime Catalog personas response exceeds its limit".to_string());
    }
    let mut registry = HashMap::new();
    for entry in entries {
        let slug = entry
            .get("slug")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 96)
            .ok_or_else(|| "Runtime Catalog persona slug is invalid".to_string())?;
        let markdown = entry
            .get("content")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 128 * 1024)
            .ok_or_else(|| "Runtime Catalog persona content is invalid".to_string())?;
        let body = strip_frontmatter(markdown).trim();
        if body.is_empty()
            || frontmatter_field(markdown, "name").as_deref() != Some(slug)
            || registry.contains_key(slug)
        {
            return Err("Runtime Catalog persona identity is invalid".to_string());
        }
        registry.insert(
            slug.to_string(),
            RuntimePersonaDefinition {
                markdown: markdown.to_string(),
                body: body.to_string(),
            },
        );
    }
    Ok(registry)
}

fn initialize_persona_registry(base_url: &str, token: &str) {
    let registry = if token.is_empty() {
        HashMap::new()
    } else {
        call_api(
            "GET",
            "/api/runtime-catalog/personas",
            None,
            base_url,
            token,
        )
        .and_then(parse_persona_catalog)
        .unwrap_or_default()
    };
    let _ = PERSONA_REGISTRY.set(registry);
}

fn persona_registry() -> &'static HashMap<String, RuntimePersonaDefinition> {
    PERSONA_REGISTRY.get_or_init(|| {
        #[cfg(test)]
        {
            let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("repository root");
            let index: Value = serde_json::from_str(
                &std::fs::read_to_string(repository.join("runtime-catalog/personas.json"))
                    .expect("runtime persona catalog"),
            )
            .expect("runtime persona catalog JSON");
            let entries = index["personas"]
                .as_array()
                .expect("runtime persona entries")
                .iter()
                .map(|persona| {
                    let source = persona["source"].as_str().expect("persona source");
                    json!({
                        "slug": persona["slug"],
                        "content": std::fs::read_to_string(repository.join(source))
                            .expect("runtime persona source"),
                    })
                })
                .collect::<Vec<_>>();
            return parse_persona_catalog(Value::Array(entries))
                .expect("valid runtime persona catalog");
        }

        #[cfg(not(test))]
        HashMap::new()
    })
}

fn runtime_agent_markdown(subagent_type: &str) -> Option<&'static str> {
    persona_registry()
        .get(subagent_type)
        .map(|persona| persona.markdown.as_str())
}

fn safe_persona_file_stem(subagent_type: &str) -> String {
    let safe: String = subagent_type
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '_',
        })
        .collect();
    if safe.is_empty() {
        "persona".to_string()
    } else {
        safe
    }
}

fn persona_content_hash(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn frontmatter_field(md: &str, key: &str) -> Option<String> {
    let after_open = md
        .strip_prefix("---\n")
        .or_else(|| md.strip_prefix("---\r\n"))?;
    for line in after_open.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        let Some((field_key, raw_value)) = trimmed.split_once(':') else {
            continue;
        };
        if field_key.trim() != key {
            continue;
        }
        let value = raw_value.trim();
        if value.is_empty() || matches!(value, "|" | "|-" | "|+" | ">" | ">-" | ">+") {
            return None;
        }
        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value)
            .trim();
        return (!unquoted.is_empty()).then(|| unquoted.to_string());
    }
    None
}

fn persona_lifecycle_manifest(
    subagent_type: &str,
    persona_md: Option<&str>,
    persona_path: Option<&str>,
) -> Value {
    if let Some(md) = persona_md {
        json!({
            "subagent_type": subagent_type,
            "name": frontmatter_field(md, "name").unwrap_or_else(|| subagent_type.to_string()),
            "description": frontmatter_field(md, "description"),
            "source": "runtime_catalog",
            "persona_path": persona_path,
            "persona_hash": persona_content_hash(md),
            "version": frontmatter_field(md, "version").unwrap_or_else(|| "1.0.0".to_string()),
            "lifecycle_stage": "production",
            "approval_status": "approved",
            "promotion_status": "approved",
            "eval_status": frontmatter_field(md, "eval_status").unwrap_or_else(|| "not_run".to_string()),
            "eval_score": frontmatter_field(md, "eval_score").and_then(|value| value.parse::<f64>().ok()),
            "regression_status": frontmatter_field(md, "regression_status").unwrap_or_else(|| "not_run".to_string()),
            "rollback_version": frontmatter_field(md, "rollback_version"),
        })
    } else {
        json!({
            "subagent_type": subagent_type,
            "name": subagent_type,
            "description": null,
            "source": "fallback",
            "persona_path": persona_path,
            "persona_hash": null,
            "version": "0.0.0",
            "lifecycle_stage": "draft",
            "approval_status": "unreviewed",
            "promotion_status": "not_requested",
            "eval_status": "not_run",
            "eval_score": null,
            "regression_status": "not_run",
            "rollback_version": null,
        })
    }
}

fn persona_manifest_path_for(persona_path: &str, subagent_type: &str) -> String {
    let mut path = std::path::PathBuf::from(persona_path);
    path.set_file_name(format!(
        "{}-manifest.json",
        safe_persona_file_stem(subagent_type)
    ));
    path.to_string_lossy().into_owned()
}

fn materialize_persona_markdown(
    run_id: &str,
    subagent_type: &str,
    cwd: &str,
) -> Result<Option<String>, String> {
    let Some(persona_md) = runtime_agent_markdown(subagent_type) else {
        return Ok(None);
    };

    let dir = persona_materialization_base(cwd).join(run_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create persona dir {}: {e}", dir.display()))?;

    let persona_path = dir.join(format!("{}.md", safe_persona_file_stem(subagent_type)));
    std::fs::write(&persona_path, persona_md).map_err(|e| {
        format!(
            "failed to write persona markdown {}: {e}",
            persona_path.display()
        )
    })?;
    let persona_path_string = persona_path.to_string_lossy().into_owned();
    let manifest_path = persona_manifest_path_for(&persona_path_string, subagent_type);
    let manifest =
        persona_lifecycle_manifest(subagent_type, Some(persona_md), Some(&persona_path_string));
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("failed to serialize persona manifest: {e}"))?;
    std::fs::write(&manifest_path, manifest_json)
        .map_err(|e| format!("failed to write persona manifest {manifest_path}: {e}"))?;

    Ok(Some(persona_path_string))
}

/// Resolve a subagent_type name to a persona system-prompt prefix. Returns
/// `Cow::Borrowed` for hardcoded specials (Explore, Plan, general-purpose)
/// and `Cow::Owned` for any persona backed by a markdown file in `agents/`.
/// Unknown types fall back to a general-purpose message that announces the
/// miss so the caller's chain-of-thought reflects the fallback.
fn resolve_persona(subagent_type: &str) -> std::borrow::Cow<'static, str> {
    use std::borrow::Cow;
    match subagent_type {
        "Explore" => Cow::Borrowed(
            "You are an Explore agent — specialized for fast, read-only codebase navigation. Use mcp__flamel__find_symbol, mcp__flamel__flamel_search, mcp__flamel__get_symbols_overview, mcp__flamel__list_dir. Keep responses tight; answer only what was asked. Do NOT edit files.",
        ),
        "Plan" => Cow::Borrowed(
            "You are a Plan agent — specialized for architectural planning. Inspect enough of the repository to resolve implementation questions before handoff. Use mcp__flamel__decision_checkpoint only for ambiguous, risky, or materially consequential choices; do not expose private chain-of-thought. Produce structured phases whose items identify verified files and symbols, behavior and data/control-flow contracts, dependencies and ordering, item-scoped validation and acceptance evidence, risks or rollback constraints, a structured reasoning_floor, and genuinely independent delegation opportunities. reasoning_floor is the minimum safe tier for Auto execution (low, medium, high, xhigh, max, or ultra), not an exact tier; choose it while you have the strongest planning context. The executor may raise it when concrete evidence demands more reasoning but must never lower it. The result must let a lower-reasoning executor work directly without repeating broad discovery. Do NOT implement code.",
        ),
        "general-purpose" | "" => Cow::Borrowed(
            "You are a general-purpose sub-agent. Complete the user's task using flamel MCP tools only. Unless the persona explicitly says read-only or planning-only, you may inspect files, edit files, run shell/build/test commands, use browser automation, and use the typed git tool when the task requires actual implementation. Return a concise summary of what you did plus any concrete deliverables the caller asked for.",
        ),
        other => {
            if let Some(persona) = persona_registry().get(other) {
                Cow::Owned(persona.body.clone())
            } else {
                Cow::Borrowed(
                    "You are a general-purpose sub-agent (requested persona was unknown). Complete the user's task using flamel MCP tools only. Unless the task is explicitly research-only, use the available flamel MCP tools to implement changes, run checks, and return concrete deliverables when asked.",
                )
            }
        }
    }
}

/// Cap a string at `max` bytes, appending `marker` when truncation happens.
/// Respects UTF-8 char boundaries. Pure helper so the truncation behavior in
/// `run_subagent` is unit-testable without spawning a subprocess.
fn cap_output(s: &str, max: usize, marker: &str) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    format!("{}{}", &s[..cut], marker)
}

/// Build the `AGENT_RUN:{run_id}\n{payload}` envelope the frontend parses on
/// tool_result to correlate buffered nested events. Pure helper.
fn format_agent_run_payload(run_id: &str, payload: &str) -> String {
    format!("AGENT_RUN:{run_id}\n{payload}")
}

const DEFAULT_AGENT_TIMEOUT_SECS: u64 = 600;
const DEFAULT_AGENT_BATCH_IDLE_TIMEOUT_SECS: u64 = 600;
const MIN_AGENT_BATCH_IDLE_TIMEOUT_SECS: u64 = 30;
const AGENT_MONITOR_INTERVAL_SECS: u64 = 5;

fn normalized_agent_batch_idle_timeout_secs(requested: Option<u64>) -> u64 {
    requested
        .unwrap_or(DEFAULT_AGENT_BATCH_IDLE_TIMEOUT_SECS)
        .max(MIN_AGENT_BATCH_IDLE_TIMEOUT_SECS)
}

fn path_is_temp_dir(path: &std::path::Path) -> bool {
    let text = path.to_string_lossy();
    text == "/tmp"
        || text.starts_with("/tmp/")
        || text == "/var/tmp"
        || text.starts_with("/var/tmp/")
}

fn usable_non_temp_path(value: String) -> Option<std::path::PathBuf> {
    if value.trim().is_empty() {
        return None;
    }
    let path = std::path::PathBuf::from(value);
    (!path_is_temp_dir(&path)).then_some(path)
}

fn flamel_storage_base(cwd: &str) -> std::path::PathBuf {
    std::env::var("FLAMEL_USER_HOME")
        .ok()
        .and_then(usable_non_temp_path)
        .or_else(|| {
            std::env::var("FLAMEL_CODEX_HOME")
                .ok()
                .and_then(usable_non_temp_path)
                .and_then(|home| home.parent().map(|parent| parent.to_path_buf()))
                .filter(|parent| !parent.as_os_str().is_empty() && !path_is_temp_dir(parent))
        })
        .or_else(|| std::env::var("HOME").ok().and_then(usable_non_temp_path))
        .unwrap_or_else(|| {
            let project_path = std::path::PathBuf::from(cwd);
            if !path_is_temp_dir(&project_path) {
                project_path
            } else {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            }
        })
}

fn persona_materialization_base(cwd: &str) -> std::path::PathBuf {
    flamel_storage_base(cwd)
        .join(".flamel")
        .join("agent-personas")
}

fn codex_subagent_home_for_run(run_id: &str, cwd: &str) -> String {
    flamel_storage_base(cwd)
        .join(".flamel")
        .join("codex-subagents")
        .join(run_id)
        .to_string_lossy()
        .into_owned()
}

fn codex_reasoning_effort() -> String {
    let value =
        std::env::var("FLAMEL_CODEX_REASONING_EFFORT").unwrap_or_else(|_| "high".to_string());
    if matches!(
        value.as_str(),
        "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
    ) {
        value
    } else {
        "high".to_string()
    }
}

fn inherit_codex_runtime_state(parent_home: &str, child_home: &str) {
    let parent = std::path::Path::new(parent_home);
    let child = std::path::Path::new(child_home);
    for file in ["auth.json", "config.toml"] {
        let source = parent.join(file);
        if source.exists() {
            if let Err(e) = std::fs::copy(&source, child.join(file)) {
                eprintln!("[flamel_mcp] failed to copy codex {file}: {e}");
            }
        }
    }

    for directory in ["skills", "plugins", "generated_images"] {
        let source = parent.join(directory);
        let destination = child.join(directory);
        if !source.is_dir() || destination.exists() {
            continue;
        }
        #[cfg(unix)]
        if let Err(e) = std::os::unix::fs::symlink(&source, &destination) {
            eprintln!("[flamel_mcp] failed to link codex {directory}: {e}");
        }
        #[cfg(windows)]
        if let Err(e) = std::os::windows::fs::symlink_dir(&source, &destination) {
            eprintln!("[flamel_mcp] failed to link codex {directory}: {e}");
        }
    }
}

fn command_on_path(command: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    #[cfg(windows)]
    let names: Vec<String> = if command.to_ascii_lowercase().ends_with(".exe") {
        vec![command.to_string()]
    } else {
        vec![command.to_string(), format!("{command}.exe")]
    };
    #[cfg(not(windows))]
    let names = vec![command.to_string()];

    for dir in std::env::split_paths(&path) {
        for name in &names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn resolve_subagent_cli_binary(provider: &str) -> Result<String, String> {
    if !matches!(provider, "claude" | "codex") {
        return Err(format!(
            "unsupported authenticated provider `{provider}`; connect and explicitly select Codex or Claude"
        ));
    }

    if let Ok(value) = std::env::var("FLAMEL_PROVIDER_BINARY") {
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }

    command_on_path(provider).ok_or_else(|| {
        format!(
            "FLAMEL_PROVIDER_BINARY is not set and the authenticated `{provider}` CLI is not on PATH; Flamel will not fall back to another provider"
        )
    })
}

fn configure_subagent_compaction_guard(cmd: &mut std::process::Command, provider: &str) {
    cmd.env_remove("CLAUDE_AUTOCOMPACT_PCT_OVERRIDE");
    if provider == "codex" {
        cmd.arg("-c")
            .arg(flamel::claude::codex_provider::CODEX_NATIVE_COMPACTION_LIMIT_CONFIG);
    } else {
        cmd.arg("--autocompact")
            .arg(flamel::claude::spawn::CLAUDE_NATIVE_COMPACTION_CEILING);
    }
}

#[path = "flamel_mcp/agents.rs"]
mod mcp_agents;
use mcp_agents::AgentBatchResult;
#[cfg(test)]
use mcp_agents::{
    parse_codex_mcp_servers_config, prepare_child_mcp_servers_config, write_codex_subagent_config,
};
#[path = "flamel_mcp/catalog.rs"]
mod mcp_catalog;
use mcp_catalog::*;
#[path = "flamel_mcp/control.rs"]
mod mcp_control;
#[path = "flamel_mcp/engineering.rs"]
mod mcp_engineering;
#[path = "flamel_mcp/media.rs"]
mod mcp_media;
#[path = "flamel_mcp/protocol.rs"]
mod mcp_protocol;
use mcp_protocol::*;
#[path = "flamel_mcp/shell.rs"]
mod mcp_shell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolFamily {
    Agents,
    Control,
    Engineering,
    Shell,
    Media,
}

fn tool_family(name: &str) -> Option<ToolFamily> {
    match name {
        "Agent" | "AgentBatch" | "AgentBatchStatus" => Some(ToolFamily::Agents),
        "decision_checkpoint"
        | "assistant"
        | "job"
        | "AskUserQuestion"
        | "plan"
        | "flamel_plan_create"
        | "flamel_plan_get"
        | "flamel_plan_list"
        | "flamel_plan_update_item"
        | "flamel_plan_add_items"
        | "flamel_plan_delete_item"
        | "flamel_plan_replace_items"
        | "flamel_plan_rename"
        | "flamel_plan_mark_finished"
        | "flamel_plan_start"
        | "flamel_plan_resume"
        | "flamel_plan_pause"
        | "flamel_plan_block"
        | "file_history"
        | "revert_file"
        | "list_file_edits"
        | "checkpoint"
        | "rollback"
        | "flamel_context_stats"
        | "diagnostics"
        | "capability_search"
        | "capability_execute"
        | "project_intelligence"
        | "skill_search"
        | "skill_catalog"
        | "work" => Some(ToolFamily::Control),
        "code_nav"
        | "flamel_search"
        | "get_symbols_overview"
        | "find_symbol"
        | "find_referencing_symbols"
        | "get_imports"
        | "gather_context"
        | "dependency_graph"
        | "file_edit"
        | "flamel_replace"
        | "replace_content"
        | "multi_replace"
        | "replace_symbol_body"
        | "insert_before_symbol"
        | "insert_after_symbol"
        | "create_text_file"
        | "diff_apply"
        | "format_file"
        | "preview_edit"
        | "file_read"
        | "flamel_read"
        | "read_file"
        | "batch_read"
        | "file_summary"
        | "flamel_glob"
        | "find_file"
        | "list_dir"
        | "code_quality"
        | "build_errors"
        | "run_tests"
        | "explain_error"
        | "find_code_issues"
        | "get_recent_changes" => Some(ToolFamily::Engineering),
        "shell" | "flamel_shell" | "shell_run" | "shell_start" | "shell_read" | "shell_write"
        | "shell_kill" | "shell_status" | "multi_shell" => Some(ToolFamily::Shell),
        "assets" | "media" | "fetch_image" | "fetch_model3d" | "create_model3d" => {
            Some(ToolFamily::Media)
        }
        _ => None,
    }
}

fn handle_tool_call(
    name: &str,
    args: &Value,
    base_url: &str,
    token: &str,
    session_id: &str,
    project_path: &str,
) -> Result<String, String> {
    match tool_family(name) {
        Some(ToolFamily::Agents) => mcp_agents::handle(name, args),
        Some(ToolFamily::Control) => {
            mcp_control::handle(name, args, base_url, token, session_id, project_path)
        }
        Some(ToolFamily::Engineering) => {
            mcp_engineering::handle(name, args, base_url, token, session_id, project_path)
        }
        Some(ToolFamily::Shell) => {
            mcp_shell::handle(name, args, base_url, token, session_id, project_path)
        }
        Some(ToolFamily::Media) => {
            mcp_media::handle(name, args, base_url, token, session_id, project_path)
        }
        None => Err(format!("Unknown tool: {name}")),
    }
}

#[cfg(test)]
#[path = "flamel_mcp/tests.rs"]
mod tests;
