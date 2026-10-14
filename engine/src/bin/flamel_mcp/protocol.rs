use super::*;

pub(super) const ENGINEERING_SCHEMA: &str = "flamel.engineering_result.v1";
pub(super) const ENGINEERING_TEXT_BYTES: usize = 10 * 1024;
pub(super) const ENGINEERING_SOURCE_BYTES: usize = 8 * 1024;
pub(super) const ENGINEERING_FIELD_BYTES: usize = 512;
pub(super) const ENGINEERING_MAX_ROWS: usize = 32;

pub(super) fn engineering_tool_action(name: &str, args: &Value) -> Option<(&'static str, String)> {
    let requested = args.get("action").and_then(Value::as_str);
    let action = match name {
        "file_read" => requested.unwrap_or("read"),
        "flamel_read" | "read_file" => requested.unwrap_or("read"),
        "batch_read" => requested.unwrap_or("batch"),
        "file_summary" => requested.unwrap_or("summary"),
        "flamel_glob" => requested.unwrap_or("glob"),
        "find_file" => requested.unwrap_or("find"),
        "list_dir" => requested.unwrap_or("list_dir"),
        "code_nav" => requested.unwrap_or("search"),
        "flamel_search" => requested.unwrap_or("search"),
        "get_symbols_overview" => requested.unwrap_or("symbols"),
        "find_symbol" => requested.unwrap_or("find"),
        "find_referencing_symbols" => requested.unwrap_or("refs"),
        "get_imports" => requested.unwrap_or("imports"),
        "gather_context" => requested.unwrap_or("context"),
        "dependency_graph" => requested.unwrap_or("deps"),
        _ => return None,
    };
    let tool = if matches!(
        name,
        "file_read"
            | "flamel_read"
            | "read_file"
            | "batch_read"
            | "file_summary"
            | "flamel_glob"
            | "find_file"
            | "list_dir"
    ) {
        "file_read"
    } else {
        "code_nav"
    };
    Some((tool, action.to_string()))
}

pub(super) fn bounded_utf8(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }
    let mut end = max_bytes.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

pub(super) fn engineering_result(
    tool: &str,
    action: &str,
    full_text: String,
    data: Value,
    counts: Value,
    payload_truncated: bool,
    mut warnings: Vec<String>,
) -> String {
    let (text, text_truncated) = bounded_utf8(&full_text, ENGINEERING_TEXT_BYTES);
    let truncated = payload_truncated || text_truncated;
    let artifacts = if truncated {
        flamel::mcp_gateway::store_text_artifact(tool, &full_text)
            .into_iter()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if truncated {
        warnings.push(
            "Result was bounded at the producer; the complete text is available as an artifact."
                .to_string(),
        );
    }
    json!({
        "schema": ENGINEERING_SCHEMA,
        "tool": tool,
        "action": action,
        "status": "ok",
        "summary": text.lines().next().unwrap_or_default(),
        "counts": counts,
        "truncated": truncated,
        "truncationReason": truncated.then_some("producer_budget"),
        "warnings": warnings,
        "artifacts": artifacts,
        "data": data,
        "text": text,
    })
    .to_string()
}

pub(super) fn engineering_error_result(name: &str, args: &Value, error: &str) -> Option<Value> {
    let (tool, action) = engineering_tool_action(name, args)?;
    let text = format!("Error: {error}");
    let structured = json!({
        "schema": ENGINEERING_SCHEMA,
        "tool": tool,
        "action": action,
        "status": "error",
        "summary": text,
        "counts": {},
        "truncated": false,
        "truncationReason": null,
        "warnings": [],
        "artifacts": [],
        "data": {"error": error},
        "text": text,
    });
    Some(flamel::mcp_gateway::bound_result(
        name,
        json!({
            "content": [{"type": "text", "text": text}],
            "structuredContent": structured,
            "isError": true,
        }),
    ))
}

pub(super) fn engineering_display_path(path: &str, project_path: &str) -> String {
    let displayed = path
        .strip_prefix(project_path)
        .and_then(|value| value.strip_prefix('/'))
        .unwrap_or(path);
    bounded_utf8(displayed, ENGINEERING_FIELD_BYTES).0
}

pub(super) fn engineering_language(path: &str) -> &'static str {
    match std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
    {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "mdx" => "markdown",
        "sh" | "bash" => "shell",
        _ => "text",
    }
}

