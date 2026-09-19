//! Flamel Browser MCP.
//!
//! The public surface stays deliberately small while the Flamel-owned service
//! manages Chrome lifecycle, accessibility references, browser actions, and
//! bounded debug evidence through the pinned Chrome DevTools MCP.

#![recursion_limit = "512"]

use base64::Engine as _;
use flamel::mcp_args::normalize_tool_arguments;
use serde_json::{Map, Value, json};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::Mutex;

const UPSTREAM_PROTOCOL: &str = "2025-11-25";
const MAX_ACTIONS: usize = 25;
const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const CAPTURE_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;
const MAX_SESSION_CAPTURES: usize = 50;

static BROWSER: Mutex<Option<BrowserService>> = Mutex::new(None);

struct ChromeDevtoolsService {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    profile: Option<String>,
}

impl Drop for ChromeDevtoolsService {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl ChromeDevtoolsService {
    fn start(profile: Option<&str>) -> Result<Self, String> {
        let mut command = Self::configured_command();
        command.arg("--headless=true");

        if let Some(executable) = chrome_executable() {
            command.arg(format!("--executablePath={}", executable.display()));
        }

        let profile = profile.map(validate_profile_name).transpose()?;
        if let Some(name) = profile.as_deref() {
            let directory = profile_directory(name)?;
            command.arg(format!("--userDataDir={}", directory.display()));
        } else {
            command.arg("--isolated=true");
        }
        Self::spawn(command, profile)
    }

    fn configured_command() -> std::process::Command {
        let mut command = chrome_devtools_command();
        command
            .arg("--usageStatistics=false")
            .arg("--performanceCrux=false")
            .arg("--redactNetworkHeaders=true")
            .arg("--experimentalVision=true")
            .arg("--categoryExtensions=false")
            .arg("--screenshotFormat=webp")
            .arg("--screenshotQuality=72")
            .arg("--screenshotMaxWidth=1600")
            .arg("--screenshotMaxHeight=1200");
        command
    }

    fn spawn(mut command: std::process::Command, profile: Option<String>) -> Result<Self, String> {
        let mut child = command
            .env("CI", "1")
            .env("CHROME_DEVTOOLS_MCP_NO_USAGE_STATISTICS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                format!(
                    "Chrome DevTools MCP is unavailable: {error}. Install chrome-devtools-mcp@1.7.0 and Chrome/Chromium."
                )
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Chrome DevTools MCP stdin is unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Chrome DevTools MCP stdout is unavailable".to_string())?;
        let mut service = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            profile,
        };
        service.request(
            "initialize",
            json!({
                "protocolVersion": UPSTREAM_PROTOCOL,
                "capabilities": {},
                "clientInfo": {"name": "flamel-browser", "version": flamel::BUILD_VERSION},
            }),
        )?;
        service.notify("notifications/initialized", json!({}))?;
        Ok(service)
    }

    fn is_alive(&mut self) -> Result<bool, String> {
        self.child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(|error| error.to_string())
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if !self.is_alive()? {
            return Err("Chrome DevTools MCP exited unexpectedly".to_string());
        }
        let id = self.next_id;
        self.next_id += 1;
        write_json_line(
            &mut self.stdin,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )?;

        let mut line = String::new();
        loop {
            line.clear();
            if self
                .stdout
                .read_line(&mut line)
                .map_err(|error| format!("Chrome DevTools MCP read failed: {error}"))?
                == 0
            {
                return Err("Chrome DevTools MCP closed its transport".to_string());
            }
            let response: Value = match serde_json::from_str(line.trim()) {
                Ok(response) => response,
                Err(_) => continue,
            };
            if response.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = response.get("error") {
                return Err(format_upstream_error(error));
            }
            return response
                .get("result")
                .cloned()
                .ok_or_else(|| "Chrome DevTools MCP returned no result".to_string());
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        write_json_line(
            &mut self.stdin,
            &json!({"jsonrpc": "2.0", "method": method, "params": params}),
        )
    }

    fn call(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
            .and_then(validate_upstream_result)
    }
}

struct BrowserService {
    service: ChromeDevtoolsService,
    profile: Option<String>,
}

impl BrowserService {
    fn start(profile: Option<&str>) -> Result<Self, String> {
        let service = ChromeDevtoolsService::start(profile)?;
        let profile = service.profile.clone();
        Ok(Self { service, profile })
    }

