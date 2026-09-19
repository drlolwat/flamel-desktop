use super::*;

pub(super) fn handle(name: &str, args: &Value) -> Result<String, String> {
    match name {
        "Agent" => run_subagent(args),
        "AgentBatch" => run_agent_batch(args),
        "AgentBatchStatus" => run_agent_batch_status(args),
        _ => Err(format!("Unknown agent tool: {name}")),
    }
}

pub(super) fn durable_batch_status(args: &Value) -> Result<Option<String>, String> {
    let session = std::env::var("FLAMEL_SESSION_ID").unwrap_or_default();
    let url = std::env::var("FLAMEL_URL").unwrap_or_default();
    let token = std::env::var("FLAMEL_TOKEN").unwrap_or_default();
    if session.is_empty() || url.is_empty() || token.is_empty() {
        return Ok(None);
    }
    let body = json!({"sessionId":session,"batch_id":args.get("batch_id"),
        "parentRunId":std::env::var("FLAMEL_PARENT_AGENT_RUN_ID").unwrap_or_default()});
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(args["timeout_ms"].as_u64().unwrap_or(0).min(3_600_000));
    let mut observed = None;
    loop {
        let status = call_api(
            "POST",
            "/api/internal/agent-batch-status",
            Some(&body),
            &url,
            &token,
        )?;
        let update = status["updated_at"].clone();
        if status["status"] != "running"
            || std::time::Instant::now() >= deadline
            || observed
                .as_ref()
                .is_some_and(|previous| previous != &update)
        {
            return Ok(Some(status.to_string()));
        }
        observed = Some(update);
        std::thread::sleep(
            std::time::Duration::from_secs(1)
                .min(deadline.saturating_duration_since(std::time::Instant::now())),
        );
    }
}

fn provider_terminal_outcome(event: &Value, codex: bool) -> Option<Result<(), String>> {
    match event["type"].as_str()? {
        "turn.completed" if codex => Some(Ok(())),
        "turn.failed" if codex => Some(Err(event["error"].to_string())),
        "error" if codex => Some(Err(event["message"]
            .as_str()
            .unwrap_or("Provider reported an error")
            .to_string())),
        "result" if !codex => Some(
            if event["is_error"] == true
                || event["subtype"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("error"))
            {
                Err(event["result"]
                    .as_str()
                    .unwrap_or("Provider reported a failed run")
                    .to_string())
            } else {
                Ok(())
            },
        ),
        _ => None,
    }
}

pub(super) fn prepare_child_mcp_servers_config(
    mcp_servers_json: &str,
    child_system_prompt: &str,
    next_depth: u32,
    parent_run_id: &str,
) -> Result<String, String> {
    let mut servers = parse_codex_mcp_servers_config(mcp_servers_json)?;

    if let Some(flamel_entry) = servers.get_mut("flamel").and_then(|v| v.as_object_mut()) {
        let env = flamel_entry
            .entry("env".to_string())
            .or_insert_with(|| json!({}));
        if !env.is_object() {
            *env = json!({});
        }
        if let Some(env_obj) = env.as_object_mut() {
            env_obj.insert(
                "FLAMEL_AGENT_DEPTH".to_string(),
                Value::String(next_depth.to_string()),
            );
            env_obj.insert(
                "FLAMEL_SYSTEM_PROMPT".to_string(),
                Value::String(child_system_prompt.to_string()),
            );
            env_obj.insert(
                "FLAMEL_PARENT_AGENT_RUN_ID".to_string(),
                Value::String(parent_run_id.to_string()),
            );
            env_obj.remove("FLAMEL_MCP_SERVERS");
        }
    }

    let snapshot_without_self_reference = json!({ "mcpServers": servers }).to_string();
    let mut active: Value = serde_json::from_str(&snapshot_without_self_reference)
        .map_err(|e| format!("failed to reparse child MCP config: {e}"))?;

    if let Some(flamel_env) = active
        .get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
        .and_then(|servers| servers.get_mut("flamel"))
        .and_then(|v| v.as_object_mut())
        .and_then(|flamel| flamel.get_mut("env"))
        .and_then(|v| v.as_object_mut())
    {
        flamel_env.insert(
            "FLAMEL_MCP_SERVERS".to_string(),
            Value::String(snapshot_without_self_reference),
        );
    }

    Ok(active.to_string())
}

pub(super) fn parse_codex_mcp_servers_config(
    mcp_servers_json: &str,
) -> Result<serde_json::Map<String, Value>, String> {
    let servers: Value = serde_json::from_str(mcp_servers_json)
        .map_err(|e| format!("failed to parse FLAMEL_MCP_SERVERS for codex config: {e}"))?;

    let normalize = |v: Value| -> Result<serde_json::Map<String, Value>, String> {
        if let Some(inner) = v.get("mcpServers").and_then(|v| v.as_object()) {
            Ok(inner.clone())
        } else if let Some(o) = v.as_object() {
            Ok(o.clone())
        } else {
            Err("MCP config is not an object".to_string())
        }
    };

    if let Some(path) = servers.as_str() {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read MCP config file {path}: {e}"))?;
        let parsed: Value = serde_json::from_str(&contents)
            .map_err(|e| format!("failed to parse MCP config file: {e}"))?;
        normalize(parsed)
    } else {
        normalize(servers)
    }
}

