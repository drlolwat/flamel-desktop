use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEvent {
    pub event_id: String,
    pub kind: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_marker: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowEventEnvelope {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub event: WorkflowEvent,
}

pub fn workflow_event(
    kind: impl Into<String>,
    source: impl Into<String>,
    session_id: Option<&str>,
    legacy_marker: Option<&str>,
    payload: Value,
) -> WorkflowEvent {
    WorkflowEvent {
        event_id: uuid::Uuid::new_v4().to_string(),
        kind: kind.into(),
        source: source.into(),
        session_id: session_id.map(str::to_string),
        timestamp: chrono::Utc::now().to_rfc3339(),
        legacy_marker: legacy_marker.map(str::to_string),
        payload,
    }
}

pub fn workflow_event_envelope(
    kind: impl Into<String>,
    source: impl Into<String>,
    session_id: Option<&str>,
    legacy_marker: Option<&str>,
    payload: Value,
) -> WorkflowEventEnvelope {
    WorkflowEventEnvelope {
        event_type: "workflow_event",
        event: workflow_event(kind, source, session_id, legacy_marker, payload),
    }
}

pub fn workflow_event_line(
    kind: impl Into<String>,
    source: impl Into<String>,
    session_id: Option<&str>,
    legacy_marker: Option<&str>,
    payload: Value,
) -> String {
    format!(
        "{}\n",
        json!(workflow_event_envelope(
            kind,
            source,
            session_id,
            legacy_marker,
            payload
        ))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_event_line_serializes_camel_case_envelope() {
        let line = workflow_event_line(
            "system.compacting",
            "chat_stream",
            Some("session-1"),
            Some("system_status"),
            json!({ "status": "compacting" }),
        );
        let parsed: Value = serde_json::from_str(line.trim()).expect("workflow event json");
        assert_eq!(parsed["type"], "workflow_event");
        assert_eq!(parsed["event"]["kind"], "system.compacting");
        assert_eq!(parsed["event"]["source"], "chat_stream");
        assert_eq!(parsed["event"]["sessionId"], "session-1");
        assert_eq!(parsed["event"]["legacyMarker"], "system_status");
        assert_eq!(parsed["event"]["payload"]["status"], "compacting");
        assert!(parsed["event"]["eventId"].as_str().is_some());
        assert!(parsed["event"]["timestamp"].as_str().is_some());
    }

    #[test]
    fn workflow_event_line_omits_optional_fields() {
        let line = workflow_event_line(
            "command.event",
            "command_event",
            None,
            None,
            json!({ "id": "event-1" }),
        );
        let parsed: Value = serde_json::from_str(line.trim()).expect("workflow event json");
        assert!(parsed["event"].get("sessionId").is_none());
        assert!(parsed["event"].get("legacyMarker").is_none());
    }
}