    fn try_attach(_profile: Option<&str>) -> Result<Option<Self>, String> {
        Ok(None)
    }

    fn cleanup_persisted(_profile: Option<&str>) -> Result<(), String> {
        Ok(())
    }

    fn is_alive(&mut self) -> Result<bool, String> {
        self.service.is_alive()
    }

    fn shutdown(&mut self) -> Result<(), String> {
        let _ = self.service.child.kill();
        let _ = self.service.child.wait();
        Ok(())
    }

    fn engine(&self) -> &'static str {
        "Flamel Browser (Chrome DevTools)"
    }

    fn call(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let name = if name == "wait_for_function" {
            "evaluate_script"
        } else {
            name
        };
        self.service.call(name, arguments)
    }
}

fn chrome_devtools_command() -> std::process::Command {
    if let Ok(binary) = std::env::var("FLAMEL_CHROME_DEVTOOLS_MCP_PATH") {
        return flamel::mcp_parent_guard::command(binary);
    }
    if executable_on_path("chrome-devtools-mcp") {
        return flamel::mcp_parent_guard::command("chrome-devtools-mcp");
    }

    #[cfg(windows)]
    let command = {
        let mut command = flamel::mcp_parent_guard::command("npx.cmd");
        command.args(["--yes", "chrome-devtools-mcp@1.7.0"]);
        command
    };
    #[cfg(not(windows))]
    let command = {
        let mut command = flamel::mcp_parent_guard::command("npx");
        command.args(["--yes", "chrome-devtools-mcp@1.7.0"]);
        command
    };
    command
}

fn executable_on_path(name: &str) -> bool {
    let path = Path::new(name);
    if path.components().count() > 1 {
        return path.is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|directory| {
        if directory.join(name).is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            ["exe", "cmd", "bat"]
                .iter()
                .any(|extension| directory.join(format!("{name}.{extension}")).is_file())
        }
        #[cfg(not(windows))]
        false
    })
}

fn main() {
    flamel::mcp_parent_guard::install();
    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some(response) = handle_request(&request) {
            let mut output = stdout.lock();
            let _ = serde_json::to_writer(&mut output, &response);
            let _ = output.write_all(b"\n");
            let _ = output.flush();
        }
    }
}

fn handle_request(request: &Value) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method")?.as_str()?;
    if method == "initialize" {
        let protocol = flamel::mcp_gateway::negotiated_protocol(
            request
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str),
        );
        return Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": protocol,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "browser_mcp", "version": flamel::BUILD_VERSION},
            }
        }));
    }
    if method.starts_with("notifications/") {
        return None;
    }
    if method == "tools/list" {
        return Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"tools": tools_list()},
        }));
    }
    if method == "tools/call" {
        let params = request.get("params")?;
        let name = params.get("name")?.as_str()?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let normalized = match normalize_tool_arguments(name, arguments, &tools_list()) {
            Ok(normalized) => normalized.arguments,
            Err(error) => return Some(tool_response(id, Err(error))),
        };
        return Some(tool_response(id, handle_tool_call(name, &normalized)));
    }
    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32601, "message": "Method not found"},
    }))
}

