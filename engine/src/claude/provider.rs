//! Provider abstraction for multi-CLI support (Claude Code CLI, Codex CLI, etc.)
//!
//! Each provider knows how to:
//! - Find and spawn its CLI binary
//! - Format stdin messages (stream-json vs plain text)
//! - Parse stdout events into a normalized format
//! - Manage credentials (OAuth tokens → CLI credential files)
//! - Configure MCP servers for the CLI

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// ProviderKind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Claude,
    Codex,
}

const BROWSER_AUTOMATION_ENV: [(&str, &str); 6] = [
    ("PUPPETEER_SKIP_DOWNLOAD", "true"),
    ("PUPPETEER_SKIP_CHROMIUM_DOWNLOAD", "true"),
    ("PUPPETEER_EXECUTABLE_PATH", "/usr/local/bin/google-chrome"),
    ("CHROME_BIN", "/usr/local/bin/google-chrome"),
    ("PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD", "1"),
    ("NODE_PATH", "/usr/lib/node_modules"),
];

pub fn add_browser_automation_env(env: &mut Vec<(String, String)>) {
    for (key, value) in BROWSER_AUTOMATION_ENV {
        if !env.iter().any(|(k, v)| k == key && !v.is_empty()) {
            env.push((key.to_string(), value.to_string()));
        }
    }
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Claude => "claude",
            ProviderKind::Codex => "codex",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(ProviderKind::Claude),
            "codex" => Some(ProviderKind::Codex),
            _ => None,
        }
    }

    /// Infer provider from a model ID string.
    pub fn from_model(model: &str) -> Self {
        if model.starts_with("claude-") {
            ProviderKind::Claude
        } else if model.starts_with("gpt-") || model.starts_with("o") {
            ProviderKind::Codex
        } else {
            // Unknown model identifiers must never silently select an unrelated
            // provider. Codex is the local default; request routing still checks
            // that it is explicitly authenticated before starting a session.
            ProviderKind::Codex
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// SpawnOpts — provider-agnostic options passed to build_args / build_env
// ---------------------------------------------------------------------------

pub struct SpawnOpts<'a> {
    pub model: Option<&'a str>,
    pub reasoning_effort: Option<&'a str>,
    pub permission_mode: &'a str,
    pub system_prompt_append: Option<&'a str>,
    pub resume_session_id: Option<&'a str>,
    pub mcp_servers: Option<&'a Value>,
    pub user_home: &'a str,
    pub session_id: &'a str,
    /// Optional provider API base URL used only to build launch-time CLI config.
    /// Capability values stay in process environment and must never be embedded here.
    pub provider_base_url: Option<&'a str>,
    /// User message — providers that take it as a CLI arg (Codex) read it here.
    /// Claude ignores this field (uses stdin instead).
    pub message: Option<&'a str>,
}

// ---------------------------------------------------------------------------
// ProviderCredentials — generic credential struct for all providers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ProviderCredentials {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix timestamp (seconds) when access_token expires
    pub expires_at: i64,
    pub scopes: String,
    /// Provider-specific extra data (e.g. id_token for Codex)
    pub extra: Option<Value>,
}

// ---------------------------------------------------------------------------
// NormalizedEvent — the common event type both providers map onto
// ---------------------------------------------------------------------------

