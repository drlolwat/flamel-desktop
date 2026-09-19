use serde_json::Value;

pub fn derive_session_title(message: &str, has_images: bool) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return if has_images {
            "Image conversation".to_string()
        } else {
            "New session".to_string()
        };
    }
    let chars = normalized.chars().collect::<Vec<_>>();
    if chars.len() <= 50 {
        normalized
    } else {
        format!("{}...", chars[..47].iter().collect::<String>())
    }
}

pub fn title_from_persisted_user_message(message: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(message).ok()?;
    let content = parsed.get("content").unwrap_or(&parsed);
    let (text, has_images) = visible_content(content);
    if text.trim().is_empty() && !has_images {
        return parsed
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.trim().is_empty())
            .map(|message| derive_session_title(message, false));
    }
    Some(derive_session_title(&text, has_images))
}

fn visible_content(content: &Value) -> (String, bool) {
    match content {
        Value::String(text) => (text.clone(), false),
        Value::Array(parts) => {
            let mut text = Vec::new();
            let mut has_images = false;
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(value) = part.get("text").and_then(Value::as_str) {
                            text.push(value);
                        }
                    }
                    Some("image") | Some("image_url") => has_images = true,
                    _ => {}
                }
            }
            (text.join(" "), has_images)
        }
        _ => (String::new(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::{derive_session_title, title_from_persisted_user_message};

    #[test]
    fn derives_compact_titles() {
        assert_eq!(
            derive_session_title("  fix   the duplicate chats  ", false),
            "fix the duplicate chats"
        );
        assert_eq!(derive_session_title("", true), "Image conversation");
    }

    #[test]
    fn reads_text_from_persisted_user_content() {
        for message in [
            r#"[{"text":"Start the Replex plan","type":"text"}]"#,
            r#"{"content":[{"text":"Start the Replex plan","type":"text"}],"role":"user"}"#,
        ] {
            assert_eq!(
                title_from_persisted_user_message(message).as_deref(),
                Some("Start the Replex plan")
            );
        }
    }

    #[test]
    fn ignores_tool_results_and_recognizes_image_turns() {
        assert_eq!(
            title_from_persisted_user_message(
                r#"[{"type":"tool_result","tool_use_id":"tool-1","content":"ok"}]"#
            ),
            None
        );
        assert_eq!(
            title_from_persisted_user_message(r#"[{"type":"image","source":{}}]"#).as_deref(),
            Some("Image conversation")
        );
    }
}