fn tool_response(id: Value, result: Result<Value, String>) -> Value {
    let result = match result {
        Ok(result) => result,
        Err(error) => json!({
            "content": [{"type": "text", "text": format!("Error: {error}")}],
            "isError": true,
        }),
    };
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn handle_tool_call(name: &str, arguments: &Value) -> Result<Value, String> {
    match name {
        "browser_session" => browser_session(arguments),
        "browser_run" => browser_run(arguments),
        "browser_inspect" => browser_inspect(arguments),
        _ => Err(format!("Unknown browser tool: {name}")),
    }
}

fn browser_session(arguments: &Value) -> Result<Value, String> {
    let action = string_arg(arguments, "action").unwrap_or("status");
    match action {
        "open" => {
            let profile = string_arg(arguments, "profile");
            ensure_browser(profile)?;
            let engine = with_browser(|browser| Ok(browser.engine()))?;
            let mut content = vec![json!({
                "type": "text",
                "text": serde_json::to_string(&json!({
                    "status": "open",
                    "profile": profile,
                    "persistent": true,
                    "engine": engine,
                })).unwrap(),
            })];
            if let Some(url) = string_arg(arguments, "url") {
                append_content(
                    &mut content,
                    with_browser(|browser| {
                        browser.call("navigate_page", json!({"type": "url", "url": url}))
                    })?,
                    "navigation",
                );
            }
            Ok(json!({"content": content}))
        }
        "status" => {
            let mut guard = BROWSER
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if guard.is_none() {
                *guard = BrowserService::try_attach(None)?;
            }
            let Some(browser) = guard.as_mut() else {
                return Ok(text_result(json!({"status": "closed", "persistent": true})));
            };
            let pages = browser.call("list_pages", json!({}))?;
            let profile = browser.profile.clone();
            let engine = browser.engine();
            Ok(combine_results(
                json!({"status": "open", "profile": profile, "persistent": true, "engine": engine}),
                vec![("pages", pages)],
            ))
        }
        "tabs" => with_browser(|browser| browser.call("list_pages", json!({}))),
        "new_tab" => {
            let url = required_string(arguments, "url")?;
            with_browser(|browser| browser.call("new_page", json!({"url": url})))
        }
        "select_tab" => {
            let page_id = required_u64(arguments, "page_id")?;
            with_browser(|browser| {
                browser.call(
                    "select_page",
                    json!({"pageId": page_id, "bringToFront": true}),
                )
            })
        }
        "close_tab" => {
            let page_id = required_u64(arguments, "page_id")?;
            with_browser(|browser| browser.call("close_page", json!({"pageId": page_id})))
        }
        "close" => {
            close_browser()?;
            Ok(text_result(json!({"status": "closed"})))
        }
        "reset" => {
            let profile = string_arg(arguments, "profile").map(str::to_string);
            close_browser()?;
            if arguments
                .get("clear_profile")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                if let Some(profile) = profile.as_deref() {
                    let path = profile_directory(profile)?;
                    match std::fs::remove_dir_all(&path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(format!(
                                "failed to clear browser profile {}: {error}",
                                path.display()
                            ));
                        }
                    }
                }
            }
            ensure_browser(profile.as_deref())?;
            Ok(text_result(json!({"status": "reset", "profile": profile})))
        }
        _ => Err(format!("Unsupported browser session action: {action}")),
    }
}

fn browser_run(arguments: &Value) -> Result<Value, String> {
    ensure_browser(None)?;
    let actions = arguments
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| "actions must be a non-empty array".to_string())?;
    if actions.is_empty() {
        return Err("actions must be a non-empty array".to_string());
    }
    if actions.len() > MAX_ACTIONS {
        return Err(format!("browser_run accepts at most {MAX_ACTIONS} actions"));
    }
    let stop_on_error = arguments
        .get("stop_on_error")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let observe = arguments
        .get("observe")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut completed = Vec::new();
    let mut rich_content = Vec::new();

    for (index, action) in actions.iter().enumerate() {
        let action_name = required_string(action, "action")?;
        match dispatch_action(action_name, action) {
            Ok(result) => {
                let mut completion = json!({
                    "index": index,
                    "action": action_name,
                    "status": "done",
                });
                if let Some(text) = result_text(&result).filter(|text| !text.trim().is_empty()) {
                    completion["result"] = Value::String(truncate_text(&text, 800));
                }
                completed.push(completion);
                append_non_text_content(&mut rich_content, &result);
            }
            Err(error) => {
                completed.push(json!({
                    "index": index,
                    "action": action_name,
                    "status": "error",
                    "error": error,
                }));
                if stop_on_error {
                    let mut content = vec![json!({
                        "type": "text",
                        "text": serde_json::to_string(&json!({
                            "status": "error",
                            "actions": completed,
                        })).unwrap(),
                    })];
                    content.extend(rich_content);
                    return Ok(json!({"content": content, "isError": true}));
                }
            }
        }
    }

    let observation = if observe {
        Some(with_browser(|browser| {
            browser.call("take_snapshot", json!({}))
        })?)
    } else {
        None
    };
    let mut content = vec![json!({
        "type": "text",
        "text": serde_json::to_string(&json!({
            "status": "complete",
            "actions": completed,
            "observation_included": observation.is_some(),
        })).unwrap(),
    })];
    content.extend(rich_content);
    if let Some(observation) = observation {
        append_content(&mut content, observation, "current page snapshot");
    }
    Ok(json!({"content": content}))
}