/// Write a Codex-compatible config.toml from the FLAMEL_MCP_SERVERS JSON string.
/// This mirrors the logic in codex_provider.rs but lives here because flamel_mcp
/// is a standalone binary that doesn't link against the server library.
pub(super) fn write_codex_subagent_config(
    mcp_servers_json: &str,
    codex_home: &str,
    instructions: Option<&str>,
) {
    let obj = match parse_codex_mcp_servers_config(mcp_servers_json) {
        Ok(obj) => obj,
        Err(e) => {
            eprintln!("[flamel_mcp] {e}");
            return;
        }
    };

    let mut managed_toml = String::new();

    managed_toml.push_str(&format!(
        "model_reasoning_effort = \"{}\"\n",
        codex_reasoning_effort()
    ));

    // Inject system prompt as developer instructions
    if let Some(instr) = instructions {
        if !instr.is_empty() {
            managed_toml.push_str(&format!("instructions = \"{}\"\n\n", escape_toml(instr)));
        }
    }

    for (name, config) in &obj {
        let command = config.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if command.is_empty() {
            continue;
        }

        managed_toml.push_str(&format!("[mcp_servers.{name}]\n"));
        managed_toml.push_str(&format!("command = \"{}\"\n", escape_toml(command)));

        if let Some(args) = config.get("args").and_then(|v| v.as_array()) {
            let args_str: Vec<String> = args
                .iter()
                .filter_map(|a| a.as_str())
                .map(|a| format!("\"{}\"", escape_toml(a)))
                .collect();
            managed_toml.push_str(&format!("args = [{}]\n", args_str.join(", ")));
        } else {
            managed_toml.push_str("args = []\n");
        }

        let tool_timeout_sec = config
            .get("tool_timeout_sec")
            .and_then(|v| v.as_u64())
            .or_else(|| (name == "git").then_some(600));
        if let Some(timeout) = tool_timeout_sec {
            managed_toml.push_str(&format!("tool_timeout_sec = {timeout}\n"));
        }

        if let Some(timeout) = config.get("startup_timeout_sec").and_then(|v| v.as_u64()) {
            managed_toml.push_str(&format!("startup_timeout_sec = {timeout}\n"));
        }

        if let Some(env) = config.get("env").and_then(|v| v.as_object()) {
            managed_toml.push_str(&format!("[mcp_servers.{name}.env]\n"));
            for (k, v) in env {
                let val = match v {
                    Value::String(s) => format!("\"{}\"", escape_toml(s)),
                    other => format!("\"{}\"", escape_toml(&other.to_string())),
                };
                managed_toml.push_str(&format!("{k} = {val}\n"));
            }
        }

        managed_toml.push('\n');
    }

    let config_path = std::path::Path::new(codex_home).join("config.toml");
    let managed = match managed_toml.parse::<toml_edit::DocumentMut>() {
        Ok(doc) => doc,
        Err(e) => {
            eprintln!("[flamel_mcp] failed to construct codex config: {e}");
            return;
        }
    };
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut merged = existing
        .parse::<toml_edit::DocumentMut>()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());
    merged["model_reasoning_effort"] = toml_edit::value(codex_reasoning_effort());
    if let Some(instr) = instructions.filter(|value| !value.is_empty()) {
        merged["instructions"] = toml_edit::value(instr);
    }
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
        eprintln!("[flamel_mcp] failed to write codex config.toml: {e}");
    }
}

