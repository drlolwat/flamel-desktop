//! LSP integration for flamel_mcp.
//! Spawns and manages Language Server Protocol servers per (project, language).
//! All servers are persistent — they stay alive across sessions, giving flamel a
//! significant advantage over Claude Code CLI which must cold-start LSP on every invocation.

use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};

// ── Server struct ──────────────────────────────────────────────────────────────

struct LspServer {
    /// Write end of the LSP process's stdin.
    stdin: Mutex<std::process::ChildStdin>,
    /// Responses keyed by request ID, filled by background reader thread.
    responses: Arc<Mutex<HashMap<u64, Value>>>,
    /// Diagnostics keyed by file URI, filled by reader thread from publishDiagnostics.
    diagnostics: Arc<Mutex<HashMap<String, Vec<Value>>>>,
    /// Files currently open in this LSP server (URIs).
    open_files: Mutex<HashSet<String>>,
    /// Monotonic request ID.
    next_id: AtomicU64,
    /// The child process — kept here so it isn't dropped while the server lives.
    _child: Mutex<std::process::Child>,
}

// SAFETY: LspServer is Send + Sync because every field is guarded by Mutex or is atomic.
unsafe impl Send for LspServer {}
unsafe impl Sync for LspServer {}

/// Global map: "{project_path}::{lang_id}" → Arc<LspServer>
fn lsp_servers() -> &'static Mutex<HashMap<String, Arc<LspServer>>> {
    static SERVERS: OnceLock<Mutex<HashMap<String, Arc<LspServer>>>> = OnceLock::new();
    SERVERS.get_or_init(|| Mutex::new(HashMap::new()))
}

// ── Language detection ─────────────────────────────────────────────────────────

/// Returns `(lang_id, shell_command)` for a given file path, or None if unsupported.
pub fn detect_lang(file: &str) -> Option<(&'static str, &'static str)> {
    let ext = std::path::Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" =>
            Some(("typescript", "typescript-language-server --stdio")),
        "rs" =>
            Some(("rust", "rust-analyzer")),
        "py" =>
            Some(("python", "pylsp")),
        "go" =>
            Some(("go", "gopls")),
        "c" | "h" | "cpp" | "cxx" | "cc" | "hpp" | "hxx" =>
            Some(("cpp", "clangd")),
        _ => None,
    }
}

fn file_lang_id(file: &str) -> &'static str {
    let ext = std::path::Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "ts" | "tsx"          => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "rs"                  => "rust",
        "py"                  => "python",
        "go"                  => "go",
        "c" | "h"             => "c",
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" => "cpp",
        _                     => "plaintext",
    }
}

pub fn path_to_uri(path: &str) -> String {
    // Percent-encode spaces and special chars in the path component
    let encoded: String = path.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '/' | '-' | '_' | '.' | '~' => c.to_string(),
        ' ' => "%20".to_string(),
        c   => format!("%{:02X}", c as u32),
    }).collect();
    if path.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}")
    }
}

pub fn uri_to_path(uri: &str) -> String {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    // Decode percent-encoding
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next().unwrap_or('0');
            let h2 = chars.next().unwrap_or('0');
            let hex = format!("{h1}{h2}");
            if let Ok(b) = u8::from_str_radix(&hex, 16) {
                out.push(b as char);
                continue;
            }
        }
        out.push(c);
    }
    out
}

// ── LSP message framing (Content-Length: N\r\n\r\n{body}) ─────────────────────

fn read_lsp_message(reader: &mut BufReader<std::process::ChildStdout>) -> Result<Value, String> {
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("LSP stdout EOF".to_string());
        }
        let trimmed = line.trim_end_matches(|c: char| c == '\r' || c == '\n');
        if trimmed.is_empty() {
            break; // blank line = end of headers
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length: ") {
            content_length = rest.trim().parse().unwrap_or(0);
        }
    }
    if content_length == 0 {
        return Err("LSP message with zero content-length".to_string());
    }
    let mut body = vec![0u8; content_length];
    // BufReader<ChildStdout> implements Read, so read_exact is available
    use std::io::Read;
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| format!("LSP JSON parse: {e}"))
}

// ── Server impl ────────────────────────────────────────────────────────────────