pub(super) fn numbered_source(source: &str, start_line: usize) -> String {
    source
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{:>6}\t{}", start_line + index, line))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn engineering_imports(content: &str, extension: &str) -> Vec<Value> {
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            match extension {
                "rs" => {
                    trimmed.starts_with("use ")
                        || trimmed.starts_with("extern crate")
                        || trimmed.starts_with("mod ")
                }
                "ts" | "tsx" | "js" | "jsx" => {
                    trimmed.starts_with("import ")
                        || (trimmed.starts_with("const ") && trimmed.contains("require("))
                }
                "py" => trimmed.starts_with("import ") || trimmed.starts_with("from "),
                "go" => trimmed.starts_with("import "),
                _ => trimmed.starts_with("import ") || trimmed.starts_with("use "),
            }
        })
        .map(|(index, line)| {
            json!({
                "line": index + 1,
                "text": bounded_utf8(line, ENGINEERING_FIELD_BYTES).0,
                "kind": if line.trim_start().starts_with("mod ") { "module" } else { "import" },
            })
        })
        .collect()
}

pub(super) fn bounded_symbol_tree(symbols: &[Value], limit: usize) -> (Vec<Value>, usize, bool) {
    let total = symbols
        .iter()
        .map(|symbol| {
            1 + symbol
                .get("methods")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        })
        .sum::<usize>();
    let mut remaining = limit;
    let mut bounded = Vec::new();
    for symbol in symbols {
        if remaining == 0 {
            break;
        }
        let mut symbol = symbol.clone();
        for field in ["name", "kind"] {
            if let Some(value) = symbol[field].as_str() {
                symbol[field] = json!(bounded_utf8(value, ENGINEERING_FIELD_BYTES).0);
            }
        }
        remaining -= 1;
        if let Some(methods) = symbol.get_mut("methods").and_then(Value::as_array_mut) {
            methods.truncate(remaining);
            for method in methods.iter_mut() {
                for field in ["name", "kind"] {
                    if let Some(value) = method[field].as_str() {
                        method[field] = json!(bounded_utf8(value, ENGINEERING_FIELD_BYTES).0);
                    }
                }
            }
        }
        let used = symbol
            .get("methods")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        remaining = remaining.saturating_sub(used);
        bounded.push(symbol);
    }
    (bounded, total, total > limit)
}

pub(super) fn run_rg_records(
    pattern: &str,
    search_path: &str,
    context: u64,
    include: Option<&str>,
    project_path: &str,
) -> Result<(Vec<Value>, Vec<String>), String> {
    let mut command = flamel::mcp_parent_guard::command("rg");
    command
        .arg("--json")
        .arg("--color=never")
        .arg("-C")
        .arg(context.to_string());
    if let Some(glob) = include {
        command.arg("--glob").arg(glob);
    }
    command
        .arg("--regexp")
        .arg(pattern)
        .arg("--")
        .arg(search_path);
    let output = command
        .output()
        .map_err(|error| format!("rg not found: {error}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(if stderr.is_empty() {
            format!("rg exited with {}", output.status)
        } else {
            format!("rg error: {stderr}")
        });
    }

    let mut records = Vec::new();
    let mut warnings = Vec::new();
    for encoded in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(event) = serde_json::from_str::<Value>(encoded) else {
            warnings.push("rg emitted an unreadable JSON event".to_string());
            continue;
        };
        let role = match event.get("type").and_then(Value::as_str) {
            Some("match") => "match",
            Some("context") => "context",
            _ => continue,
        };
        let data = &event["data"];
        let Some(path) = data["path"]["text"].as_str() else {
            warnings.push("rg returned a non-UTF-8 path".to_string());
            continue;
        };
        let Some(text) = data["lines"]["text"].as_str() else {
            warnings.push("rg returned a non-UTF-8 line".to_string());
            continue;
        };
        let line = data["line_number"].as_u64().unwrap_or(0);
        let column = data["submatches"]
            .as_array()
            .and_then(|matches| matches.first())
            .and_then(|value| value["start"].as_u64())
            .map(|start| start + 1);
        records.push(json!({
            "path": engineering_display_path(path, project_path),
            "line": line,
            "column": column,
            "text": text.trim_end_matches(['\r', '\n']),
            "role": role,
        }));
    }
    warnings.sort();
    warnings.dedup();
    Ok((records, warnings))
}

