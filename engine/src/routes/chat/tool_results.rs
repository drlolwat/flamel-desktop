use serde_json::{Value, json};

/// One envelope for live provider results and persisted history. Output chunks
/// are progress, not evidence that a command has finished.
pub(super) fn tool_result_is_terminal(result: &Value) -> bool {
    result.get("is_delta").and_then(Value::as_bool) != Some(true)
}

pub(super) fn tool_result_message(result: &Value) -> Value {
    let mut message = json!({
        "type": "tool_result",
        "tool_use_id": result.get("tool_use_id"),
        "content": result.get("content"),
        "is_error": result.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        "is_delta": !tool_result_is_terminal(result),
        "same_turn": true,
    });
    for key in ["presentation", "artifacts"] {
        if let Some(value) = result.get(key) {
            message[key] = value.clone();
        }
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_does_not_complete_a_tool() {
        let result = json!({"tool_use_id":"parallel-a", "is_delta":true, "content":"working"});
        assert!(!tool_result_is_terminal(&result));
        assert_eq!(tool_result_message(&result)["is_delta"], true);
    }

    #[test]
    fn terminal_result_keeps_structured_presentation_and_artifacts() {
        let result = json!({"tool_use_id":"parallel-b", "content":"done", "is_error":true,
            "presentation":{"schema":"flamel.result.v1", "structuredContent":{"files":["a.rs"]}},
            "artifacts":[{"id":"image"}]});
        let message = tool_result_message(&result);
        assert!(tool_result_is_terminal(&result));
        assert_eq!(message["presentation"], result["presentation"]);
        assert_eq!(message["artifacts"], result["artifacts"]);
        assert_eq!(message["is_error"], true);
        assert_eq!(message["tool_use_id"], "parallel-b");
    }
}