/// Events normalized from provider-specific CLI output into a common format.
/// The session pool broadcasts these; chat.rs re-serializes them to Claude's
/// SSE format for the frontend (so the frontend needs zero changes).
const TOOL_PRESENTATION_TEXT_BYTES: usize = 16 * 1024;
const TOOL_PRESENTATION_STRUCTURED_BYTES: usize = 48 * 1024;
const TOOL_PRESENTATION_ITEMS: usize = 64;
const TOOL_PRESENTATION_STRING_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolResourceLink {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolMediaReference {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultEnvelope {
    pub version: u8,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<ToolResourceLink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<ToolMediaReference>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn bounded_tool_string(value: Option<&str>, max_bytes: usize) -> Option<String> {
    value
        .filter(|value| !value.is_empty())
        .map(|value| truncate_utf8(value, max_bytes))
}

fn encoded_tool_payload(key: &str, value: &str) -> bool {
    matches!(key.to_ascii_lowercase().as_str(), "data" | "blob")
        || (value.to_ascii_lowercase().starts_with("data:")
            && value[..value.len().min(256)]
                .to_ascii_lowercase()
                .contains(";base64,"))
}

fn safe_tool_resource_uri(value: Option<&str>) -> Option<String> {
    let value = value?;
    if value.is_empty() || value.len() > 2_048 || value.chars().any(char::is_control) {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("data:")
        || lower.starts_with("vbscript:")
    {
        return None;
    }
    if value.starts_with('/')
        || [
            "http://",
            "https://",
            "file://",
            "flamel://",
            "artifact://",
            "mcp://",
            "skill://",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        Some(value.to_string())
    } else {
        None
    }
}

fn sanitize_tool_value(value: &Value, truncated: &mut bool, depth: usize, key: &str) -> Value {
    if depth > 8 {
        *truncated = true;
        return Value::String("[depth limit]".to_string());
    }
    match value {
        Value::String(value) => {
            if encoded_tool_payload(key, value) {
                *truncated = true;
                Value::String("[binary payload omitted]".to_string())
            } else if value.len() > TOOL_PRESENTATION_STRING_BYTES {
                *truncated = true;
                Value::String(truncate_utf8(value, TOOL_PRESENTATION_STRING_BYTES))
            } else {
                Value::String(value.clone())
            }
        }
        Value::Array(values) => {
            if values.len() > TOOL_PRESENTATION_ITEMS {
                *truncated = true;
            }
            Value::Array(
                values
                    .iter()
                    .take(TOOL_PRESENTATION_ITEMS)
                    .map(|value| sanitize_tool_value(value, truncated, depth + 1, ""))
                    .collect(),
            )
        }
        Value::Object(values) => {
            if values.len() > TOOL_PRESENTATION_ITEMS {
                *truncated = true;
            }
            Value::Object(
                values
                    .iter()
                    .take(TOOL_PRESENTATION_ITEMS)
                    .map(|(key, value)| {
                        (
                            key.clone(),
                            sanitize_tool_value(value, truncated, depth + 1, key),
                        )
                    })
                    .collect(),
            )
        }
        _ => value.clone(),
    }
}

fn tool_resource_link(value: &Value) -> Option<ToolResourceLink> {
    let uri = safe_tool_resource_uri(value.get("uri").and_then(Value::as_str))?;
    Some(ToolResourceLink {
        uri,
        name: bounded_tool_string(value.get("name").and_then(Value::as_str), 256),
        title: bounded_tool_string(value.get("title").and_then(Value::as_str), 256),
        description: bounded_tool_string(value.get("description").and_then(Value::as_str), 1_024),
        media_type: bounded_tool_string(
            value
                .get("mimeType")
                .or_else(|| value.get("mime_type"))
                .and_then(Value::as_str),
            128,
        ),
        size: value.get("size").and_then(Value::as_u64),
    })
}

fn tool_media_reference(value: &Value) -> Option<ToolMediaReference> {
    let kind = value
        .get("kind")
        .or_else(|| value.get("type"))
        .and_then(Value::as_str)?;
    if !matches!(kind, "image" | "audio") {
        return None;
    }
    Some(ToolMediaReference {
        kind: kind.to_string(),
        media_type: bounded_tool_string(
            value
                .get("mediaType")
                .or_else(|| value.get("mimeType"))
                .or_else(|| value.get("mime_type"))
                .and_then(Value::as_str),
            128,
        ),
        resource_uri: safe_tool_resource_uri(
            value
                .get("resourceUri")
                .or_else(|| value.get("resource_uri"))
                .or_else(|| value.get("uri"))
                .and_then(Value::as_str),
        ),
        alt: bounded_tool_string(value.get("alt").and_then(Value::as_str), 512),
    })
}

fn safe_tool_block_text(block: &Value, truncated: &mut bool) -> String {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => block
            .get("text")
            .and_then(Value::as_str)
            .map(|text| {
                if encoded_tool_payload("text", text) {
                    *truncated = true;
                    "[binary payload omitted]".to_string()
                } else {
                    text.to_string()
                }
            })
            .unwrap_or_default(),
        Some("image") | Some("audio") => serde_json::json!({
            "type": block.get("type"),
            "mimeType": block
                .get("mediaType")
                .or_else(|| block.get("mimeType"))
                .or_else(|| block.get("mime_type")),
            "delivered_to_model": true,
        })
        .to_string(),
        Some("resource_link") => tool_resource_link(block)
            .and_then(|resource| serde_json::to_string(&resource).ok())
            .unwrap_or_default(),
        Some("resource") => {
            let Some(resource) = block.get("resource") else {
                return String::new();
            };
            let mut parts = Vec::new();
            if let Some(link) = tool_resource_link(resource)
                && let Ok(serialized) = serde_json::to_string(&link)
            {
                parts.push(serialized);
            }
            if let Some(text) = resource.get("text").and_then(Value::as_str) {
                parts.push(text.to_string());
            }
            if resource.get("blob").is_some() {
                *truncated = true;
            }
            parts.join("\n")
        }
        _ => {
            serde_json::to_string(&sanitize_tool_value(block, truncated, 0, "")).unwrap_or_default()
        }
    }
}

pub fn normalize_tool_result(result: &Value) -> (String, ToolResultEnvelope) {
    let mut truncated = false;
    let output = if let Some(text) = result.as_str() {
        if encoded_tool_payload("", text) {
            truncated = true;
            "[binary payload omitted]".to_string()
        } else {
            text.to_string()
        }
    } else if let Some(content) = result.get("content").and_then(Value::as_array) {
        content
            .iter()
            .map(|block| safe_tool_block_text(block, &mut truncated))
            .collect::<Vec<_>>()
            .join("")
    } else {
        serde_json::to_string(&sanitize_tool_value(result, &mut truncated, 0, ""))
            .unwrap_or_default()
    };

    let structured_candidate = result
        .get("structuredContent")
        .or_else(|| result.get("structured_content"));
    let structured_content = structured_candidate.and_then(|value| {
        let sanitized = sanitize_tool_value(value, &mut truncated, 0, "");
        match serde_json::to_vec(&sanitized) {
            Ok(bytes) if bytes.len() <= TOOL_PRESENTATION_STRUCTURED_BYTES => Some(sanitized),
            _ => {
                truncated = true;
                None
            }
        }
    });

    let mut resources = Vec::new();
    let mut media = Vec::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for block in content.iter().take(TOOL_PRESENTATION_ITEMS) {
            match block.get("type").and_then(Value::as_str) {
                Some("resource_link") => {
                    if let Some(resource) = tool_resource_link(block)
                        && !resources
                            .iter()
                            .any(|existing: &ToolResourceLink| existing.uri == resource.uri)
                    {
                        resources.push(resource);
                    }
                }
                Some("resource") => {
                    if let Some(resource) = block.get("resource").and_then(tool_resource_link)
                        && !resources
                            .iter()
                            .any(|existing: &ToolResourceLink| existing.uri == resource.uri)
                    {
                        resources.push(resource);
                    }
                }
                Some("image") | Some("audio") => {
                    if let Some(reference) = tool_media_reference(block) {
                        media.push(reference);
                    }
                }
                _ => {}
            }
        }
        if content.len() > TOOL_PRESENTATION_ITEMS {
            truncated = true;
        }
    }

    let text = truncate_utf8(&output, TOOL_PRESENTATION_TEXT_BYTES);
    if text.len() < output.len() {
        truncated = true;
    }
    (
        output,
        ToolResultEnvelope {
            version: 1,
            text,
            structured_content,
            resources,
            media,
            truncated,
        },
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event_type")]
pub enum NormalizedEvent {
    /// CLI initialized, session ID available for resume
    Init { session_id: String },

    /// Text content from the assistant (may be partial or complete)
    AssistantText {
        text: String,
        /// If true, this is a complete message (not a delta)
        complete: bool,
    },

    /// Extended thinking / reasoning content
    Thinking { text: String },

    /// Model is reasoning privately; only expose an activity marker.
    ThinkingRedacted,

    /// Model is using a tool
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },

    /// Tool execution result
    ToolResult {
        tool_use_id: String,
        output: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        presentation: Option<ToolResultEnvelope>,
        is_error: bool,
    },

    /// Incremental output from a still-running tool. Clients must append this
    /// without completing the tool or splitting the assistant turn.
    ToolOutputDelta { tool_use_id: String, output: String },

    /// MCP tool call (Codex-specific, maps to ToolUse + ToolResult)
    McpToolCall {
        server: String,
        tool: String,
        arguments: Value,
        result: Option<Value>,
        error: Option<String>,
        status: String,
        id: Option<String>,
    },

    /// Low-level streaming delta (Claude-specific, forwarded verbatim)
    StreamEvent { raw: Value },

    /// Turn/session complete with usage stats
    Result {
        session_id: Option<String>,
        usage: Option<TokenUsage>,
        stop_reason: Option<String>,
        is_error: bool,
        error_message: Option<String>,
    },

    /// Provider-reported cumulative context usage. Codex app-server sends this
    /// independently from turn completion, including while a turn is running.
    Usage {
        usage: TokenUsage,
        context_window: Option<i64>,
    },

    /// Non-fatal error
    Error { message: String },

    /// Context compaction in progress (Claude-specific)
    Compacting,

    /// Raw passthrough — event not recognized, forward as-is
    Raw { data: Value },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: Option<i64>,
}

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

pub trait Provider: Send + Sync {
    /// Which provider this is
    fn kind(&self) -> ProviderKind;

    /// Locate the CLI binary on disk
    fn find_binary(&self) -> Result<String, String>;

    /// Build CLI arguments for spawning
    fn build_args(&self, opts: &SpawnOpts) -> Vec<String>;

    /// Build environment variables for the subprocess.
    /// Includes credential injection (writing credential files, setting env vars).
    /// `mcp_servers` is the mcpServers JSON object — providers that configure MCP
    /// via config files (Codex) write the config here rather than in build_args.
    fn build_env(
        &self,
        creds: Option<&ProviderCredentials>,
        db: Option<&crate::db::Db>,
        user_home: &str,
        mcp_servers: Option<&Value>,
        reasoning_effort: Option<&str>,
    ) -> Vec<(String, String)>;

    /// Format a user message for stdin.
    /// Claude: JSON `{"type":"user","message":{...}}`
    /// Codex: plain text
    fn format_input(&self, message: &str, images: &[ImageBlock]) -> Vec<u8>;

    /// Parse a single stdout line into a normalized event.
    /// Returns None if the line should be ignored.
    fn parse_event(&self, line: &str) -> Option<NormalizedEvent>;

    /// Re-serialize a NormalizedEvent back into the provider-specific JSON format
    /// that the frontend expects (Claude format for SSE).
    fn serialize_for_frontend(&self, event: &NormalizedEvent) -> Option<Value>;

    /// CSV list of built-in tools to disable via --disallowedTools or equivalent
    fn disallowed_tools_csv(&self) -> String;

    /// Default model ID when none specified
    fn default_model(&self) -> &str;
}

/// Image content block for multi-modal input
#[derive(Debug, Clone)]
pub struct ImageBlock {
    pub media_type: String,
    pub data: String, // base64
}

// ---------------------------------------------------------------------------
// Provider registry — get a provider instance by kind
// ---------------------------------------------------------------------------

pub fn get_provider(kind: ProviderKind) -> Box<dyn Provider> {
    match kind {
        ProviderKind::Claude => Box::new(super::claude_provider::ClaudeProvider),
        ProviderKind::Codex => Box::new(super::codex_provider::CodexProvider::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_or_missing_legacy_models_never_select_claude() {
        assert_eq!(ProviderKind::from_model(""), ProviderKind::Codex);
        assert_eq!(ProviderKind::from_model("auto"), ProviderKind::Codex);
        assert_eq!(
            ProviderKind::from_model("future-model"),
            ProviderKind::Codex
        );
        assert_eq!(
            ProviderKind::from_model("claude-sonnet-4-6"),
            ProviderKind::Claude
        );
    }

    #[test]
    fn normalized_tool_results_keep_safe_metadata_without_inline_binary() {
        let encoded = "sensitive-base64-payload";
        let (output, envelope) = normalize_tool_result(&serde_json::json!({
            "content": [
                {"type": "text", "text": "Rendered preview"},
                {
                    "type": "image",
                    "mimeType": "image/webp",
                    "resourceUri": "artifact://render/preview.webp",
                    "data": encoded
                },
                {
                    "type": "resource_link",
                    "uri": "artifact://render/preview.webp",
                    "title": "Preview"
                },
                {
                    "type": "resource_link",
                    "uri": "javascript:alert(1)",
                    "title": "Unsafe"
                }
            ],
            "structuredContent": {
                "dimensions": {"width": 1280, "height": 720},
                "blob": encoded
            }
        }));

        let serialized = serde_json::to_string(&envelope).expect("serialize envelope");
        assert!(output.contains("Rendered preview"));
        assert!(!output.contains(encoded));
        assert!(!serialized.contains(encoded));
        assert_eq!(envelope.version, 1);
        assert_eq!(envelope.resources.len(), 1);
        assert_eq!(envelope.resources[0].uri, "artifact://render/preview.webp");
        assert_eq!(envelope.media.len(), 1);
        assert_eq!(
            envelope.media[0].resource_uri.as_deref(),
            Some("artifact://render/preview.webp")
        );
        assert_eq!(
            envelope.structured_content.as_ref().expect("structured")["blob"],
            serde_json::json!("[binary payload omitted]")
        );
        assert!(envelope.truncated);
    }

    #[test]
    fn normalized_tool_results_bound_text_collections_and_structured_size() {
        let oversized = Value::Array(
            (0..80)
                .map(|index| Value::String(format!("{index}:{}", "x".repeat(2_000))))
                .collect(),
        );
        let (output, envelope) = normalize_tool_result(&serde_json::json!({
            "content": [{"type": "text", "text": "y".repeat(20_000)}],
            "structuredContent": oversized,
        }));

        assert_eq!(output.len(), 20_000);
        assert!(envelope.text.len() <= TOOL_PRESENTATION_TEXT_BYTES);
        assert!(envelope.structured_content.is_none());
        assert!(envelope.truncated);
    }
}
