use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HiddenControlDirective {
    Continue,
    Complete,
}

impl HiddenControlDirective {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Complete => "complete",
        }
    }
}

pub(super) fn parse_hidden_control_tag(tag: &str) -> Option<HiddenControlDirective> {
    let normalized = tag
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    match normalized.as_str() {
        "<flamel:continue/>" => Some(HiddenControlDirective::Continue),
        "<flamel:complete/>" => Some(HiddenControlDirective::Complete),
        _ => None,
    }
}

pub(super) fn strip_hidden_control_markers(text: &str) -> (String, Option<HiddenControlDirective>) {
    let mut cleaned = String::with_capacity(text.len());
    let mut directive = None;
    let mut cursor = 0;

    while let Some(rel_start) = text[cursor..].find("<flamel:") {
        let start = cursor + rel_start;
        cleaned.push_str(&text[cursor..start]);

        let remainder = &text[start..];
        let Some(rel_end) = remainder.find('>') else {
            cursor = text.len();
            break;
        };
        let end = start + rel_end + 1;
        if let Some(tag_directive) = parse_hidden_control_tag(&text[start..end]) {
            directive = Some(tag_directive);
        }
        cursor = end;
    }

    cleaned.push_str(&text[cursor..]);
    (cleaned, directive)
}

pub(super) fn sanitize_content_blocks_and_extract_directive(
    content: &[Value],
) -> (Vec<Value>, Option<HiddenControlDirective>) {
    let mut sanitized = Vec::with_capacity(content.len());
    let mut directive = None;

    for block in content {
        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
            let original = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
            let (text, block_directive) = strip_hidden_control_markers(original);
            if let Some(block_directive) = block_directive {
                directive = Some(block_directive);
            }
            if text.is_empty() {
                continue;
            }
            let mut sanitized_block = block.clone();
            sanitized_block["text"] = Value::String(text);
            sanitized.push(sanitized_block);
        } else {
            let mut sanitized_block = block.clone();
            redact_sensitive_tool_input(&mut sanitized_block);
            sanitized.push(sanitized_block);
        }
    }

    (sanitized, directive)
}

fn redact_sensitive_tool_input(block: &mut Value) {
    if block.get("type").and_then(Value::as_str) != Some("tool_use") {
        return;
    }
    let tool_name = block.get("name").and_then(Value::as_str).unwrap_or("");
    let is_shell = matches!(tool_name, "shell" | "flamel_shell" | "shell_write")
        || tool_name.ends_with("__shell");
    if !is_shell {
        return;
    }

    if let Some(partial) = block.get("_inputStr").and_then(Value::as_str)
        && (partial.contains("\"action\":\"store_secret\"")
            || partial.contains("\"action\": \"store_secret\"")
            || partial.contains("\"sensitive\":true")
            || partial.contains("\"sensitive\": true"))
    {
        block["_inputStr"] = Value::String("[redacted sensitive shell input]".into());
    }

    let Some(input) = block.get_mut("input").and_then(Value::as_object_mut) else {
        return;
    };
    let action = input.get("action").and_then(Value::as_str).unwrap_or("");
    let sensitive_write = action == "write"
        && input
            .get("sensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    if (action == "store_secret" || sensitive_write) && input.contains_key("text") {
        input.insert("text".into(), Value::String("[redacted]".into()));
    }
}