fn dispatch_action(action: &str, arguments: &Value) -> Result<Value, String> {
    let call = |tool: &str, args: Value| with_browser(|browser| browser.call(tool, args));
    match action {
        "navigate" => call(
            "navigate_page",
            json!({
                "type": "url",
                "url": required_string(arguments, "url")?,
                "timeout": timeout_ms(arguments),
            }),
        ),
        "back" | "forward" | "reload" => call(
            "navigate_page",
            json!({"type": action, "timeout": timeout_ms(arguments)}),
        ),
        "snapshot" => call(
            "take_snapshot",
            json!({"verbose": bool_arg(arguments, "verbose")}),
        ),
        "click" => call(
            "click",
            json!({
                "uid": required_string(arguments, "uid")?,
                "dblClick": bool_arg(arguments, "double_click"),
                "includeSnapshot": false,
            }),
        ),
        "click_at" => call(
            "click_at",
            json!({
                "x": required_u64(arguments, "x")?,
                "y": required_u64(arguments, "y")?,
                "dblClick": bool_arg(arguments, "double_click"),
            }),
        ),
        "fill" => call(
            "fill",
            json!({
                "uid": required_string(arguments, "uid")?,
                "value": required_string(arguments, "value")?,
                "includeSnapshot": false,
            }),
        ),
        "fill_form" => call(
            "fill_form",
            json!({
                "elements": arguments.get("elements").cloned().ok_or_else(|| "elements is required".to_string())?,
                "includeSnapshot": false,
            }),
        ),
        "type_text" => call(
            "type_text",
            json!({
                "text": required_string(arguments, "text")?,
                "submitKey": string_arg(arguments, "submit_key"),
            }),
        ),
        "press_key" => call(
            "press_key",
            json!({"key": required_string(arguments, "key")?}),
        ),
        "hover" => call("hover", json!({"uid": required_string(arguments, "uid")?})),
        "drag" => call(
            "drag",
            json!({
                "from_uid": required_string(arguments, "from_uid")?,
                "to_uid": required_string(arguments, "to_uid")?,
            }),
        ),
        "upload_file" => call(
            "upload_file",
            json!({
                "uid": required_string(arguments, "uid")?,
                "filePath": validated_upload_path(required_string(arguments, "file_path")?)?,
            }),
        ),
        "evaluate" => call(
            "evaluate_script",
            json!({"function": required_string(arguments, "function")?}),
        ),
        "wait_for" => call(
            "wait_for_function",
            json!({
                "function": wait_function(arguments)?,
            }),
        ),
        "resize" => call(
            "resize_page",
            json!({
                "width": required_u64(arguments, "width")?,
                "height": required_u64(arguments, "height")?,
            }),
        ),
        "emulate" => call(
            "emulate",
            copy_keys(
                arguments,
                &[
                    ("network", "networkConditions"),
                    ("cpu", "cpuThrottlingRate"),
                    ("geolocation", "geolocation"),
                    ("user_agent", "userAgent"),
                    ("color_scheme", "colorScheme"),
                    ("viewport", "viewport"),
                    ("headers", "extraHttpHeaders"),
                ],
            ),
        ),
        "handle_dialog" => call(
            "handle_dialog",
            json!({
                "action": required_string(arguments, "dialog_action")?,
                "promptText": string_arg(arguments, "prompt_text"),
            }),
        ),
        "trace_start" => call(
            "performance_start_trace",
            json!({"reload": false, "autoStop": false}),
        ),
        "trace_stop" => call("performance_stop_trace", json!({})),
        "download" => call(
            "download",
            json!({
                "uid": required_string(arguments, "uid")?,
                "timeoutMs": timeout_ms(arguments),
            }),
        ),
        "wait_for_download" => call(
            "wait_for_download",
            json!({"timeoutMs": timeout_ms(arguments)}),
        ),
        _ => Err(format!("Unsupported browser action: {action}")),
    }
}