pub(super) fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Spawn a child `claude` subprocess for the Agent tool.
/// Reads required env vars set by the parent flamel server (or parent Agent call).
/// Blocks until the child exits or its idle watchdog fires.
///
/// Uses `--output-format stream-json` so we can parse each event as JSON and
/// forward it live to the parent flamel server via `POST /api/internal/nested-event`.
/// The tool_result output starts with `AGENT_RUN:{run_id}\n` so the frontend can
/// correlate buffered nested events to this specific invocation.
pub(super) fn run_subagent(args: &Value) -> Result<String, String> {
    const MAX_DEPTH: u32 = 5;
    const OUTPUT_CAP: usize = 50 * 1024; // 50KB
    const TRUNCATE_MARKER: &str = "\n\n[... sub-agent output truncated at 50KB ...]";
    let agent_idle_timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_AGENT_TIMEOUT_SECS)
        .max(MIN_AGENT_BATCH_IDLE_TIMEOUT_SECS);

    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("(unnamed)");
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or("prompt required")?;
    let subagent_type = args
        .get("subagent_type")
        .and_then(|v| v.as_str())
        .unwrap_or("general-purpose");
    let model_override = args.get("model").and_then(|v| v.as_str());

    // Env var checks — bail with a clear error if the parent forgot to set them.
    let provider = std::env::var("FLAMEL_PROVIDER").map_err(|_| {
        "FLAMEL_PROVIDER is not set; refusing to guess a sub-agent provider".to_string()
    })?;
    let cli_bin = resolve_subagent_cli_binary(&provider)?;
    let mcp_servers_json = std::env::var("FLAMEL_MCP_SERVERS").map_err(|_| {
        "FLAMEL_MCP_SERVERS env var not set — parent spawn context is missing".to_string()
    })?;
    let parent_system_prompt = std::env::var("FLAMEL_SYSTEM_PROMPT").unwrap_or_default();
    let disallowed_tools = std::env::var("FLAMEL_DISALLOWED_TOOLS").unwrap_or_else(|_| {
        "Agent,Task,Read,Write,Edit,Bash,Grep,Glob,TodoWrite,NotebookEdit".to_string()
    });
    let cwd = std::env::var("FLAMEL_PROJECT_PATH").unwrap_or_else(|_| ".".to_string());
    let is_codex = provider == "codex";

    // These are optional — if unavailable (e.g. running flamel_mcp standalone outside
    // a chat session), we skip the event forwarding but still run the sub-agent.
    let session_id = std::env::var("FLAMEL_SESSION_ID").unwrap_or_default();
    let flamel_url = std::env::var("FLAMEL_URL").unwrap_or_default();
    let flamel_token = std::env::var("FLAMEL_TOKEN").unwrap_or_default();
    let can_forward = !session_id.is_empty() && !flamel_url.is_empty() && !flamel_token.is_empty();
    let parent_run_id = args
        .get("parent_run_id")
        .and_then(|v| v.as_str())
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            std::env::var("FLAMEL_PARENT_AGENT_RUN_ID")
                .ok()
                .filter(|id| !id.is_empty())
        });

    // Depth guardrail.
    let depth: u32 = std::env::var("FLAMEL_AGENT_DEPTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if depth >= MAX_DEPTH {
        return Err(format!(
            "sub-agent depth limit reached (current depth = {depth}, max = {MAX_DEPTH}). Refusing to spawn — the parent agent should complete this work directly instead of delegating further."
        ));
    }

    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .map(|id| id.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let persona_file_path = match materialize_persona_markdown(&run_id, subagent_type, &cwd) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("[flamel_mcp] failed to materialize persona markdown: {error}");
            None
        }
    };
    let persona_manifest_path = persona_file_path
        .as_deref()
        .map(|path| persona_manifest_path_for(path, subagent_type));
    let persona_lifecycle = persona_lifecycle_manifest(
        subagent_type,
        runtime_agent_markdown(subagent_type),
        persona_file_path.as_deref(),
    );
    let persona_lifecycle_json = serde_json::to_string_pretty(&persona_lifecycle)
        .unwrap_or_else(|_| persona_lifecycle.to_string());
    let persona_file_instruction = match (&persona_file_path, &persona_manifest_path) {
        (Some(path), Some(manifest_path)) => format!(
            "\n\n--- SUB-AGENT PERSONA FILE ({subagent_type}) ---\nMandatory first actions: read the full persona markdown with `file_read(action=\"read\", file=\"{path}\")`, then read the lifecycle manifest with `file_read(action=\"read\", file=\"{manifest_path}\")`, before doing task work. Treat the `.md` file as the authoritative persona, including any frontmatter context, and treat the manifest as the version/evaluation/promotion record for this run. The same directory contains `base-config.yml` when that bundled file is available."
        ),
        _ => String::new(),
    };
    let persona_prefix = resolve_persona(subagent_type);
    let child_system_prompt = format!(
        "{parent_system_prompt}{persona_file_instruction}\n\n--- SUB-AGENT PERSONA LIFECYCLE ({subagent_type}) ---\n{persona_lifecycle_json}\n\n--- SUB-AGENT PERSONA ({subagent_type}) ---\n{persona_prefix}\n\nYou are a sub-agent spawned at depth {next_depth}. Your task ID is \"{description}\". Focus on completing the task below. Unless your persona explicitly says read-only or planning-only, do actual implementation work through Flamel MCP tools when required: edit files with file_edit, run checks with shell or code_quality, use browser automation, and use the typed git tool only when explicitly in scope. Explore remains read-only and Plan remains planning-only. When done, emit a concise final message with the deliverable and stop.",
        next_depth = depth + 1,
    );
    let child_mcp_servers_json = prepare_child_mcp_servers_config(
        &mcp_servers_json,
        &child_system_prompt,
        depth + 1,
        &run_id,
    )?;

    let child_input = prompt.to_string();

    // Helper: fire-and-forget POST to the nested-event route. Failures are logged
    // to stderr (captured by the MCP host) but never fail the run.
    let forward_event = |event: &Value| -> Result<(), String> {
        if !can_forward {
            return Ok(());
        }
        let body = json!({
            "sessionId": session_id,
            "runId": run_id,
            "parentRunId": parent_run_id,
            "event": event,
        });
        call_api(
            "POST",
            "/api/internal/nested-event",
            Some(&body),
            &flamel_url,
            &flamel_token,
        )
        .map(|_| ())
    };

    // Synthetic start event — tells the frontend a new run is beginning and carries
    // metadata so the UI can render the pending header even before the child emits
    // its first event.
    forward_event(&json!({
        "type": "nested_start",
        "description": description,
        "subagent_type": subagent_type,
        "depth": depth + 1,
        "idle_timeout_secs": agent_idle_timeout_secs,
        "persona": persona_lifecycle.clone(),
        "persona_file_path": persona_file_path,
        "persona_manifest_path": persona_manifest_path,
    }))?;

    // Build CLI args based on provider
    let mut cmd = flamel::mcp_parent_guard::command(&cli_bin);
    flamel::mcp_parent_guard::configure_child(&mut cmd);
    cmd.current_dir(&cwd);

    if is_codex {
        let reasoning_effort = codex_reasoning_effort();
        // Codex: system prompt injected via config.toml `instructions` field,
        // user prompt passed as the CLI arg. This keeps them cleanly separated.
        cmd.arg("exec")
            .arg("--json")
            .arg("--dangerously-bypass-approvals-and-sandbox")
            .arg("--skip-git-repo-check")
            .arg("-c")
            .arg(format!("model_reasoning_effort={reasoning_effort}"))
            .arg("-c")
            .arg("features.image_generation=true");
        configure_subagent_compaction_guard(&mut cmd, &provider);
        if let Some(m) = model_override {
            cmd.arg("-m").arg(m);
        }
        cmd.arg("-");
    } else {
        // Claude: stream-json via stdin, all flags inline
        cmd.arg("--print")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages")
            .arg("--append-system-prompt")
            .arg(&child_system_prompt)
            .arg("--mcp-config")
            .arg(&child_mcp_servers_json)
            .arg("--disallowedTools")
            .arg(&disallowed_tools)
            .arg("--permission-mode")
            .arg("bypassPermissions");
        configure_subagent_compaction_guard(&mut cmd, &provider);
        if let Some(m) = model_override {
            cmd.arg("--model").arg(m);
        }
    }

    // Both providers receive the task over stdin. Codex uses `-` as the prompt placeholder;
    // Claude reads the stream-json prompt body from the same pipe.
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    // Bump depth for the child so nested Agent calls see depth+1.
    cmd.env("FLAMEL_AGENT_DEPTH", (depth + 1).to_string());
    cmd.env("FLAMEL_SYSTEM_PROMPT", &child_system_prompt);
    cmd.env("FLAMEL_MCP_SERVERS", &child_mcp_servers_json);

    // Codex subagents need CODEX_HOME with auth.json + config.toml (MCP servers).
    let codex_subagent_home = if is_codex {
        Some(codex_subagent_home_for_run(&run_id, &cwd))
    } else {
        None
    };
    if let Some(ref subagent_home) = codex_subagent_home {
        if let Err(e) = std::fs::create_dir_all(&subagent_home) {
            eprintln!("[flamel_mcp] failed to create codex subagent home: {e}");
        }
        // Inherit native Codex auth, config, skills, and plugins before merging
        // the child-specific MCP servers and developer instructions.
        let parent_home = std::env::var("FLAMEL_CODEX_HOME")
            .or_else(|_| std::env::var("CODEX_HOME"))
            .ok()
            .filter(|s| !s.is_empty());
        if let Some(ref ph) = parent_home {
            inherit_codex_runtime_state(ph, subagent_home);
            if !std::path::Path::new(ph).join("auth.json").exists() {
                eprintln!(
                    "[flamel_mcp] codex auth.json not found at {}",
                    std::path::Path::new(ph).join("auth.json").display()
                );
            }
        } else {
            eprintln!(
                "[flamel_mcp] no FLAMEL_CODEX_HOME or CODEX_HOME — codex sub-agent will lack credentials"
            );
        }
        // Write config.toml with MCP servers + system prompt as instructions.
        write_codex_subagent_config(
            &child_mcp_servers_json,
            &subagent_home,
            Some(&child_system_prompt),
        );
        cmd.env("CODEX_HOME", &subagent_home);
    }

    let mut child = cmd.spawn().map_err(|e| {
        let _ =
            forward_event(&json!({"type": "nested_error", "error": format!("spawn failed: {e}")}));
        format!("failed to spawn sub-agent: {e}")
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(child_input.as_bytes());
        // Explicit drop closes stdin, signaling EOF to the child CLI.
    }

    let stdout = child
        .stdout
        .take()
        .ok_or("failed to capture sub-agent stdout")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("failed to capture sub-agent stderr")?;

    // Message types: (kind, payload). kind = 0 stdout line, 1 stderr line, 2 stdout done.
    let (tx, rx) = std::sync::mpsc::channel::<(u8, String)>();

    // Stdout reader — one line per message so we can parse events incrementally.
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        let _ = tx.send((0, l));
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send((2, String::new()));
        });
    }
    // Stderr reader — line-buffered so stderr activity keeps the idle watchdog alive.
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        let _ = tx.send((1, l));
                    }
                    Err(_) => break,
                }
            }
        });
    }

    let idle_timeout = std::time::Duration::from_secs(agent_idle_timeout_secs);
    let monitor_interval = std::time::Duration::from_secs(AGENT_MONITOR_INTERVAL_SECS);
    let start_time = std::time::Instant::now();
    let mut last_activity = start_time;
    let mut stderr_buf = String::new();
    let mut final_text = String::new();
    let mut last_assistant_text = String::new();
    let mut stdout_done = false;
    let mut status = None;
    let mut provider_failure = None;
    let mut provider_completed = false;

    loop {
        let idle_remaining = idle_timeout.saturating_sub(last_activity.elapsed());
        if idle_remaining.is_zero() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = forward_event(&json!({
                "type": "nested_error",
                "error": format!("idle timeout after {agent_idle_timeout_secs} seconds"),
            }));
            return Err(format!(
                "sub-agent idle timeout after {agent_idle_timeout_secs} seconds (task: {description})"
            ));
        }

        let wait_for = idle_remaining.min(monitor_interval);
        match rx.recv_timeout(wait_for) {
            Ok((0, line)) => {
                last_activity = std::time::Instant::now();
                if line.trim().is_empty() {
                    continue;
                }
                // Try to parse as JSON; if not JSON, skip forwarding but don't fail.
                match serde_json::from_str::<Value>(&line) {
                    Ok(event) => {
                        if let Some(outcome) = provider_terminal_outcome(&event, is_codex) {
                            match outcome {
                                Ok(()) => provider_completed = true,
                                Err(error) => provider_failure = Some(error),
                            }
                        }
                        // Track final assistant text to return as the tool_result body.
                        if let Some(ev_type) = event.get("type").and_then(|v| v.as_str()) {
                            if is_codex {
                                // Codex exec --json events use dot-separated names.
                                match ev_type {
                                    "message_delta" | "agent_message.delta" => {
                                        if let Some(t) = event.get("delta").and_then(|v| v.as_str())
                                        {
                                            last_assistant_text.push_str(t);
                                        }
                                    }
                                    "item.completed" => {
                                        if let Some(item) = event.get("item") {
                                            let item_type = item
                                                .get("type")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            match item_type {
                                                "agent_message" => {
                                                    if let Some(t) =
                                                        item.get("text").and_then(|v| v.as_str())
                                                    {
                                                        if !t.is_empty() {
                                                            last_assistant_text = t.to_string();
                                                        }
                                                    }
                                                }
                                                "tool_call" | "command_execution"
                                                | "file_change" => {}
                                                _ => {}
                                            }
                                        }
                                    }
                                    "item.started" => {}
                                    "reasoning_delta" | "reasoning_text.delta" => {}
                                    "command_execution.output_delta" => {}
                                    "turn.completed" | "turn.failed" | "error" => {}
                                    _ => {}
                                }
                            } else {
                                // Claude: "result" event has the final text; fall back to
                                // the last assistant message content.
                                if ev_type == "result" {
                                    if let Some(r) = event.get("result").and_then(|v| v.as_str()) {
                                        final_text = r.to_string();
                                    }
                                } else if ev_type == "assistant" {
                                    if let Some(content) = event
                                        .get("message")
                                        .and_then(|m| m.get("content"))
                                        .and_then(|c| c.as_array())
                                    {
                                        let mut piece = String::new();
                                        for b in content {
                                            if b.get("type").and_then(|v| v.as_str())
                                                == Some("text")
                                            {
                                                if let Some(t) =
                                                    b.get("text").and_then(|v| v.as_str())
                                                {
                                                    piece.push_str(t);
                                                }
                                            }
                                        }
                                        if !piece.is_empty() {
                                            last_assistant_text = piece;
                                        }
                                    }
                                }
                            }
                        }
                        if let Err(error) = forward_event(&event) {
                            eprintln!("[flamel_mcp] agent event save failed: {error}");
                        }
                    }
                    Err(_) => {
                        // Not JSON — forward as a raw text event so nothing is lost in the UI.
                        let _ = forward_event(&json!({"type": "nested_raw", "line": line}));
                    }
                }
            }
            Ok((1, s)) => {
                last_activity = std::time::Instant::now();
                stderr_buf.push_str(&s);
                stderr_buf.push('\n');
            }
            Ok((2, _)) => {
                last_activity = std::time::Instant::now();
                stdout_done = true;
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if last_activity.elapsed() >= idle_timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = forward_event(&json!({
                        "type": "nested_error",
                        "error": format!("idle timeout after {agent_idle_timeout_secs} seconds"),
                    }));
                    return Err(format!(
                        "sub-agent idle timeout after {agent_idle_timeout_secs} seconds"
                    ));
                }
                if stdout_done {
                    if let Some(s) = child
                        .try_wait()
                        .map_err(|e| format!("failed to poll sub-agent: {e}"))?
                    {
                        status = Some(s);
                        break;
                    }
                }
                let _ = forward_event(&json!({
                    "type": "nested_progress",
                    "elapsed_ms": start_time.elapsed().as_millis() as u64,
                    "idle_ms": last_activity.elapsed().as_millis() as u64,
                    "idle_timeout_secs": agent_idle_timeout_secs,
                }));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(s) = child
                    .try_wait()
                    .map_err(|e| format!("failed to poll sub-agent: {e}"))?
                {
                    status = Some(s);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }

        if stdout_done {
            if let Some(s) = child
                .try_wait()
                .map_err(|e| format!("failed to poll sub-agent: {e}"))?
            {
                status = Some(s);
                break;
            }
        }
    }

    let status = status.ok_or_else(|| "sub-agent exited without a status".to_string())?;

    while let Ok((kind, s)) = rx.try_recv() {
        if kind == 1 {
            stderr_buf.push_str(&s);
            stderr_buf.push('\n');
        }
    }
    if !status.success() {
        if let Some(ref home) = codex_subagent_home {
            let _ = std::fs::remove_dir_all(home);
        }
        let _ = forward_event(&json!({
            "type": "nested_error",
            "error": format!("exit {}", status.code().unwrap_or(-1)),
            "stderr": stderr_buf.chars().take(2000).collect::<String>(),
        }));
        return Err(format!(
            "sub-agent exited with status {} — stderr:\n{}",
            status.code().unwrap_or(-1),
            stderr_buf.chars().take(2000).collect::<String>(),
        ));
    }

    if let Some(error) = provider_failure.or_else(|| {
        (!provider_completed)
            .then(|| "Provider exited without confirming the child run completed".to_string())
    }) {
        let _ = forward_event(&json!({"type":"nested_error","error":error}));
        if let Some(ref home) = codex_subagent_home {
            let _ = std::fs::remove_dir_all(home);
        }
        return Err(error);
    }

    // Prefer the terminal `result` event text; fall back to the last assistant message.
    let body_text = if !final_text.is_empty() {
        final_text
    } else {
        last_assistant_text
    };
    let trimmed = body_text.trim();

    // Signal run completion to the frontend (unblocks any "running…" spinner it
    // was showing on the AgentRow based on the nested_start event).
    forward_event(&json!({
        "type": "nested_done",
        "ok": true,
        "output": cap_output(trimmed, OUTPUT_CAP, TRUNCATE_MARKER),
    }))?;

    // Clean up temp CODEX_HOME if we created one.
    if let Some(ref home) = codex_subagent_home {
        let _ = std::fs::remove_dir_all(home);
    }

    // Prepend `AGENT_RUN:{run_id}` on the first line so the frontend can correlate
    // buffered nested events to this tool_result.
    let payload = cap_output(trimmed, OUTPUT_CAP, TRUNCATE_MARKER);
    Ok(format_agent_run_payload(&run_id, &payload))
}