pub(super) fn bounded_rg_records(records: &[Value]) -> (Vec<Value>, bool) {
    let mut truncated = records.len() > ENGINEERING_MAX_ROWS;
    let bounded = records
        .iter()
        .take(ENGINEERING_MAX_ROWS)
        .cloned()
        .map(|mut record| {
            if let Some(text) = record["text"].as_str() {
                let (text, field_truncated) = bounded_utf8(text, ENGINEERING_FIELD_BYTES);
                record["text"] = json!(text);
                truncated |= field_truncated;
            }
            record
        })
        .collect();
    (bounded, truncated)
}

pub(super) fn format_rg_records(records: &[Value], empty: &str) -> String {
    if records.is_empty() {
        return empty.to_string();
    }
    records
        .iter()
        .map(|record| {
            let separator = if record["role"] == "match" { ':' } else { '-' };
            let path = record["path"].as_str().unwrap_or_default();
            let line = record["line"].as_u64().unwrap_or(0);
            let column = record["column"].as_u64().unwrap_or(0);
            let text = record["text"].as_str().unwrap_or_default();
            if column > 0 {
                format!("{path}{separator}{line}:{column}{separator}{text}")
            } else {
                format!("{path}{separator}{line}{separator}{text}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn structured_display_text<'a>(
    encoded: &'a str,
    structured: Option<&'a Value>,
) -> &'a str {
    structured
        .filter(|value| value.get("schema").and_then(Value::as_str) == Some(ENGINEERING_SCHEMA))
        .and_then(|value| value.get("text"))
        .and_then(Value::as_str)
        .unwrap_or(encoded)
}

pub(super) fn handle_request(
    req: &Value,
    base_url: &str,
    token: &str,
    session_id: &str,
    project_path: &str,
    profile: &McpProfile,
) -> Option<Value> {
    let id = req.get("id");
    let method = req.get("method")?.as_str()?;

    if method == "initialize" {
        let requested_protocol = req
            .get("params")
            .and_then(|params| params.get("protocolVersion"))
            .and_then(Value::as_str)
            .unwrap_or(flamel::mcp_gateway::LEGACY_PROTOCOL_VERSION);
        let protocol_version = flamel::mcp_gateway::negotiated_protocol(Some(requested_protocol));
        return Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": protocol_version,
                "capabilities": {
                    "tools": {"listChanged": false},
                    "prompts": {"listChanged": false}
                },
                "serverInfo": {"name": "flamel", "version": flamel::BUILD_VERSION},
                "instructions": profile.instructions(),
            }
        }));
    }

    if method == "server/discover" {
        return Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": flamel::mcp_gateway::discovery(),
        }));
    }

    if method.starts_with("notifications/") {
        return None;
    }

    if method == "prompts/list" {
        return Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "prompts": [{
                    "name": "ponytail",
                    "title": "Ponytail mode",
                    "description": "Apply Ponytail's minimal, shortest-working-solution coding instructions.",
                    "arguments": [{
                        "name": "mode",
                        "description": "Intensity: lite, full (default), or ultra",
                        "required": false
                    }]
                }]
            }
        }));
    }

    if method == "prompts/get" {
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
        if params.get("name").and_then(Value::as_str) != Some("ponytail") {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32602, "message": "Unknown prompt"}
            }));
        }
        let mode = params
            .get("arguments")
            .and_then(|arguments| arguments.get("mode"))
            .and_then(Value::as_str);
        return Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "description": "Ponytail coding instructions",
                "messages": [{
                    "role": "user",
                    "content": {
                        "type": "text",
                        "text": flamel::ponytail::build_instructions(mode)
                    }
                }]
            }
        }));
    }

    if method == "tools/list" {
        return Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": tools_list_for_profile(profile),
                "ttlMs": 300000,
                "cacheScope": "public",
            },
        }));
    }

    if method == "tools/call" {
        let params = req.get("params")?;
        let name = params.get("name")?.as_str()?;
        if !profile.allows_tool(name) {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": tool_error_result(
                    name,
                    &format!(
                        "tool {name} is not available in MCP profile {}",
                        profile.label()
                    ),
                ),
            }));
        }
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let args = match normalize_tool_arguments(name, args, &tools_list_for_profile(profile)) {
            Ok(normalized) => normalized.arguments,
            Err(e) => {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": tool_error_result(name, &e),
                }));
            }
        };

        if name == "ponytail_instructions" {
            let requested_mode = args.get("mode").and_then(Value::as_str);
            let mode = flamel::ponytail::resolve_mode(requested_mode);
            let instructions = flamel::ponytail::build_instructions(Some(mode));
            let result = flamel::mcp_gateway::bound_result(
                name,
                json!({
                    "content": [{"type": "text", "text": instructions}],
                    "structuredContent": {
                        "mode": mode,
                        "instructions": instructions
                    },
                    "isError": false
                }),
            );
            return Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
        }

        if flamel::mcp_gateway::owns_tool(name) {
            let result = flamel::mcp_gateway::call(name, args)
                .unwrap_or_else(|error| tool_error_result(name, &error));
            return Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
        }

        match handle_tool_call(name, &args, base_url, token, session_id, project_path) {
            Ok(text) => {
                // Multi-content: "data:multi;<json-array>" — pass through directly.
                // Engineering envelopes expose their concise compatibility text separately.
                let structured_content = serde_json::from_str::<Value>(&text)
                    .ok()
                    .filter(|value| value.is_object() || value.is_array());
                let display_text = structured_display_text(&text, structured_content.as_ref());
                let content = if let Some(rest) = text.strip_prefix("data:multi;") {
                    serde_json::from_str::<serde_json::Value>(rest)
                        .unwrap_or_else(|_| json!([{"type": "text", "text": &text}]))
                } else if let Some(rest) = text.strip_prefix("data:") {
                    if let Some(semi) = rest.find(';') {
                        let mime = &rest[..semi];
                        let after_semi = &rest[semi + 1..];
                        if let Some(b64) = after_semi.strip_prefix("base64,") {
                            json!([{"type": "image", "data": b64, "mimeType": mime}])
                        } else {
                            json!([{"type": "text", "text": display_text}])
                        }
                    } else {
                        json!([{"type": "text", "text": display_text}])
                    }
                } else {
                    json!([{"type": "text", "text": display_text}])
                };
                let mut result = json!({"content": content, "isError": false});
                if let Some(structured_content) = structured_content {
                    result["structuredContent"] = structured_content;
                }
                let result = flamel::mcp_gateway::bound_result(name, result);
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result,
                }));
            }
            Err(error) => {
                let result = engineering_error_result(name, &args, &error)
                    .unwrap_or_else(|| tool_error_result(name, &error));
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result,
                }));
            }
        }
    }

    id.map(|id| {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": "Method not found"},
        })
    })
}