fn browser_inspect(arguments: &Value) -> Result<Value, String> {
    ensure_browser(None)?;
    let requested = arguments
        .get("include")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["snapshot", "console", "network"]);
    let page_size = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 200);
    let mut results = Vec::new();
    for kind in requested {
        let mut result = match kind {
            "snapshot" => with_browser(|browser| browser.call("take_snapshot", json!({}))),
            "console" => with_browser(|browser| {
                browser.call(
                    "list_console_messages",
                    json!({"pageSize": page_size, "includePreservedMessages": true}),
                )
            }),
            "network" => with_browser(|browser| {
                browser.call(
                    "list_network_requests",
                    json!({"pageSize": page_size, "includePreservedRequests": true}),
                )
            }),
            "screenshot" => with_browser(|browser| {
                browser.call(
                    "take_screenshot",
                    json!({
                        "format": "webp",
                        "quality": 72,
                        "fullPage": bool_arg(arguments, "full_page"),
                    }),
                )
            }),
            "lighthouse" => with_browser(|browser| {
                browser.call(
                    "lighthouse_audit",
                    json!({
                        "mode": string_arg(arguments, "audit_mode").unwrap_or("navigation"),
                        "device": string_arg(arguments, "device").unwrap_or("desktop"),
                    }),
                )
            }),
            "performance" | "trace" => with_browser(|browser| {
                browser.call(
                    "performance_start_trace",
                    json!({"reload": true, "autoStop": true}),
                )
            }),
            "pdf" => with_browser(|browser| {
                browser.call("pdf", json!({"timeoutMs": timeout_ms(arguments)}))
            }),
            unknown => Err(format!("Unsupported inspection kind: {unknown}")),
        }?;
        attach_private_capture_artifacts(&mut result)?;
        results.push((kind, result));
    }
    Ok(combine_results(
        json!({"status": "complete", "inspections": results.iter().map(|(name, _)| *name).collect::<Vec<_>>() }),
        results,
    ))
}

fn ensure_browser(profile: Option<&str>) -> Result<(), String> {
    let mut guard = BROWSER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(browser) = guard.as_mut() {
        if browser.is_alive()? {
            if profile.is_none() || browser.profile.as_deref() == profile {
                return Ok(());
            }
            return Err(format!(
                "browser is already open with profile {}; close it before selecting another profile",
                browser.profile.as_deref().unwrap_or("isolated")
            ));
        }
        *guard = None;
    }
    *guard = Some(BrowserService::start(profile)?);
    Ok(())
}

fn with_browser<T>(
    operation: impl FnOnce(&mut BrowserService) -> Result<T, String>,
) -> Result<T, String> {
    ensure_browser(None)?;
    let mut guard = BROWSER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    operation(
        guard
            .as_mut()
            .ok_or_else(|| "browser failed to start".to_string())?,
    )
}

fn close_browser() -> Result<(), String> {
    let mut browser = {
        let mut guard = BROWSER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.take()
    };
    if browser.is_none() {
        browser = BrowserService::try_attach(None)?;
    }
    let Some(mut browser) = browser else {
        return BrowserService::cleanup_persisted(None);
    };
    browser.shutdown()
}

