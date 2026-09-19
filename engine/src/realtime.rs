use crate::db::{Db, realtime::SessionEvent};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, broadcast};

const TICKET_TTL: Duration = Duration::from_secs(60);
const EVENT_BROADCAST_CAPACITY: usize = 16_384;
const SNAPSHOT_JOURNAL_INTERVAL: Duration = Duration::from_millis(100);

fn durable_event_kind(payload: &serde_json::Value) -> &str {
    let kind = payload
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("event");
    if kind == "message"
        && payload
            .get("message")
            .and_then(|message| message.get("type"))
            .and_then(|value| value.as_str())
            == Some("tool_result")
    {
        "tool_result"
    } else {
        kind
    }
}

#[derive(Clone)]
struct RealtimeTicket {
    user_id: String,
    expires_at: Instant,
}

/// Durable session-event journal plus a low-latency notification channel.
///
/// The database is authoritative. The broadcast only wakes connected clients;
/// reconnecting or lagged clients fill every gap from `session_events`.
pub struct RealtimeHub {
    db: Arc<Db>,
    events: broadcast::Sender<SessionEvent>,
    tickets: Mutex<HashMap<String, RealtimeTicket>>,
}

impl RealtimeHub {
    pub fn new(db: Arc<Db>) -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENT_BROADCAST_CAPACITY);
        Arc::new(Self {
            db,
            events,
            tickets: Mutex::new(HashMap::new()),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    pub fn publish_value(
        &self,
        session_id: &str,
        run_id: Option<&str>,
        payload: &serde_json::Value,
    ) -> Option<SessionEvent> {
        let kind = durable_event_kind(payload);
        let event = crate::db::realtime::append_event(&self.db, session_id, run_id, kind, payload)?;
        let _ = self.events.send(event.clone());
        Some(event)
    }

    pub fn publish_line(
        &self,
        session_id: &str,
        run_id: Option<&str>,
        line: &str,
    ) -> Option<SessionEvent> {
        let payload = serde_json::from_str::<serde_json::Value>(line.trim()).ok()?;
        self.publish_value(session_id, run_id, &payload)
    }

    /// Bridges current producers while the legacy NDJSON transport remains
    /// available. This task owns a receiver, so a browser disconnect cannot stop
    /// event persistence or provider execution.
    pub fn attach_legacy_stream(
        self: &Arc<Self>,
        session_id: String,
        run_id: Option<String>,
        mut receiver: broadcast::Receiver<String>,
    ) {
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            let mut pending_snapshot: Option<serde_json::Value> = None;
            let mut last_snapshot_at = Instant::now()
                .checked_sub(SNAPSHOT_JOURNAL_INTERVAL)
                .unwrap_or_else(Instant::now);
            let mut snapshot_tick = tokio::time::interval(SNAPSHOT_JOURNAL_INTERVAL);
            snapshot_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            snapshot_tick.tick().await;

            loop {
                tokio::select! {
                    result = receiver.recv() => match result {
                        Ok(line) => {
                            let Some(payload) = serde_json::from_str::<serde_json::Value>(line.trim()).ok() else {
                                continue;
                            };
                            let kind = durable_event_kind(&payload);
                            if kind == "message" {
                                if last_snapshot_at.elapsed() >= SNAPSHOT_JOURNAL_INTERVAL {
                                    hub.publish_value(&session_id, run_id.as_deref(), &payload);
                                    last_snapshot_at = Instant::now();
                                } else {
                                    pending_snapshot = Some(payload);
                                }
                                continue;
                            }

                            // Preserve the last visible assistant state before a
                            // terminal event so reconnect never observes `done`
                            // without the content that immediately preceded it.
                            if matches!(kind, "done" | "error")
                                && let Some(snapshot) = pending_snapshot.take()
                            {
                                hub.publish_value(&session_id, run_id.as_deref(), &snapshot);
                                last_snapshot_at = Instant::now();
                            }
                            hub.publish_value(&session_id, run_id.as_deref(), &payload);
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::error!(session_id, skipped, "realtime journal relay lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            if let Some(snapshot) = pending_snapshot.take() {
                                hub.publish_value(&session_id, run_id.as_deref(), &snapshot);
                            }
                            break;
                        }
                    },
                    _ = snapshot_tick.tick(), if pending_snapshot.is_some() => {
                        if let Some(snapshot) = pending_snapshot.take() {
                            hub.publish_value(&session_id, run_id.as_deref(), &snapshot);
                            last_snapshot_at = Instant::now();
                        }
                    }
                }
            }
        });
    }

    pub async fn issue_ticket(&self, user_id: String) -> (String, u64) {
        let ticket = uuid::Uuid::new_v4().simple().to_string();
        let mut tickets = self.tickets.lock().await;
        let now = Instant::now();
        tickets.retain(|_, entry| entry.expires_at > now);
        tickets.insert(
            ticket.clone(),
            RealtimeTicket {
                user_id,
                expires_at: now + TICKET_TTL,
            },
        );
        (ticket, TICKET_TTL.as_secs())
    }

    pub async fn consume_ticket(&self, ticket: &str) -> Option<String> {
        let entry = self.tickets.lock().await.remove(ticket)?;
        (entry.expires_at > Instant::now()).then_some(entry.user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        realtime::{create_run, replay},
        sessions::{UpsertSession, upsert_session},
    };

    #[tokio::test]
    async fn legacy_relay_keeps_latest_snapshot_before_terminal_event() {
        let db = Arc::new(Db::open(":memory:"));
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/tmp/project".into(),
                title: Some("Session".into()),
                message_count: Some(0),
                pinned: None,
                model: None,
                input_tokens: None,
                output_tokens: None,
                provider: None,
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );
        assert!(create_run(
            &db,
            "run-1",
            "session-1",
            "user-1",
            &serde_json::json!({"message": "hello"}),
            "running",
        ));

        let hub = RealtimeHub::new(Arc::clone(&db));
        let mut published = hub.subscribe();
        let (tx, rx) = broadcast::channel(512);
        hub.attach_legacy_stream("session-1".into(), Some("run-1".into()), rx);

        for index in 0..200 {
            tx.send(serde_json::json!({"type": "message", "content": index}).to_string())
                .unwrap();
        }
        tx.send(serde_json::json!({"type": "done"}).to_string())
            .unwrap();
        drop(tx);

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if published.recv().await.unwrap().kind == "done" {
                    break;
                }
            }
        })
        .await
        .expect("relay did not publish terminal event");

        let through = crate::db::realtime::max_sequence(&db, "user-1", "session-1");
        let events = replay(&db, "user-1", "session-1", 0, through);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "message")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .find(|event| event.kind == "message")
                .unwrap()
                .payload["content"],
            199
        );
        assert_eq!(events.last().unwrap().kind, "done");
    }

    #[tokio::test]
    async fn legacy_relay_never_coalesces_tool_results_with_assistant_snapshots() {
        let db = Arc::new(Db::open(":memory:"));
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/tmp/project".into(),
                title: Some("Session".into()),
                message_count: Some(0),
                pinned: None,
                model: None,
                input_tokens: None,
                output_tokens: None,
                provider: None,
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );
        assert!(create_run(
            &db,
            "run-1",
            "session-1",
            "user-1",
            &serde_json::json!({"message": "hello"}),
            "running",
        ));

        let hub = RealtimeHub::new(Arc::clone(&db));
        let mut published = hub.subscribe();
        let (tx, rx) = broadcast::channel(32);
        hub.attach_legacy_stream("session-1".into(), Some("run-1".into()), rx);

        tx.send(
            serde_json::json!({
                "type": "message",
                "message": {"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "tool-1"}]}}
            })
            .to_string(),
        )
        .unwrap();
        tx.send(
            serde_json::json!({
                "type": "message",
                "message": {"type": "tool_result", "tool_use_id": "tool-1", "content": "ok", "is_error": false}
            })
            .to_string(),
        )
        .unwrap();
        tx.send(
            serde_json::json!({
                "type": "message",
                "message": {"type": "assistant", "message": {"content": [{"type": "text", "text": "continuing"}]}}
            })
            .to_string(),
        )
        .unwrap();
        tx.send(serde_json::json!({"type": "done"}).to_string())
            .unwrap();
        drop(tx);

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if published.recv().await.unwrap().kind == "done" {
                    break;
                }
            }
        })
        .await
        .expect("relay did not publish terminal event");

        let through = crate::db::realtime::max_sequence(&db, "user-1", "session-1");
        let events = replay(&db, "user-1", "session-1", 0, through);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "message")
                .count(),
            1
        );
        let result = events
            .iter()
            .find(|event| event.kind == "tool_result")
            .expect("tool result must remain replayable");
        assert_eq!(result.payload["message"]["tool_use_id"], "tool-1");
        assert_eq!(events.last().unwrap().kind, "done");
    }
}