impl LspServer {
    fn write_lsp(&self, msg: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        let mut stdin = self.stdin.lock().map_err(|_| "stdin lock poisoned".to_string())?;
        stdin.write_all(header.as_bytes()).map_err(|e| e.to_string())?;
        stdin.write_all(&body).map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn send_notification(&self, method: &str, params: Value) -> Result<(), String> {
        self.write_lsp(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    fn send_request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.write_lsp(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;

        // Spin-poll for the response (reader thread fills self.responses).
        // Timeout: 10 s for slow servers (rust-analyzer initial index can be sluggish).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            {
                let mut map = self.responses.lock()
                    .map_err(|_| "responses lock poisoned".to_string())?;
                if let Some(resp) = map.remove(&id) {
                    return if let Some(err) = resp.get("error") {
                        Err(format!("LSP error: {err}"))
                    } else {
                        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
                    };
                }
            }
            if std::time::Instant::now() > deadline {
                return Err(format!("LSP '{method}' timed out after 10 s"));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Ensure `file` (absolute path) is open in this LSP server.
    /// If already open, returns immediately. Otherwise sends didOpen and waits
    /// briefly for initial diagnostics.
    fn ensure_open(&self, file: &str) -> Result<(), String> {
        let uri = path_to_uri(file);
        let already = {
            let open = self.open_files.lock().map_err(|_| "open_files lock".to_string())?;
            open.contains(&uri)
        };
        if already {
            return Ok(());
        }
        let content = std::fs::read_to_string(file)
            .map_err(|e| format!("Cannot read {file}: {e}"))?;
        let lang_id = file_lang_id(file);
        self.send_notification("textDocument/didOpen", json!({
            "textDocument": {
                "uri": &uri,
                "languageId": lang_id,
                "version": 1,
                "text": content,
            }
        }))?;
        {
            let mut open = self.open_files.lock().map_err(|_| "open_files lock".to_string())?;
            open.insert(uri);
        }
        // Give the server ~300 ms to parse and emit publishDiagnostics
        std::thread::sleep(std::time::Duration::from_millis(300));
        Ok(())
    }

    /// Notify the LSP that a file was saved with new content, wait for diagnostics.
    pub fn notify_saved(&self, file: &str, content: &str) -> Result<(), String> {
        let uri = path_to_uri(file);
        // Make sure it's open first
        {
            let open = self.open_files.lock().map_err(|_| "open_files lock".to_string())?;
            if !open.contains(&uri) {
                drop(open);
                self.ensure_open(file)?;
                return Ok(()); // ensure_open already synced content
            }
        }
        // didChange (full sync)
        self.send_notification("textDocument/didChange", json!({
            "textDocument": {"uri": &uri, "version": 2},
            "contentChanges": [{"text": content}],
        }))?;
        // didSave
        self.send_notification("textDocument/didSave", json!({
            "textDocument": {"uri": &uri},
            "text": content,
        }))?;
        // Give the server time to re-lint and emit diagnostics
        std::thread::sleep(std::time::Duration::from_millis(600));
        Ok(())
    }

    fn get_diagnostics_str(&self, file: &str) -> String {
        let uri = path_to_uri(file);
        let map = match self.diagnostics.lock() {
            Ok(m) => m,
            Err(_) => return String::new(),
        };
        let diags = match map.get(&uri) {
            Some(d) if !d.is_empty() => d,
            _ => return String::new(),
        };
        diags.iter().map(|d| {
            let severity = match d["severity"].as_u64().unwrap_or(1) {
                1 => "error",
                2 => "warning",
                3 => "info",
                _ => "hint",
            };
            let ln  = d["range"]["start"]["line"].as_u64().unwrap_or(0) + 1;
            let col = d["range"]["start"]["character"].as_u64().unwrap_or(0);
            let msg = d["message"].as_str().unwrap_or("").replace('\n', " ");
            let code = d.get("code")
                .map(|c| format!("[{}] ", c))
                .unwrap_or_default();
            format!("{file}:{ln}:{col}: {severity}: {code}{msg}")
        }).collect::<Vec<_>>().join("\n")
    }
}

// ── Server lifecycle ───────────────────────────────────────────────────────────

fn spawn_lsp_server(command: &str, project_path: &str) -> Result<Arc<LspServer>, String> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return Err("empty LSP command".to_string());
    }
    let (prog, extra_args) = (parts[0], &parts[1..]);

    let mut command = std::process::Command::new(prog);
    crate::mcp_parent_guard::configure_child(&mut command);
    let mut child = command
        .args(extra_args)
        .current_dir(project_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // suppress LSP server noise
        .spawn()
        .map_err(|e| format!("Failed to spawn '{prog}': {e}. Is it installed and on PATH?"))?;

    let child_stdin  = child.stdin.take().ok_or("no stdin pipe")?;
    let child_stdout = child.stdout.take().ok_or("no stdout pipe")?;

    let responses:   Arc<Mutex<HashMap<u64, Value>>>         = Arc::new(Mutex::new(HashMap::new()));
    let diagnostics: Arc<Mutex<HashMap<String, Vec<Value>>>> = Arc::new(Mutex::new(HashMap::new()));

    // Background reader thread — routes responses and diagnostics
    {
        let resp_tx = Arc::clone(&responses);
        let diag_tx = Arc::clone(&diagnostics);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(child_stdout);
            loop {
                match read_lsp_message(&mut reader) {
                    Ok(msg) => {
                        if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
                            // Response to one of our requests
                            if let Ok(mut map) = resp_tx.lock() {
                                map.insert(id, msg);
                            }
                        } else if msg.get("method").and_then(|v| v.as_str())
                                     == Some("textDocument/publishDiagnostics")
                        {
                            let uri  = msg["params"]["uri"].as_str().unwrap_or("").to_string();
                            let diags = msg["params"]["diagnostics"]
                                .as_array().cloned().unwrap_or_default();
                            if let Ok(mut map) = diag_tx.lock() {
                                map.insert(uri, diags);
                            }
                        }
                        // All other notifications (window/logMessage, etc.) silently ignored
                    }
                    Err(_) => break, // Process exited
                }
            }
        });
    }