fn tools_list() -> Value {
    json!([
        {
            "name": "browser_session",
            "description": "Open and manage one persistent Flamel browser for this chat. Use an opt-in named profile only when the user explicitly asks to save or reuse a browser login; otherwise use an isolated session. Actions: open, status, tabs, new_tab, select_tab, close_tab, reset, close.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["open", "status", "tabs", "new_tab", "select_tab", "close_tab", "reset", "close"], "default": "status"},
                    "url": {"type": "string", "description": "Initial URL for open or URL for new_tab."},
                    "page_id": {"type": "integer", "minimum": 0},
                    "profile": {"type": "string", "description": "Opt-in saved login profile name. Only set after explicit user instruction; omit for an isolated disposable session."},
                    "clear_profile": {"type": "boolean", "description": "Delete the selected saved profile while resetting."}
                },
                "required": ["action"]
            }
        },
        {
            "name": "browser_run",
            "description": "Run up to 25 browser actions sequentially against the persistent chat browser, then return one current accessibility snapshot. Target elements by uid from the latest snapshot rather than fragile CSS selectors. Use one batch for a coherent interaction instead of issuing many MCP calls.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "actions": {
                        "type": "array", "minItems": 1, "maxItems": 25,
                        "items": {
                            "type": "object",
                            "properties": {
                                "action": {"type": "string", "enum": ["navigate", "back", "forward", "reload", "snapshot", "click", "click_at", "fill", "fill_form", "type_text", "press_key", "hover", "drag", "upload_file", "evaluate", "wait_for", "resize", "emulate", "handle_dialog", "trace_start", "trace_stop", "download", "wait_for_download"]},
                                "url": {"type": "string"}, "uid": {"type": "string"}, "value": {"type": "string"}, "text": {"type": "string"}, "key": {"type": "string"},
                                "function": {"type": "string", "description": "JavaScript function declaration, e.g. () => document.title."},
                                "selector": {"type": "string"}, "condition": {"type": "string", "description": "JavaScript expression for wait_for."},
                                "timeout_ms": {"type": "integer", "minimum": 100, "maximum": 300000},
                                "elements": {"type": "array", "items": {"type": "object", "properties": {"uid": {"type": "string"}, "value": {"type": "string"}}, "required": ["uid", "value"]}},
                                "from_uid": {"type": "string"}, "to_uid": {"type": "string"}, "file_path": {"type": "string"},
                                "width": {"type": "integer"}, "height": {"type": "integer"}, "x": {"type": "integer"}, "y": {"type": "integer"},
                                "double_click": {"type": "boolean"}, "submit_key": {"type": "string"}, "verbose": {"type": "boolean"},
                                "network": {"type": "string"}, "cpu": {"type": "number"}, "geolocation": {"type": "string"}, "user_agent": {"type": "string"}, "color_scheme": {"type": "string"}, "viewport": {"type": "string"}, "headers": {"type": "string"},
                                "dialog_action": {"type": "string", "enum": ["accept", "dismiss"]}, "prompt_text": {"type": "string"}
                            },
                            "required": ["action"]
                        }
                    },
                    "observe": {"type": "boolean", "default": true},
                    "stop_on_error": {"type": "boolean", "default": true}
                },
                "required": ["actions"]
            }
        },
        {
            "name": "browser_inspect",
            "description": "Collect bounded debugging evidence from the persistent browser in one call. Defaults to accessibility snapshot, console, and network. Add screenshot or PDF for visual evidence, lighthouse for accessibility/SEO/best-practice audit, or performance/trace for a Chrome DevTools trace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "include": {"type": "array", "items": {"type": "string", "enum": ["snapshot", "console", "network", "screenshot", "pdf", "lighthouse", "performance", "trace"]}},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50},
                    "full_page": {"type": "boolean"},
                    "audit_mode": {"type": "string", "enum": ["navigation", "snapshot"]},
                    "device": {"type": "string", "enum": ["desktop", "mobile"]}
                }
            }
        }
    ])
}

fn write_json_line(writer: &mut impl Write, value: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn validate_upstream_result(result: Value) -> Result<Value, String> {
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(result_text(&result).unwrap_or_else(|| "browser action failed".to_string()));
    }
    Ok(result)
}

fn format_upstream_error(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Chrome DevTools MCP request failed")
        .to_string()
}

fn result_text(result: &Value) -> Option<String> {
    let parts = result.get("content")?.as_array()?;
    Some(
        parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn truncate_text(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut output = text
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

fn combine_results(summary: Value, results: Vec<(&str, Value)>) -> Value {
    let mut content = vec![json!({"type": "text", "text": summary.to_string()})];
    for (label, result) in results {
        append_content(&mut content, result, label);
    }
    json!({"content": content})
}

fn append_content(content: &mut Vec<Value>, result: Value, label: &str) {
    let Some(parts) = result.get("content").and_then(Value::as_array) else {
        content.push(json!({"type": "text", "text": format!("{label}: {result}")}));
        return;
    };
    for part in parts {
        if part.get("type").and_then(Value::as_str) == Some("text") {
            let text = part.get("text").and_then(Value::as_str).unwrap_or_default();
            content.push(json!({"type": "text", "text": format!("{label}:\n{text}")}));
        } else {
            content.push(part.clone());
        }
    }
}

fn append_non_text_content(content: &mut Vec<Value>, result: &Value) {
    if let Some(parts) = result.get("content").and_then(Value::as_array) {
        content.extend(
            parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) != Some("text"))
                .cloned(),
        );
    }
}

fn attach_private_capture_artifacts(result: &mut Value) -> Result<(), String> {
    let Some(parts) = result.get("content").and_then(Value::as_array) else {
        return Ok(());
    };
    let mut artifacts = Vec::new();
    for part in parts {
        if part.get("type").and_then(Value::as_str) != Some("image") {
            continue;
        }
        let Some(data) = part.get("data").and_then(Value::as_str) else {
            continue;
        };
        let media_type = part
            .get("mimeType")
            .or_else(|| part.get("mime_type"))
            .and_then(Value::as_str)
            .unwrap_or("image/webp");
        let extension = capture_extension(media_type);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| format!("browser capture was not valid base64: {error}"))?;
        let directory = capture_directory()?;
        prune_capture_directory(&directory);
        let path = directory.join(format!("capture-{}.{}", uuid::Uuid::new_v4(), extension));
        write_private_capture(&path, &bytes)?;
        artifacts.push(json!({
            "path": path,
            "media_type": media_type,
            "bytes": bytes.len(),
        }));
    }
    if artifacts.is_empty() {
        return Ok(());
    }
    let content = result
        .get_mut("content")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "browser capture content changed unexpectedly".to_string())?;
    content.push(json!({
        "type": "text",
        "text": json!({"status": "captured", "artifacts": artifacts}).to_string(),
    }));
    Ok(())
}