pub(super) fn extract_text_from_content_blocks(content: &[Value]) -> String {
    content
        .iter()
        .filter_map(|block| {
            (block.get("type").and_then(|t| t.as_str()) == Some("text"))
                .then(|| block.get("text").and_then(|t| t.as_str()).unwrap_or(""))
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn content_block_id(block: &Value) -> Option<&str> {
    block
        .get("id")
        .or_else(|| block.get("tool_use_id"))
        .and_then(|id| id.as_str())
}

pub(super) fn content_block_type(block: &Value) -> Option<&str> {
    block.get("type").and_then(|t| t.as_str())
}

pub(super) fn merge_codex_assistant_content(existing: &mut Vec<Value>, incoming: &[Value]) {
    if incoming.is_empty() {
        return;
    }

    // Multi-block Codex snapshots are canonical. Single-block snapshots can be item-level
    // deltas from app-server/runner paths, so merge them without dropping prior tool blocks.
    if incoming.len() > 1 || existing.is_empty() {
        *existing = incoming.to_vec();
        return;
    }

    let incoming_block = incoming[0].clone();
    if let Some(incoming_id) = content_block_id(&incoming_block) {
        if let Some(pos) = existing
            .iter()
            .position(|block| content_block_id(block) == Some(incoming_id))
        {
            existing[pos] = incoming_block;
            return;
        }
    }

    match content_block_type(&incoming_block) {
        Some("text") => {
            if let Some(last_block) = existing.last_mut() {
                if content_block_type(last_block) == Some("text") {
                    let existing_text = last_block
                        .get("text")
                        .and_then(|text| text.as_str())
                        .unwrap_or("");
                    let incoming_text = incoming_block
                        .get("text")
                        .and_then(|text| text.as_str())
                        .unwrap_or("");
                    if incoming_text.starts_with(existing_text)
                        || existing_text.is_empty()
                        || existing_text.starts_with(incoming_text)
                    {
                        *last_block = incoming_block;
                    } else {
                        existing.push(incoming_block);
                    }
                    return;
                }
            }
            existing.push(incoming_block);
        }
        _ => existing.push(incoming_block),
    }
}

pub(super) fn looks_like_unverified_progress_status_line(content: &str) -> bool {
    let normalized = content.trim().to_lowercase();
    if normalized.is_empty() || normalized.ends_with('?') {
        return false;
    }

    normalized.starts_with("concrete check ")
        || normalized.ends_with(" is the current item.")
        || normalized.ends_with(" are the current item.")
        || normalized.ends_with(" is verified.")
        || normalized.ends_with(" are verified.")
}

pub(super) fn codex_text_block_should_be_hidden(text: &str) -> bool {
    let progress_lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    !progress_lines.is_empty()
        && progress_lines.iter().all(|line| {
            looks_like_partial_progress_recap(line)
                || looks_like_unverified_progress_status_line(line)
        })
}

pub(super) fn codex_content_without_unverified_progress_text(
    content: &[Value],
    has_unfinished_plan: bool,
) -> Vec<Value> {
    if !has_unfinished_plan {
        return content.to_vec();
    }

    content
        .iter()
        .filter(|block| {
            if block.get("type").and_then(|t| t.as_str()) != Some("text") {
                return true;
            }
            let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
            !codex_text_block_should_be_hidden(text)
        })
        .cloned()
        .collect()
}

pub(super) fn codex_content_without_rejected_native_plan_text(
    content: &[Value],
    native_plan_rejected: bool,
) -> Vec<Value> {
    if !native_plan_rejected {
        return content.to_vec();
    }

    content
        .iter()
        .filter(|block| block.get("type").and_then(|t| t.as_str()) != Some("text"))
        .cloned()
        .collect()
}

pub(super) fn looks_like_stall_recap(content: &str) -> bool {
    let normalized = content.trim().to_lowercase();
    if normalized.is_empty() || normalized.ends_with('?') {
        return false;
    }

    let stall_actions = [
        "i'm checking",
        "i am checking",
        "checking the",
        "i'm loading",
        "i am loading",
        "loading the",
        "i'm reading",
        "i am reading",
        "reading the",
        "i'm tracing",
        "i am tracing",
        "tracing the",
        "i'm looking at",
        "i am looking at",
        "i'm going to",
        "i am going to",
        "going to continue",
        "will continue",
        "continuing inline",
        "retrying the",
        "falling back to",
        "re-reading the",
        "reopening the",
        "pulling the",
        "reviewing the",
        "scanning the",
    ];
    let stall_targets = [
        "active plan",
        "plan panel",
        "plan items",
        "plan service",
        "plan tool",
        "repo state",
        "current repo",
        "repo contents",
        "existing code",
        "codebase",
        "entrypoints",
        "project state",
        "implementation inline",
        "cleanup",
    ];

    stall_actions
        .iter()
        .any(|marker| normalized.contains(marker))
        && stall_targets
            .iter()
            .any(|marker| normalized.contains(marker))
}

pub(super) fn looks_like_partial_progress_recap(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() || trimmed.ends_with('?') {
        return false;
    }

    let normalized = trimmed.to_lowercase();
    if looks_like_stall_recap(trimmed) || looks_like_unverified_progress_status_line(trimmed) {
        return true;
    }

    let blocking_markers = [
        "full plan is in the plan panel",
        "wait for explicit user approval",
        "need your approval",
        "please choose",
        "what would you like",
        "do you want",
        "would you like",
        "can you confirm",
        "which option",
    ];
    if blocking_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return false;
    }

    let status_only_markers = [
        "done",
        "done.",
        "all done",
        "completed the task",
        "finished the task",
        "implemented everything",
        "all plan items are done",
        "checking ",
        "verifying ",
        "reviewing ",
        "moving on ",
        "continuing ",
    ];
    if status_only_markers
        .iter()
        .any(|marker| normalized == *marker || normalized.starts_with(marker))
    {
        return true;
    }

    let progress_markers = [
        "implemented ",
        "completed ",
        "finished ",
        "verified ",
        "fixed ",
        "added ",
        "updated ",
        "wrote ",
        "ran ",
        "passes",
        "passed",
        "are in place",
        "i also fixed",
    ];
    let recap_markers = [
        "plan item",
        "test suite",
        "integrations",
        "coverage",
        "unittest",
        "vitest",
        "sdk",
        "verified them",
    ];
    let unfinished_markers = [
        "remaining",
        "still need",
        "next i",
        "next up",
        "moving on to",
        "continuing with",
        "resume with",
        "left to do",
    ];

    let has_progress_marker = progress_markers
        .iter()
        .any(|marker| normalized.contains(marker));
    let has_recap_marker = recap_markers
        .iter()
        .any(|marker| normalized.contains(marker));
    let has_unfinished_marker = unfinished_markers
        .iter()
        .any(|marker| normalized.contains(marker));

    has_progress_marker && (has_recap_marker || has_unfinished_marker)
}

pub(super) fn is_tool_result_only_user_message(content: &[Value]) -> bool {
    !content.is_empty()
        && content
            .iter()
            .all(|block| block.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
}