    let server = Arc::new(LspServer {
        stdin:      Mutex::new(child_stdin),
        responses,
        diagnostics,
        open_files: Mutex::new(HashSet::new()),
        next_id:    AtomicU64::new(1),
        _child:     Mutex::new(child),
    });

    // ── LSP handshake ──────────────────────────────────────────────────────────
    server.send_request("initialize", json!({
        "processId": std::process::id(),
        "rootUri": path_to_uri(project_path),
        "capabilities": {
            "textDocument": {
                "hover":      {"contentFormat": ["markdown", "plaintext"]},
                "definition": {},
                "references": {},
                "signatureHelp": {},
                "formatting": {},
                "codeAction": {},
                "publishDiagnostics": {
                    "relatedInformation": false,
                    "versionSupport": false,
                }
            },
            "workspace": {
                "workspaceFolders": true,
            }
        },
        "workspaceFolders": [{
            "uri":  path_to_uri(project_path),
            "name": std::path::Path::new(project_path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("workspace"),
        }]
    }))?;

    server.send_notification("initialized", json!({}))?;

    Ok(server)
}

/// Return the Arc<LspServer> for this (project, file) pair, spawning if needed.
fn get_or_create_server(file: &str, project_path: &str) -> Result<Arc<LspServer>, String> {
    let (lang_id, command) = detect_lang(file)
        .ok_or_else(|| {
            let ext = std::path::Path::new(file)
                .extension().and_then(|e| e.to_str()).unwrap_or("?");
            format!("No LSP server configured for .{ext} files. Supported: .ts .tsx .js .jsx .rs .py .go .c .cpp .h")
        })?;

    let key = format!("{project_path}::{lang_id}");

    // Fast path: already running
    {
        let map = lsp_servers().lock().map_err(|_| "lsp_servers lock".to_string())?;
        if let Some(s) = map.get(&key) {
            return Ok(Arc::clone(s));
        }
    }

    // Slow path: spawn a new server
    let server = spawn_lsp_server(command, project_path)?;
    {
        let mut map = lsp_servers().lock().map_err(|_| "lsp_servers lock".to_string())?;
        // Double-checked insert
        map.entry(key).or_insert_with(|| Arc::clone(&server));
    }
    Ok(server)
}

// ── Format helpers ─────────────────────────────────────────────────────────────

fn format_locations(result: &Value, label: &str) -> String {
    let locs: Vec<&Value> = match result {
        Value::Array(arr) if !arr.is_empty() => arr.iter().collect(),
        Value::Object(_) => vec![result],
        _ => return format!("No {label} found."),
    };

    let mut lines: Vec<String> = locs.iter().map(|loc| {
        let uri = loc.get("uri")
            .or_else(|| loc.get("targetUri"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let range = loc.get("range")
            .or_else(|| loc.get("targetRange"))
            .or_else(|| loc.get("targetSelectionRange"));
        let (ln, col) = range
            .and_then(|r| r.get("start"))
            .map(|s| (
                s["line"].as_u64().unwrap_or(0) + 1,
                s["character"].as_u64().unwrap_or(0),
            ))
            .unwrap_or((0, 0));
        format!("{}:{ln}:{col}", uri_to_path(uri))
    }).collect();
    lines.sort();
    lines.dedup();
    lines.join("\n")
}

// ── Public MCP tool functions ─────────────────────────────────────────────────

/// Open a file in its language server so diagnostics start flowing.
pub fn lsp_open_file(file: &str, project_path: &str) -> Result<String, String> {
    let server = get_or_create_server(file, project_path)?;
    server.ensure_open(file)?;
    let diag_str = server.get_diagnostics_str(file);
    if diag_str.is_empty() {
        Ok(format!("Opened {file} in LSP server. No diagnostics."))
    } else {
        Ok(format!("Opened {file} in LSP server.\n\nDiagnostics:\n{diag_str}"))
    }
}

/// Get type/doc info at a position (0-based line and character).
pub fn lsp_hover(file: &str, line: u32, character: u32, project_path: &str) -> Result<String, String> {
    let server = get_or_create_server(file, project_path)?;
    server.ensure_open(file)?;
    let result = server.send_request("textDocument/hover", json!({
        "textDocument": {"uri": path_to_uri(file)},
        "position": {"line": line, "character": character},
    }))?;
    if result.is_null() {
        return Ok("No hover information at this position.".to_string());
    }
    let contents = &result["contents"];
    let text = if let Some(v) = contents.get("value").and_then(|v| v.as_str()) {
        v.to_string()
    } else if let Some(s) = contents.as_str() {
        s.to_string()
    } else if let Some(arr) = contents.as_array() {
        arr.iter()
            .filter_map(|v| v.get("value").and_then(|v| v.as_str())
                            .or_else(|| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n---\n")
    } else {
        result.to_string()
    };
    if text.trim().is_empty() {
        Ok("No hover information at this position.".to_string())
    } else {
        Ok(text)
    }
}

/// Go to definition — returns file:line:col references.
pub fn lsp_definition(file: &str, line: u32, character: u32, project_path: &str) -> Result<String, String> {
    let server = get_or_create_server(file, project_path)?;
    server.ensure_open(file)?;
    let result = server.send_request("textDocument/definition", json!({
        "textDocument": {"uri": path_to_uri(file)},
        "position": {"line": line, "character": character},
    }))?;
    Ok(format_locations(&result, "definition"))
}

/// Find all references — returns file:line:col list.
pub fn lsp_references(
    file: &str, line: u32, character: u32,
    include_declaration: bool, project_path: &str,
) -> Result<String, String> {
    let server = get_or_create_server(file, project_path)?;
    server.ensure_open(file)?;
    let result = server.send_request("textDocument/references", json!({
        "textDocument": {"uri": path_to_uri(file)},
        "position": {"line": line, "character": character},
        "context": {"includeDeclaration": include_declaration},
    }))?;
    Ok(format_locations(&result, "references"))
}

/// Return cached diagnostics for a file (errors, warnings, hints).
pub fn lsp_diagnostics(file: &str, project_path: &str) -> Result<String, String> {
    let server = get_or_create_server(file, project_path)?;
    server.ensure_open(file)?;
    let diag_str = server.get_diagnostics_str(file);
    if diag_str.is_empty() {
        Ok(format!("No diagnostics for {file} — looks clean."))
    } else {
        Ok(diag_str)
    }
}

/// Called after a file edit: notifies the LSP, waits for fresh diagnostics,
/// returns the diagnostic string (empty if clean). Used by flamel_replace etc.
pub fn lsp_after_edit(file: &str, project_path: &str) -> String {
    // Only proceed if a server is already running for this language —
    // don't spawn one just because a file was edited.
    let lang_id = match detect_lang(file) {
        Some((l, _)) => l,
        None => return String::new(),
    };
    let key = format!("{project_path}::{lang_id}");
    let server = {
        let map = match lsp_servers().lock() {
            Ok(m) => m,
            Err(_) => return String::new(),
        };
        map.get(&key).map(Arc::clone)
    };
    let server = match server {
        Some(s) => s,
        None => return String::new(), // Server not running yet — skip
    };

    // Make sure the file is open, then notify saved
    if let Err(_) = server.ensure_open(file) {
        return String::new();
    }
    let content = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    if let Err(_) = server.notify_saved(file, &content) {
        return String::new();
    }
    server.get_diagnostics_str(file)
}