/// Resolve a path that may be relative against project_path.
/// Resolve a path that may be relative against project_path.
/// If the path is absolute, validate it stays within FLAMEL_USER_HOME (if set).
pub(super) fn resolve_path(path: &str, project_path: &str) -> String {
    if std::path::Path::new(path).is_absolute() {
        // Security: reject paths that escape the user's home directory
        if let Ok(user_home) = std::env::var("FLAMEL_USER_HOME") {
            if !user_home.is_empty() {
                // Canonicalize to resolve symlinks and .. traversal
                let canonical =
                    std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path));
                if !canonical.starts_with(&user_home) {
                    tracing::warn!(
                        "resolve_path: blocked escape attempt — {} is outside user home {}",
                        path,
                        user_home
                    );
                    return format!("__BLOCKED:{path}");
                }
            }
        }
        path.to_string()
    } else {
        format!("{project_path}/{path}")
    }
}

/// Build a modified args Value with relative_path resolved to absolute.
pub(super) fn with_resolved_path(args: &Value, project_path: &str) -> Value {
    let mut a = args.clone();
    let key = if args.get("relative_path").is_some() {
        "relative_path"
    } else {
        "path"
    };
    if let Some(p) = args.get(key).and_then(|v| v.as_str()) {
        a[key] = json!(resolve_path(p, project_path));
    }
    a
}