fn capture_directory() -> Result<PathBuf, String> {
    let user_home = std::env::var_os("FLAMEL_USER_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| "authenticated browser artifact root is unavailable".to_string())?;
    let session = profile_scope(std::env::var("FLAMEL_SESSION_ID").ok().as_deref());
    let directory = user_home
        .join(".flamel")
        .join("browser-artifacts")
        .join(session);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("failed to create browser artifact directory: {error}"))?;
    set_owner_only(&directory)?;
    Ok(directory)
}

fn capture_extension(media_type: &str) -> &'static str {
    match media_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        _ => "webp",
    }
}

fn prune_capture_directory(directory: &Path) {
    let now = std::time::SystemTime::now();
    let mut captures = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then(|| (entry.path(), metadata.modified().ok()))
        })
        .collect::<Vec<_>>();
    captures.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    for (index, (path, modified)) in captures.into_iter().enumerate() {
        let expired = modified
            .and_then(|value| now.duration_since(value).ok())
            .is_some_and(|age| age.as_secs() > CAPTURE_RETENTION_SECS);
        if index >= MAX_SESSION_CAPTURES || expired {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn write_private_capture(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes)
        .map_err(|error| format!("failed to persist browser capture: {error}"))?;
    set_private_file(path)
}

fn text_result(value: Value) -> Value {
    json!({"content": [{"type": "text", "text": value.to_string()}]})
}

fn string_arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn required_string<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    string_arg(arguments, key).ok_or_else(|| format!("{key} is required"))
}

fn required_u64(arguments: &Value, key: &str) -> Result<u64, String> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{key} is required"))
}

fn bool_arg(arguments: &Value, key: &str) -> bool {
    arguments.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn timeout_ms(arguments: &Value) -> u64 {
    arguments
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(100, 300_000)
}

fn copy_keys(arguments: &Value, keys: &[(&str, &str)]) -> Value {
    let mut output = Map::new();
    for (source, target) in keys {
        if let Some(value) = arguments.get(*source) {
            output.insert((*target).to_string(), value.clone());
        }
    }
    Value::Object(output)
}

fn wait_function(arguments: &Value) -> Result<String, String> {
    let timeout = timeout_ms(arguments);
    let predicate = if let Some(selector) = string_arg(arguments, "selector") {
        format!("document.querySelector({}) !== null", json!(selector))
    } else if let Some(text) = string_arg(arguments, "text") {
        format!("document.body?.innerText.includes({})", json!(text))
    } else if let Some(condition) = string_arg(arguments, "condition") {
        format!("Boolean({condition})")
    } else {
        return Err("wait_for requires selector, text, or condition".to_string());
    };
    Ok(format!(
        "async () => {{ const deadline = Date.now() + {timeout}; while (Date.now() < deadline) {{ if ({predicate}) return {{matched:true,url:location.href,title:document.title}}; await new Promise(resolve => setTimeout(resolve, 250)); }} throw new Error('wait_for timed out after {timeout}ms'); }}"
    ))
}

fn validate_profile_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(
            "profile must contain 1-64 ASCII letters, numbers, dashes, or underscores".to_string(),
        );
    }
    Ok(name.to_string())
}

