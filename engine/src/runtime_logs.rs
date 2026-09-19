use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const MAX_RUNTIME_LOGS: usize = 2_000;
const MAX_FIELD_CHARS: usize = 2_000;

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeLogField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeLogEntry {
    pub sequence: u64,
    pub timestamp: String,
    pub level: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<RuntimeLogField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

struct RuntimeLogState {
    next_sequence: u64,
    entries: VecDeque<RuntimeLogEntry>,
}

static RUNTIME_LOGS: OnceLock<Mutex<RuntimeLogState>> = OnceLock::new();

pub struct RuntimeLogLayer;

pub fn layer() -> RuntimeLogLayer {
    RuntimeLogLayer
}

pub fn recent_logs(limit: usize, min_level: Option<&str>) -> Vec<RuntimeLogEntry> {
    let limit = limit.clamp(1, 200);
    let min_rank = min_level.and_then(level_rank);
    let Ok(state) = store().lock() else {
        return Vec::new();
    };
    let mut entries: Vec<_> = state
        .entries
        .iter()
        .rev()
        .filter(|entry| {
            min_rank
                .map(|rank| level_rank(&entry.level).unwrap_or(0) >= rank)
                .unwrap_or(true)
        })
        .take(limit)
        .cloned()
        .collect();
    entries.reverse();
    entries
}

pub fn counts_by_level() -> BTreeMap<String, usize> {
    let Ok(state) = store().lock() else {
        return BTreeMap::new();
    };
    let mut counts = BTreeMap::new();
    for entry in &state.entries {
        *counts.entry(entry.level.clone()).or_insert(0) += 1;
    }
    counts
}

fn store() -> &'static Mutex<RuntimeLogState> {
    RUNTIME_LOGS.get_or_init(|| {
        Mutex::new(RuntimeLogState {
            next_sequence: 1,
            entries: VecDeque::new(),
        })
    })
}

impl<S> Layer<S> for RuntimeLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = RuntimeLogVisitor::default();
        event.record(&mut visitor);
        let metadata = event.metadata();
        let entry = RuntimeLogEntry {
            sequence: 0,
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: metadata.level().as_str().to_string(),
            target: metadata.target().to_string(),
            message: visitor.message,
            fields: visitor.fields,
            file: metadata.file().map(str::to_string),
            line: metadata.line(),
        };
        push_log(entry);
    }
}

fn push_log(mut entry: RuntimeLogEntry) {
    let Ok(mut state) = store().lock() else {
        return;
    };
    entry.sequence = state.next_sequence;
    state.next_sequence = state.next_sequence.saturating_add(1);
    state.entries.push_back(entry);
    while state.entries.len() > MAX_RUNTIME_LOGS {
        state.entries.pop_front();
    }
}

#[derive(Default)]
struct RuntimeLogVisitor {
    message: Option<String>,
    fields: Vec<RuntimeLogField>,
}

impl RuntimeLogVisitor {
    fn record_value(&mut self, field: &Field, value: String) {
        let name = field.name();
        let value = sanitize_log_value(name, &value);
        if name == "message" {
            self.message = Some(value);
        } else {
            self.fields.push(RuntimeLogField {
                name: name.to_string(),
                value,
            });
        }
    }
}

impl Visit for RuntimeLogVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record_value(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_value(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_value(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_value(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_value(field, value.to_string());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.record_value(field, value.to_string());
    }
}

fn sanitize_log_value(_name: &str, value: &str) -> String {
    truncate_chars(value.replace(['\r', '\n'], " "), MAX_FIELD_CHARS)
}

fn truncate_chars(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value;
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...[truncated]");
    truncated
}

fn level_rank(level: &str) -> Option<u8> {
    match level.to_ascii_uppercase().as_str() {
        "TRACE" => Some(1),
        "DEBUG" => Some(2),
        "INFO" => Some(3),
        "WARN" => Some(4),
        "ERROR" => Some(5),
        _ => None,
    }
}

pub fn sanitize_diagnostic_text(value: &str) -> String {
    sanitize_log_value("diagnostic", value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_user_supplied_values_in_runtime_logs() {
        assert_eq!(
            sanitize_log_value("authorization", "Bearer abc"),
            "Bearer abc"
        );
        assert_eq!(
            sanitize_log_value("message", "access_token=abc"),
            "access_token=abc"
        );
    }

    #[test]
    fn level_filter_orders_by_severity() {
        assert!(level_rank("ERROR") > level_rank("WARN"));
        assert!(level_rank("INFO") > level_rank("DEBUG"));
    }
}