/// Spawn multiple sub-agents in parallel, collect all results, detect file
/// conflicts, and return a structured combined output for cross-checking.
///
/// Each sub-agent runs as an independent `claude` subprocess (reusing the
/// same spawning logic as `run_subagent`). The batch handler:
/// 1. Emits a `batch_start` nested event with agent metadata
/// 2. Spawns N threads, each running `run_subagent` with synthetic args
/// 3. Waits for all to complete while emitting progress; active agents are not wall-clock capped
/// 4. Emits `batch_done` nested event
/// 5. Returns structured output: per-agent results + file conflicts + review prompt
///
/// Output format: `AGENT_BATCH:{batch_id}\nAGENT_RUNS:{run_id1},{run_id2},...\n{body}`
pub(super) fn run_agent_batch(args: &Value) -> Result<String, String> {
    let batch_description = args
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("(unnamed batch)")
        .to_string();
    let agents = args
        .get("agents")
        .and_then(|v| v.as_array())
        .ok_or("agents array required")?;
    let review_prompt = args
        .get("review_prompt")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let failure_strategy = args
        .get("failure_strategy")
        .and_then(|v| v.as_str())
        .unwrap_or("continue")
        .to_string();
    let idle_timeout_secs =
        normalized_agent_batch_idle_timeout_secs(args.get("timeout_secs").and_then(|v| v.as_u64()));

    if agents.is_empty() {
        return Err("agents array must not be empty".to_string());
    }
    if agents.len() > 10 {
        return Err("agents array exceeds maximum of 10".to_string());
    }

    let batch_id = uuid::Uuid::new_v4().to_string();

    // Env vars for event forwarding (same as run_subagent)
    let session_id = std::env::var("FLAMEL_SESSION_ID").unwrap_or_default();
    let flamel_url = std::env::var("FLAMEL_URL").unwrap_or_default();
    let flamel_token = std::env::var("FLAMEL_TOKEN").unwrap_or_default();
    let can_forward = !session_id.is_empty() && !flamel_url.is_empty() && !flamel_token.is_empty();
    let parent_run_id = std::env::var("FLAMEL_PARENT_AGENT_RUN_ID")
        .ok()
        .filter(|id| !id.is_empty());

    // Forward a batch-level event (uses a synthetic run_id = batch_id so the
    // frontend can distinguish batch events from individual agent events).
    let forward_batch_event = |event: &Value| -> Result<(), String> {
        if !can_forward {
            return Ok(());
        }
        let body = json!({
            "sessionId": session_id,
            "runId": batch_id,
            "parentRunId": parent_run_id,
            "event": event,
        });
        call_api(
            "POST",
            "/api/internal/nested-event",
            Some(&body),
            &flamel_url,
            &flamel_token,
        )
        .map(|_| ())
    };

    // Build stable child run ids before spawning so status/history can be
    // correlated even if the parent AgentBatch tool call times out first.
    let child_run_ids: Vec<String> = (0..agents.len())
        .map(|_| uuid::Uuid::new_v4().to_string())
        .collect();

    // Build agent metadata for the batch_start event
    let tracked_agents: Vec<AgentBatchTrackedAgent> = agents
        .iter()
        .enumerate()
        .map(|(i, a)| AgentBatchTrackedAgent {
            index: i,
            description: a
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("(unnamed)")
                .to_string(),
            subagent_type: a
                .get("subagent_type")
                .and_then(|v| v.as_str())
                .unwrap_or("general-purpose")
                .to_string(),
            status: "running".to_string(),
            run_id: child_run_ids.get(i).cloned().unwrap_or_default(),
            duration_ms: None,
            output: None,
            output_truncated: false,
            error: None,
        })
        .collect();
    let agent_meta: Vec<Value> = agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            json!({
                "index": i,
                "run_id": child_run_ids.get(i).map(|id| id.as_str()).unwrap_or(""),
                "description": a.get("description").and_then(|v| v.as_str()).unwrap_or("(unnamed)"),
                "subagent_type": a.get("subagent_type").and_then(|v| v.as_str()).unwrap_or("general-purpose"),
                "status": "running",
            })
        })
        .collect();

    forward_batch_event(&json!({
        "type": "batch_start",
        "batch_id": batch_id,
        "description": batch_description,
        "agent_count": agents.len(),
        "idle_timeout_secs": idle_timeout_secs,
        "agents": agent_meta,
    }))?; // Do not launch untrackable work when its durable receipt cannot be saved.

    register_agent_batch(
        &batch_id,
        batch_description.clone(),
        idle_timeout_secs,
        tracked_agents,
    );

    let agent_descriptions: Vec<String> = agents
        .iter()
        .map(|a| {
            a.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("(unnamed)")
                .to_string()
        })
        .collect();

    // Spawn all agents in parallel threads and collect results via a channel.
    // Active agents are allowed to run beyond the idle watchdog; each child only
    // fails when it stops producing output for `idle_timeout_secs`.
    let start_time = std::time::Instant::now();
    let fail_fast_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (result_tx, result_rx) = std::sync::mpsc::channel::<(usize, AgentBatchResult)>();

    for (idx, agent_spec) in agents.iter().enumerate() {
        let sub_args = json!({
            "description": agent_spec.get("description").and_then(|v| v.as_str()).unwrap_or("(unnamed)"),
            "prompt": agent_spec.get("prompt").and_then(|v| v.as_str()).unwrap_or(""),
            "subagent_type": agent_spec.get("subagent_type").and_then(|v| v.as_str()).unwrap_or("general-purpose"),
            "model": agent_spec.get("model").and_then(|v| v.as_str()),
            "timeout_secs": idle_timeout_secs,
            "run_id": child_run_ids.get(idx).map(|id| id.as_str()).unwrap_or(""),
            "parent_run_id": batch_id,
        });
        let ff = fail_fast_flag.clone();
        let sid = session_id.clone();
        let furl = flamel_url.clone();
        let ftok = flamel_token.clone();
        let bid = batch_id.clone();
        let parent_id = parent_run_id.clone();
        let can_fwd = can_forward;
        let desc = agent_spec
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("(unnamed)")
            .to_string();
        let fs = failure_strategy.clone();
        let tx = result_tx.clone();

        std::thread::spawn(move || {
            // Check fail-fast before starting
            let result = if ff.load(std::sync::atomic::Ordering::Relaxed) {
                (
                    idx,
                    AgentBatchResult {
                        description: desc,
                        status: "skipped".to_string(),
                        output: String::new(),
                        run_id: sub_args["run_id"].as_str().unwrap_or("").to_string(),
                        duration_ms: 0,
                        error: Some("skipped due to fail_fast — earlier agent failed".to_string()),
                    },
                )
            } else {
                let t0 = std::time::Instant::now();
                let result = run_subagent(&sub_args);
                let duration_ms = t0.elapsed().as_millis() as u64;

                let batch_result = match result {
                    Ok(output) => {
                        let run_id = if output.starts_with("AGENT_RUN:") {
                            let nl = output.find('\n').unwrap_or(output.len());
                            output["AGENT_RUN:".len()..nl].trim().to_string()
                        } else {
                            String::new()
                        };
                        let clean = if let Some(nl) = output.find('\n') {
                            output[nl + 1..].to_string()
                        } else {
                            String::new()
                        };
                        AgentBatchResult {
                            description: desc.clone(),
                            status: "success".to_string(),
                            output: clean,
                            run_id,
                            duration_ms,
                            error: None,
                        }
                    }
                    Err(e) => {
                        if fs == "fail_fast" {
                            ff.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        let status = if e.contains("idle timeout") {
                            "timeout"
                        } else {
                            "failed"
                        };
                        AgentBatchResult {
                            description: desc.clone(),
                            status: status.to_string(),
                            output: String::new(),
                            run_id: sub_args["run_id"].as_str().unwrap_or("").to_string(),
                            duration_ms,
                            error: Some(e),
                        }
                    }
                };

                (idx, batch_result)
            };

            if can_fwd {
                let batch_result = &result.1;
                let event = json!({
                    "type": "batch_agent_done",
                    "batch_id": bid,
                    "agent_index": idx,
                    "description": batch_result.description,
                    "status": batch_result.status,
                    "run_id": batch_result.run_id.clone(),
                    "child_run_id": batch_result.run_id,
                    "duration_ms": batch_result.duration_ms,
                    "output": cap_output(&batch_result.output,15*1024,"\n[Output truncated at 15KB]"),
                    "output_truncated": batch_result.output.len() > 15*1024,
                    "error": batch_result.error,
                });
                let body = json!({
                    "sessionId": sid,
                    "runId": bid,
                    "parentRunId": parent_id,
                    "event": event,
                });
                if let Err(error) = call_api(
                    "POST",
                    "/api/internal/nested-event",
                    Some(&body),
                    &furl,
                    &ftok,
                ) {
                    eprintln!("[flamel_mcp] child receipt save failed: {error}");
                }
            }

            let _ = tx.send(result);
        });
    }
    drop(result_tx);

    // Collect results without a wall-clock cap. The workers enforce their own
    // idle watchdog, and this loop emits progress while the batch keeps running.
    let monitor_interval = std::time::Duration::from_secs(AGENT_MONITOR_INTERVAL_SECS);
    let mut indexed_results: Vec<Option<AgentBatchResult>> =
        (0..agents.len()).map(|_| None).collect();
    let mut completed = 0usize;

    while completed < agents.len() {
        match result_rx.recv_timeout(monitor_interval) {
            Ok((idx, result)) => {
                if idx < indexed_results.len() && indexed_results[idx].is_none() {
                    update_agent_batch_agent(&batch_id, idx, &result);
                    indexed_results[idx] = Some(result);
                    completed += 1;
                    let _ = forward_batch_event(&json!({
                        "type": "batch_progress",
                        "batch_id": batch_id,
                        "completed": completed,
                        "total": agents.len(),
                        "running": agents.len().saturating_sub(completed),
                        "elapsed_ms": start_time.elapsed().as_millis() as u64,
                        "idle_timeout_secs": idle_timeout_secs,
                    }));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = forward_batch_event(&json!({
                    "type": "batch_progress",
                    "batch_id": batch_id,
                    "completed": completed,
                    "total": agents.len(),
                    "running": agents.len().saturating_sub(completed),
                    "elapsed_ms": start_time.elapsed().as_millis() as u64,
                    "idle_timeout_secs": idle_timeout_secs,
                }));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let elapsed_ms = start_time.elapsed().as_millis() as u64;
    let results: Vec<(usize, AgentBatchResult)> = indexed_results
        .into_iter()
        .enumerate()
        .map(|(idx, result)| {
            let result = result.unwrap_or_else(|| AgentBatchResult {
                description: agent_descriptions
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| "(incomplete)".to_string()),
                status: "timeout".to_string(),
                output: String::new(),
                run_id: child_run_ids.get(idx).cloned().unwrap_or_default(),
                duration_ms: elapsed_ms,
                error: Some(format!(
                    "batch ended before this agent reported a result; idle watchdog was {idle_timeout_secs}s"
                )),
            });
            (idx, result)
        })
        .collect();

    // Collect all run_ids for the header
    for (idx, result) in &results {
        update_agent_batch_agent(&batch_id, *idx, result);
        // Retry the final receipt from the collecting thread as well. This is
        // harmless on duplicate delivery and repairs an earlier transport loss.
        forward_batch_event(&json!({"type":"batch_agent_done","batch_id":batch_id,
            "agent_index":idx,"run_id":result.run_id,"child_run_id":result.run_id,"status":result.status,"duration_ms":result.duration_ms,
            "output":cap_output(&result.output,15*1024,"\n[Output truncated at 15KB]"),
            "output_truncated":result.output.len()>15*1024,"error":result.error}))?;
    }

    let run_ids: Vec<&str> = results.iter().map(|(_, r)| r.run_id.as_str()).collect();

    // Count outcomes
    let total = results.len();
    let succeeded = results
        .iter()
        .filter(|(_, r)| r.status == "success")
        .count();
    let failed = results.iter().filter(|(_, r)| r.status == "failed").count();
    let timed_out = results
        .iter()
        .filter(|(_, r)| r.status == "timeout")
        .count();
    let skipped = results
        .iter()
        .filter(|(_, r)| r.status == "skipped")
        .count();
    let total_duration = start_time.elapsed();
    let batch_status = if failed > 0 || timed_out > 0 || skipped > 0 {
        "completed_with_errors"
    } else {
        "success"
    };
    finish_agent_batch(&batch_id, batch_status);

    // Detect file conflicts: parse "Files changed:" lines from each agent's output
    // (agents typically list modified files; we look for overlaps)
    let mut files_by_agent: Vec<(usize, Vec<String>)> = Vec::new();
    for (idx, r) in &results {
        if r.status != "success" {
            continue;
        }
        let mut files = Vec::new();
        for line in r.output.lines() {
            let trimmed = line.trim();
            // Look for common patterns: "Edited X", "Created X", "Modified X", file paths ending in common extensions
            if trimmed.starts_with("Edited ")
                || trimmed.starts_with("Created ")
                || trimmed.starts_with("Modified ")
            {
                if let Some(path) = trimmed.split_whitespace().nth(1) {
                    files.push(path.to_string());
                }
            }
        }
        if !files.is_empty() {
            files_by_agent.push((*idx, files));
        }
    }

    let mut conflicts: Vec<String> = Vec::new();
    for i in 0..files_by_agent.len() {
        for j in (i + 1)..files_by_agent.len() {
            let (idx_a, files_a) = &files_by_agent[i];
            let (idx_b, files_b) = &files_by_agent[j];
            for fa in files_a {
                if files_b.contains(fa) {
                    let desc_a = &results[*idx_a].1.description;
                    let desc_b = &results[*idx_b].1.description;
                    conflicts.push(format!(
                        "⚠ {} modified by Agent {} ({}) AND Agent {} ({}) — review for conflicts",
                        fa,
                        idx_a + 1,
                        desc_a,
                        idx_b + 1,
                        desc_b
                    ));
                }
            }
        }
    }

    // Keep real child IDs/outcomes in the persisted tool result too. Reopening
    // chat history must not require the original live nested-event buffer.
    let output = json!({
        "batch_id":batch_id,"description":batch_description,"status":batch_status,
        "total":total,"completed":total,"succeeded":succeeded,"failed":failed,
        "timed_out":timed_out,"skipped":skipped,"running":0,
        "elapsed_ms":total_duration.as_millis() as u64,
        "message":format!("{succeeded} of {total} agents succeeded"),
        "agents":results.iter().map(|(idx,r)| json!({
            "index":idx,"run_id":r.run_id,"description":r.description,"status":r.status,
            "subagent_type":agents[*idx].get("subagent_type"),"duration_ms":r.duration_ms,
            "output":cap_output(&r.output,15*1024,"\n[Output truncated at 15KB]"),
            "output_truncated":r.output.len()>15*1024,"error":r.error,
        })).collect::<Vec<_>>(),"possible_file_conflicts":conflicts,"review_prompt":review_prompt,
    });

    // Emit batch_done event
    forward_batch_event(&json!({
        "type": "batch_done",
        "batch_id": batch_id,
        "succeeded": succeeded,
        "failed": failed,
        "timed_out": timed_out,
        "skipped": skipped,
        "total_duration_ms": total_duration.as_millis() as u64,
    }))?;

    // Format output with AGENT_BATCH header so the frontend can identify this as a batch result
    let header = format!(
        "AGENT_BATCH:{}\nAGENT_RUNS:{}\n",
        batch_id,
        run_ids.join(",")
    );
    Ok(format!("{header}{output}"))
}

pub(super) struct AgentBatchResult {
    pub(super) description: String,
    pub(super) status: String, // "success", "failed", "timeout", "skipped"
    pub(super) output: String,
    pub(super) run_id: String,
    pub(super) duration_ms: u64,
    pub(super) error: Option<String>,
}

#[cfg(test)]
mod tracking_tests {
    use super::*;

    #[test]
    fn status_client_fixture_child() {
        if std::env::var("FLAMEL_TRACKING_TEST_CHILD").as_deref() != Ok("1") {
            return;
        }
        let status = durable_batch_status(&json!({"batch_id":"recorded-child"}))
            .unwrap()
            .unwrap();
        let value: Value = serde_json::from_str(&status).unwrap();
        assert_eq!(value["agents"][0]["output"], "Recovered from the server");
        assert_eq!(value["status"], "success");
    }

    #[test]
    fn fresh_process_recovers_status_without_a_process_local_registry() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            let (mut connection, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(10))
                    }
                    Err(error) => panic!("status fixture did not connect: {error}"),
                }
            };
            connection
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = connection.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                let raw = String::from_utf8_lossy(&request);
                if let Some((headers, body)) = raw.split_once("\r\n\r\n") {
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|n| n.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if body.len() >= length {
                        break;
                    }
                }
            }
            let raw = String::from_utf8(request).unwrap();
            assert!(raw.starts_with("POST /api/internal/agent-batch-status "));
            assert!(
                raw.to_ascii_lowercase()
                    .contains("x-flamel-user-id: fixture-owner")
            );
            let body: Value = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body["sessionId"], "fixture-session");
            assert_eq!(body["parentRunId"], "fixture-parent");
            let response = json!({"status":"success","agents":[{"run_id":"recorded-child","status":"success","output":"Recovered from the server"}]}).to_string();
            write!(connection,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
        });
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "mcp_agents::tracking_tests::status_client_fixture_child",
                "--nocapture",
            ])
            .env("FLAMEL_TRACKING_TEST_CHILD", "1")
            .env("FLAMEL_SESSION_ID", "fixture-session")
            .env("FLAMEL_USER_ID", "fixture-owner")
            .env("FLAMEL_PARENT_AGENT_RUN_ID", "fixture-parent")
            .env("FLAMEL_URL", format!("http://{address}"))
            .env("FLAMEL_TOKEN", "fixture-token")
            .output()
            .unwrap();
        server.join().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[test]
    fn provider_errors_are_not_successful_child_results() {
        for event in [
            json!({"type":"turn.failed","error":{"message":"rate limit"}}),
            json!({"type":"error","message":"CLI failed"}),
        ] {
            assert!(provider_terminal_outcome(&event, true).unwrap().is_err());
        }
        for event in [
            json!({"type":"result","is_error":true,"result":"Failed"}),
            json!({"type":"result","subtype":"error_during_execution"}),
        ] {
            assert!(provider_terminal_outcome(&event, false).unwrap().is_err());
        }
        assert_eq!(
            provider_terminal_outcome(&json!({"type":"turn.completed"}), true),
            Some(Ok(()))
        );
        assert_eq!(
            provider_terminal_outcome(&json!({"type":"result","subtype":"success"}), false),
            Some(Ok(()))
        );
        assert_eq!(
            provider_terminal_outcome(
                &json!({"type":"item.completed","item":{"type":"command_execution"}}),
                true
            ),
            None
        );
    }
}