fn profile_directory(profile: &str) -> Result<PathBuf, String> {
    let profile = validate_profile_name(profile)?;
    let base = std::env::var_os("FLAMEL_BROWSER_PROFILE_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("FLAMEL_USER_HOME")
                .map(|home| PathBuf::from(home).join(".flamel/browser-profiles"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".flamel/browser-profiles"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp/flamel-browser-profiles"));
    let scope = profile_scope(std::env::var("FLAMEL_USER_ID").ok().as_deref());
    let root = base.join(scope);
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("failed to create browser profile root: {error}"))?;
    set_owner_only(&root)?;
    let directory = root.join(profile);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("failed to create browser profile: {error}"))?;
    set_owner_only(&directory)?;
    Ok(directory)
}

fn profile_scope(user_id: Option<&str>) -> String {
    let scope = user_id
        .unwrap_or("local")
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(128)
        .collect::<String>();
    if scope.is_empty() {
        "local".to_string()
    } else {
        scope
    }
}

fn validated_upload_path(raw_path: &str) -> Result<String, String> {
    let project_root = std::env::var_os("FLAMEL_PROJECT_PATH")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "browser upload project root is unavailable".to_string())?;
    let path = PathBuf::from(raw_path);
    let path = if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    };
    let path = path
        .canonicalize()
        .map_err(|error| format!("browser upload file is unavailable: {error}"))?;
    if !path.is_file() {
        return Err("browser upload path must be a regular file".to_string());
    }

    let mut roots = vec![project_root];
    if let Some(user_home) = std::env::var_os("FLAMEL_USER_HOME") {
        roots.push(PathBuf::from(user_home));
    }
    if !path_allowed_for_upload(&path, &roots) {
        return Err("browser upload path is outside the authenticated user roots".to_string());
    }
    Ok(path.to_string_lossy().into_owned())
}

fn path_allowed_for_upload(path: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| path.starts_with(root))
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| format!("failed to protect {}: {error}", path.display()))
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("failed to protect {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn chrome_executable() -> Option<PathBuf> {
    for value in [
        std::env::var_os("CHROME_BIN"),
        std::env::var_os("PUPPETEER_EXECUTABLE_PATH"),
    ]
    .into_iter()
    .flatten()
    {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    [
        "/usr/local/bin/google-chrome",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/microsoft-edge",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_only_the_compact_browser_surface() {
        let tools = tools_list();
        let names = tools
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(names, ["browser_session", "browser_run", "browser_inspect"]);
    }

    #[test]
    fn saved_profile_names_cannot_escape_the_profile_root() {
        assert!(validate_profile_name("qa-login_2").is_ok());
        assert!(validate_profile_name("../secrets").is_err());
        assert!(validate_profile_name("with spaces").is_err());
        assert_eq!(profile_scope(Some("user-123")), "user-123");
        assert_eq!(profile_scope(Some("../../")), "local");
    }

    #[test]
    fn wait_for_is_bounded_and_supports_stable_selectors() {
        let function =
            wait_function(&json!({"selector": "[data-ready]", "timeout_ms": 1200})).unwrap();
        assert!(function.contains("document.querySelector"));
        assert!(function.contains("1200"));
        assert!(function.contains("setTimeout(resolve, 250)"));
    }

    #[test]
    fn upload_paths_cannot_escape_the_allowed_roots() {
        let root =
            std::env::temp_dir().join(format!("flamel-browser-upload-{}", uuid::Uuid::new_v4()));
        let outside =
            std::env::temp_dir().join(format!("flamel-browser-outside-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("asset.txt"), b"asset").unwrap();
        std::fs::write(&outside, b"private").unwrap();

        let inside = root.join("asset.txt").canonicalize().unwrap();
        let outside = outside.canonicalize().unwrap();
        assert!(path_allowed_for_upload(
            &inside,
            std::slice::from_ref(&root)
        ));
        assert!(!path_allowed_for_upload(
            &outside,
            std::slice::from_ref(&root)
        ));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(outside);
    }

    #[cfg(unix)]
    #[test]
    fn browser_captures_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "flamel-browser-private-capture-{}.webp",
            uuid::Uuid::new_v4()
        ));
        write_private_capture(&path, b"capture").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let _ = std::fs::remove_file(path);
        assert_eq!(mode, 0o600);
    }
}