pub(super) fn format_runtime_logs(payload: &Value) -> String {
    let logs = payload
        .get("logs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let count = logs.len();
    let counts = payload.get("countsByLevel").and_then(|v| v.as_object());
    let count_summary = counts
        .map(|map| {
            ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"]
                .iter()
                .filter_map(|level| {
                    map.get(*level)
                        .and_then(|v| v.as_u64())
                        .filter(|n| *n > 0)
                        .map(|n| format!("{level}={n}"))
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "no captured counts".to_string());

    if logs.is_empty() {
        return format!("Runtime logs: none returned ({count_summary}).");
    }

    let mut lines = vec![format!("Runtime logs: {count} returned ({count_summary})")];
    for entry in logs {
        let sequence = entry.get("sequence").and_then(|v| v.as_u64()).unwrap_or(0);
        let timestamp = entry
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let level = entry.get("level").and_then(|v| v.as_str()).unwrap_or("");
        let target = entry.get("target").and_then(|v| v.as_str()).unwrap_or("");
        let message = entry.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let fields = entry
            .get("fields")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let name = item.get("name").and_then(|v| v.as_str())?;
                        let value = item.get("value").and_then(|v| v.as_str()).unwrap_or("");
                        Some(format!("{name}={value}"))
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let location = match (
            entry.get("file").and_then(|v| v.as_str()),
            entry.get("line").and_then(|v| v.as_u64()),
        ) {
            (Some(file), Some(line)) => format!(" ({file}:{line})"),
            _ => String::new(),
        };
        let detail = [message, fields.as_str()]
            .iter()
            .filter(|part| !part.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let mut line = if detail.is_empty() {
            format!("#{sequence} {timestamp} {level} {target}{location}")
        } else {
            format!("#{sequence} {timestamp} {level} {target}{location}: {detail}")
        };
        line = cap_chars(&line, 1_000, "...[truncated]");
        lines.push(line);
    }
    lines.join("\n")
}

pub(super) fn build_runtime_logs_path(args: &Value) -> String {
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .clamp(1, 200);
    let mut path = format!("/api/admin/diagnostics/runtime-logs?limit={limit}");
    if let Some(min_level) = args
        .get("min_level")
        .or_else(|| args.get("minLevel"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        path.push_str("&minLevel=");
        path.push_str(&urlencoding::encode(min_level));
    }
    path
}

pub(super) fn value_field(value: &Value, key: &str) -> Value {
    value.get(key).cloned().unwrap_or(Value::Null)
}

pub(super) fn plan_marker_payload(plan: &Value, include_items: bool) -> Value {
    let mut payload = json!({
        "planId": value_field(plan, "id"),
        "title": value_field(plan, "title"),
        "description": value_field(plan, "description"),
        "itemCount": plan.get("items").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        "finished": value_field(plan, "finished"),
        "versionCount": value_field(plan, "versionCount"),
        "executionStatus": value_field(plan, "executionStatus"),
        "approvalStatus": value_field(plan, "approvalStatus"),
        "activeItemId": value_field(plan, "activeItemId"),
        "approvalRequestedAt": value_field(plan, "approvalRequestedAt"),
        "approvedAt": value_field(plan, "approvedAt"),
        "executionStartedAt": value_field(plan, "executionStartedAt"),
        "executionUpdatedAt": value_field(plan, "executionUpdatedAt"),
    });
    if include_items {
        payload["items"] = value_field(plan, "items");
    }
    payload
}

pub(super) fn plan_item_marker_payload(plan_id: &str, item: &Value) -> Value {
    json!({
        "planId": plan_id,
        "itemId": value_field(item, "id"),
        "status": value_field(item, "status"),
        "text": value_field(item, "text"),
        "notes": value_field(item, "notes"),
        "section": value_field(item, "section"),
        "reasoningFloor": value_field(item, "reasoningFloor"),
        "executionStatus": value_field(item, "executionStatus"),
        "approvalStatus": value_field(item, "approvalStatus"),
        "activeItemId": value_field(item, "activeItemId"),
        "finished": value_field(item, "finished"),
    })
}

pub(super) fn format_plan(plan: &Value) -> String {
    let title = plan
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled");
    let description = plan.get("description").and_then(|v| v.as_str());
    let id = plan.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let execution_status = plan
        .get("executionStatus")
        .and_then(|v| v.as_str())
        .unwrap_or("ready");
    let approval_status = plan
        .get("approvalStatus")
        .and_then(|v| v.as_str())
        .unwrap_or("approved");
    let active_item_id = plan.get("activeItemId").and_then(|v| v.as_str());

    let items = plan
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let done_count = items
        .iter()
        .filter(|i| i.get("status").and_then(|s| s.as_str()) == Some("done"))
        .count();
    let blocked_count = items
        .iter()
        .filter(|i| i.get("status").and_then(|s| s.as_str()) == Some("blocked"))
        .count();

    let mut parts = vec![
        format!(
            "# {title}{}",
            description.map(|d| format!("\n{d}")).unwrap_or_default()
        ),
        format!("Plan ID: {id}"),
        format!("Progress: {done_count}/{} done", items.len()),
        format!("State: execution={execution_status}, approval={approval_status}"),
    ];
    if let Some(review) = plan.get("browserReview") {
        parts.push(format!("Browser review: {review}"));
    }
    if let Some(active_item_id) = active_item_id {
        parts.push(format!("Active item ID: {active_item_id}"));
    }
    if blocked_count > 0 {
        parts.push(format!("Blocked items: {blocked_count}"));
    }
    if approval_status == "pending" {
        parts.push(format!(
            "Approval pending: after explicit user approval, call plan(action=\"start\", plan_id=\"{id}\") before marking items in_progress or done."
        ));
    } else {
        parts.push(format!(
            "Track execution live with plan(action=\"update_item\", plan_id=\"{id}\", item_id=\"...\", status=\"in_progress\"|\"done\"|\"blocked\"). Mark each item done immediately after its own acceptance checks pass, before starting the next item; never batch-complete items at the end."
        ));
    }
    parts.push(
        "Full item data is in the leading PLAN_* marker JSON and the Plan panel.".to_string(),
    );
    parts.push(format!(
        "NEVER use Python, sqlite3, or shell to read plan data - only plan(action=\"get\", plan_id=\"{id}\")."
    ));
    parts.join("\n")
}

pub(super) fn build_plan_list_path(args: &Value, session_id: &str) -> String {
    let mut qs = String::new();
    if let Some(q) = args.get("query").and_then(|v| v.as_str()) {
        qs.push_str(&format!("q={}&", urlencoding::encode(q)));
    }
    if let Some(pp) = args.get("project_path").and_then(|v| v.as_str()) {
        qs.push_str(&format!("projectPath={}&", urlencoding::encode(pp)));
    }
    if let Some(list_session_id) = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| (!session_id.is_empty()).then_some(session_id))
    {
        qs.push_str(&format!(
            "sessionId={}&",
            urlencoding::encode(list_session_id)
        ));
    }
    if qs.is_empty() {
        "/api/plans".to_string()
    } else {
        format!("/api/plans?{qs}")
    }
}
