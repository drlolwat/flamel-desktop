use axum::{
    Json,
    extract::{Query, State, WebSocketUpgrade, ws::Message},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

use crate::routes::admin::get_current_user_id;
use crate::{
    AppState,
    db::sessions::{get_session, has_visible_session_messages, load_persisted_session_messages},
};

fn persisted_snapshot_event_type(role: &str) -> Option<&'static str> {
    match role {
        "assistant" => Some("assistant"),
        "tool_result" => Some("tool_result"),
        _ => None,
    }
}

fn subscription_includes_history(command: &Value, after_sequence: u64) -> bool {
    // Older clients rely on the first snapshot for their transcript. Clients
    // with an authoritative REST bootstrap can request only live control state.
    // A resumed cursor never needs to reload history, regardless of this flag.
    after_sequence == 0
        && command
            .get("includeHistory")
            .and_then(Value::as_bool)
            .unwrap_or(true)
}

fn snapshot_history_lines(
    include_history: bool,
    load_history: impl FnOnce() -> Vec<(String, String)>,
) -> Vec<Value> {
    if !include_history {
        return Vec::new();
    }
    load_history()
        .into_iter()
        .filter_map(|(role, message)| {
            let event_type = persisted_snapshot_event_type(&role)?;
            let message = serde_json::from_str::<Value>(&message).ok()?;
            // Never disguise a persisted user prompt as a tool result.
            Some(json!({
                "type": "message",
                "message": {"type": event_type, "message": message}
            }))
        })
        .collect()
}

pub(crate) fn pending_question_projection(
    request: &crate::db::input_requests::InputRequest,
) -> Value {
    json!({
        "type": "ask_question",
        "kind": "question.requested",
        "requestId": request.id,
        "sessionId": request.session_id,
        "runId": request.run_id,
        "status": request.status,
        "question": request.request.get("question"),
        "choices": request.request.get("choices"),
        "allowFreeText": request.request.get("allowFreeText").and_then(Value::as_bool).unwrap_or(true),
        "multiSelect": false,
        "previewMarkdown": request.request.get("previewMarkdown"),
    })
}

#[derive(Deserialize)]
pub struct TicketQuery {
    ticket: String,
}

pub async fn handle_ticket(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Some(user_id) = get_current_user_id(&state, &headers) else {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))).into_response();
    };
    let (ticket, expires_in) = state.realtime.issue_ticket(user_id).await;
    Json(json!({"ticket": ticket, "expiresIn": expires_in})).into_response()
}

pub async fn handle_socket(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(query): Query<TicketQuery>,
) -> Response {
    let Some(user_id) = state.realtime.consume_ticket(&query.ticket).await else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Invalid or expired realtime ticket"})),
        )
            .into_response();
    };
    ws.on_upgrade(move |socket| serve_socket(socket, state, user_id))
}

async fn snapshot_lines(
    state: &AppState,
    user_id: &str,
    session_id: &str,
    through_sequence: u64,
    include_history: bool,
) -> Option<(Vec<Value>, u64)> {
    let session = get_session(&state.db, session_id)?;
    if session.user_id.as_deref() != Some(user_id) {
        return None;
    }
    let mut lines = snapshot_history_lines(include_history, || {
        load_persisted_session_messages(&state.db, session_id)
    });
    let is_session_warm = state.session_pool.is_session_warm(session_id).await;
    let is_running = matches!(
        session.continuity_status.as_str(),
        "queued" | "active" | "recovering"
    );
    let requires_resume = session.claude_session_id.is_some()
        && !is_session_warm
        && !is_running
        && has_visible_session_messages(&state.db, session_id);
    let active = state.active_sessions.lock().await.contains(session_id) || is_running;
    let replay_after = if active {
        session
            .active_request_id
            .as_deref()
            .and_then(|run_id| {
                crate::db::realtime::first_sequence_for_run(&state.db, user_id, session_id, run_id)
            })
            .map(|sequence| sequence.saturating_sub(1))
            .unwrap_or(through_sequence)
    } else {
        through_sequence
    };
    lines.push(json!({"type": "streaming", "active": active}));
    let recovery = crate::db::run_lifecycle::recovery_metadata(
        &state.db,
        session.active_request_id.as_deref(),
    );
    lines.push(json!({
        "type": "continuity",
        "completionManaged": "server",
        "retryAt": recovery["retryAt"],
        "recoveryReason": recovery["recoveryReason"],
        "status": session.continuity_status,
        "recoveryStrategy": session.recovery_strategy,
        "activeRequestId": session.active_request_id,
        "lastCheckpointAt": session.last_checkpoint_at,
        "lastError": session.last_error,
        "provider": session.provider,
        "isSessionWarm": is_session_warm,
        "requiresResume": requires_resume,
    }));
    let pending_questions =
        crate::db::input_requests::list_pending_for_session(&state.db, user_id, session_id)
            .unwrap_or_default();
    let durable_ids = pending_questions
        .iter()
        .map(|request| request.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    lines.extend(pending_questions.iter().map(pending_question_projection));
    // Runner approval prompts still use their existing in-memory protocol. Durable
    // AskUserQuestion rows always win if an id ever overlaps.
    if let Some(legacy) = state.pending_asks.lock().await.get(session_id) {
        lines.extend(
            legacy
                .iter()
                .filter(|(request_id, _)| !durable_ids.contains(request_id.as_str()))
                .map(|(_, event)| event.clone()),
        );
    }
    lines.extend(
        crate::db::realtime::list_queued_runs(&state.db, user_id, session_id)
            .into_iter()
            .map(|run| {
                json!({
                    "type": "queue_updated",
                    "sessionId": run.session_id,
                    "requestId": run.id,
                    "state": "queued",
                    "message": run.message,
                    "hiddenContinuation": run.hidden_continuation,
                    "createdAt": run.created_at,
                })
            }),
    );
    Some((lines, replay_after))
}

async fn send_json<S>(
    sender: &mut futures::stream::SplitSink<S, Message>,
    value: Value,
) -> Result<(), axum::Error>
where
    S: futures::Sink<Message, Error = axum::Error> + Unpin,
{
    sender.send(Message::Text(value.to_string().into())).await
}

fn question_request_id(event: &crate::db::realtime::SessionEvent) -> Option<&str> {
    let lifecycle = event
        .payload
        .get("kind")
        .and_then(Value::as_str)
        .or_else(|| {
            (event.kind == "ask_question"
                || event.payload.get("type").and_then(Value::as_str) == Some("ask_question"))
            .then_some("question.requested")
        });
    if lifecycle != Some("question.requested") {
        return None;
    }
    event
        .payload
        .get("payload")
        .unwrap_or(&event.payload)
        .get("requestId")
        .and_then(Value::as_str)
}

fn realtime_event_envelope(event: &crate::db::realtime::SessionEvent, replay: bool) -> Value {
    json!({"type":"event", "replay":replay, "event":event})
}

fn replayable_question_event(
    event: &crate::db::realtime::SessionEvent,
    fresh_snapshot: bool,
    is_pending: bool,
) -> bool {
    question_request_id(event).is_none() || (!fresh_snapshot && is_pending)
}

async fn should_replay_event(
    state: &AppState,
    event: &crate::db::realtime::SessionEvent,
    fresh_snapshot: bool,
) -> bool {
    let Some(request_id) = question_request_id(event) else {
        return true;
    };
    let durable = crate::db::input_requests::get_input_request_for_user(
        &state.db,
        request_id,
        &event.user_id,
    )
    .ok()
    .flatten();
    let is_pending = durable.as_ref().is_some_and(|request| {
        request.session_id == event.session_id && request.status == "pending"
    }) || (durable.is_none()
        && state
            .pending_asks
            .lock()
            .await
            .get(&event.session_id)
            .is_some_and(|questions| questions.contains_key(request_id)));
    // A fresh snapshot already includes every currently pending question. Old
    // question events must not reopen prompts that have been answered.
    replayable_question_event(event, fresh_snapshot, is_pending)
}

pub async fn serve_socket<S>(socket: S, state: Arc<AppState>, user_id: String)
where
    S: futures::Sink<Message, Error = axum::Error>
        + futures::Stream<Item = Result<Message, axum::Error>>
        + Unpin,
{
    let (mut sender, mut receiver) = socket.split();
    let mut live = state.realtime.subscribe();
    let mut shutdown = state.shutdown_receiver();
    let mut cursors: HashMap<String, u64> = HashMap::new();

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    let _ = sender.send(Message::Close(None)).await;
                    break;
                }
            }
            incoming = receiver.next() => {
                let Some(Ok(message)) = incoming else { break };
                match message {
                    Message::Text(text) => {
                        let Ok(command) = serde_json::from_str::<Value>(&text) else { continue };
                        match command.get("type").and_then(Value::as_str) {
                            Some("subscribe") => {
                                let Some(session_id) = command.get("sessionId").and_then(Value::as_str).map(str::to_string) else { continue };
                                let after = command.get("afterSequence").and_then(Value::as_u64).unwrap_or(0);
                                let include_history = subscription_includes_history(&command, after);
                                if get_session(&state.db, &session_id).and_then(|s| s.user_id).as_deref() != Some(user_id.as_str()) {
                                    if send_json(&mut sender, json!({"type":"error","sessionId":session_id,"error":"Forbidden"})).await.is_err() { break; }
                                    continue;
                                }

                                // Subscribe happened before the high-water read, so events created
                                // during replay will either be replayed or observed on `live`.
                                let through = crate::db::realtime::max_sequence(&state.db, &user_id, &session_id);
                                let replay_after = if after == 0 {
                                    let (lines, replay_after) = snapshot_lines(
                                        &state,
                                        &user_id,
                                        &session_id,
                                        through,
                                        include_history,
                                    ).await.unwrap_or_default();
                                    if send_json(&mut sender, json!({
                                        "type":"snapshot", "sessionId":session_id,
                                        "throughSequence":replay_after, "lines":lines
                                    })).await.is_err() { break; }
                                    replay_after
                                } else {
                                    // Every subscription needs the streaming boundary,
                                    // including an in-process resubscribe from a saved
                                    // cursor. Without it the client remains in its history
                                    // phase and receives live events without rendering them.
                                    let (lines, _) = snapshot_lines(
                                        &state,
                                        &user_id,
                                        &session_id,
                                        through,
                                        false,
                                    ).await.unwrap_or_default();
                                    if send_json(&mut sender, json!({
                                        "type":"snapshot", "sessionId":session_id,
                                        "throughSequence":after,
                                        "lines":lines
                                    })).await.is_err() { break; }
                                    after
                                };
                                for event in crate::db::realtime::replay(&state.db, &user_id, &session_id, replay_after, through) {
                                    if !should_replay_event(&state, &event, after == 0).await { continue; }
                                    if send_json(&mut sender, realtime_event_envelope(&event, true)).await.is_err() { return; }
                                }
                                cursors.insert(session_id.clone(), through.max(after));
                                if send_json(&mut sender, json!({"type":"caughtUp","sessionId":session_id,"sequence":through})).await.is_err() { break; }
                            }
                            Some("unsubscribe") => {
                                if let Some(session_id) = command.get("sessionId").and_then(Value::as_str) {
                                    cursors.remove(session_id);
                                }
                            }
                            Some("ping") => {
                                if send_json(&mut sender, json!({"type":"pong"})).await.is_err() { break; }
                            }
                            _ => {}
                        }
                    }
                    Message::Ping(bytes) => {
                        if sender.send(Message::Pong(bytes)).await.is_err() { break; }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            event = live.recv() => {
                match event {
                    Ok(event) => {
                        let Some(cursor) = cursors.get_mut(&event.session_id) else { continue };
                        if event.user_id != user_id || event.sequence <= *cursor { continue; }
                        if !should_replay_event(&state, &event, false).await {
                            *cursor = event.sequence;
                            continue;
                        }
                        if send_json(&mut sender, realtime_event_envelope(&event, false)).await.is_err() { break; }
                        *cursor = event.sequence;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Replay every subscribed session from its last acknowledged cursor.
                        let subscriptions = cursors.clone();
                        for (session_id, cursor) in subscriptions {
                            let through = crate::db::realtime::max_sequence(&state.db, &user_id, &session_id);
                            for event in crate::db::realtime::replay(&state.db, &user_id, &session_id, cursor, through) {
                                if !should_replay_event(&state, &event, false).await { continue; }
                                if send_json(&mut sender, realtime_event_envelope(&event, true)).await.is_err() { return; }
                            }
                            cursors.insert(session_id, through);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::realtime::SessionEvent;

    fn event(kind: &str, payload: Value) -> SessionEvent {
        SessionEvent {
            sequence: 1,
            event_id: "event-1".into(),
            session_id: "session-1".into(),
            user_id: "user-1".into(),
            run_id: Some("run-1".into()),
            kind: kind.into(),
            payload,
            created_at: "2026-07-20T00:00:00Z".into(),
        }
    }

    #[test]
    fn durable_projection_carries_stable_owner_and_run_identity() {
        let projected = pending_question_projection(&crate::db::input_requests::InputRequest {
            id: "question-1".into(),
            session_id: "session-1".into(),
            run_id: Some("run-1".into()),
            user_id: "user-1".into(),
            request: json!({
                "question": "Which path?",
                "choices": [{"label": "A"}, {"label": "B"}],
                "allowFreeText": false,
            }),
            status: "pending".into(),
            answer: None,
            delivery_kind: None,
            delivery_claim_owner: None,
            delivery_claimed_at: None,
            delivery_claim_expires_at: None,
            created_at: "2026-07-20T00:00:00Z".into(),
            updated_at: "2026-07-20T00:00:00Z".into(),
            answered_at: None,
            cancelled_at: None,
            consumed_at: None,
        });
        assert_eq!(projected["kind"], json!("question.requested"));
        assert_eq!(projected["sessionId"], json!("session-1"));
        assert_eq!(projected["runId"], json!("run-1"));
        assert_eq!(projected["status"], json!("pending"));
    }

    #[test]
    fn fresh_snapshots_do_not_replay_question_journal_entries() {
        let question = event(
            "ask_question",
            json!({"type":"ask_question", "requestId":"question-1"}),
        );
        assert!(!replayable_question_event(&question, true, true));
        assert!(!replayable_question_event(&question, true, false));
    }

    #[test]
    fn reconnect_replay_only_restores_questions_that_are_still_pending() {
        let question = event(
            "ask_question",
            json!({"type":"ask_question", "requestId":"question-1"}),
        );
        assert!(replayable_question_event(&question, false, true));
        assert!(!replayable_question_event(&question, false, false));
        assert!(replayable_question_event(
            &event("assistant", json!({"type":"message"})),
            false,
            false,
        ));
    }

    #[test]
    fn event_envelopes_distinguish_replay_from_live_delivery() {
        let completion = event("done", json!({"type":"done", "queuePending":true}));

        let replayed = realtime_event_envelope(&completion, true);
        let live = realtime_event_envelope(&completion, false);

        assert_eq!(replayed["replay"], json!(true));
        assert_eq!(live["replay"], json!(false));
        assert_eq!(replayed["event"]["payload"], completion.payload);
    }

    #[test]
    fn legacy_subscriptions_include_history_unless_explicitly_omitted() {
        for command in [
            json!({"type":"subscribe"}),
            json!({"includeHistory":true}),
            json!({"includeHistory":null}),
            json!({"includeHistory":"false"}),
        ] {
            assert!(subscription_includes_history(&command, 0));
        }
        assert!(!subscription_includes_history(
            &json!({"includeHistory":false}),
            0,
        ));
    }

    #[test]
    fn control_only_and_resumed_snapshots_never_load_the_transcript() {
        for (command, after) in [
            (json!({"includeHistory":false}), 0),
            (json!({}), 7),
            (json!({"includeHistory":true}), 7),
            (json!({"includeHistory":false}), 7),
        ] {
            let lines =
                snapshot_history_lines(subscription_includes_history(&command, after), || {
                    panic!("Control snapshots must not read or parse the full transcript")
                });
            assert!(lines.is_empty());
        }
    }

    #[test]
    fn legacy_snapshot_still_projects_assistant_and_tool_history() {
        let assistant = json!({"content":[{"type":"text","text":"Saved answer"}]});
        let result =
            json!({"content":[{"type":"tool_result","tool_use_id":"tool-1","content":"Done"}]});
        let lines = snapshot_history_lines(subscription_includes_history(&json!({}), 0), || {
            vec![
                ("user".into(), "not parsed as provider output".into()),
                ("assistant".into(), assistant.to_string()),
                ("tool_result".into(), result.to_string()),
            ]
        });
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["message"]["type"], "assistant");
        assert_eq!(lines[0]["message"]["message"], assistant);
        assert_eq!(lines[1]["message"]["type"], "tool_result");
        assert_eq!(lines[1]["message"]["message"], result);
    }

    #[tokio::test]
    async fn control_snapshots_keep_running_question_queue_and_replay_boundary() {
        let state = AppState::new(":memory:", 0);
        state.db.conn().execute_batch(
            "INSERT INTO sessions (id,project_path,created_at,updated_at,user_id,continuity_status,active_request_id)
             VALUES ('session-1','/repo','2026-09-06','2026-09-06','user-1','active','run-1');
             INSERT INTO session_messages (session_id,role,message,created_at)
             VALUES ('session-1','assistant','{\"content\":[{\"type\":\"text\",\"text\":\"Saved answer\"}]}','2026-09-06');"
        ).unwrap();
        assert!(crate::db::realtime::create_run(
            &state.db,
            "run-1",
            "session-1",
            "user-1",
            &json!({"message":"Current task"}),
            "running",
        ));
        let older = crate::db::realtime::append_event(
            &state.db,
            "session-1",
            Some("previous-run"),
            "done",
            &json!({"type":"done"}),
        )
        .unwrap();
        let current = crate::db::realtime::append_event(
            &state.db,
            "session-1",
            Some("run-1"),
            "message",
            &json!({"type":"message"}),
        )
        .unwrap();
        let latest = crate::db::realtime::append_event(
            &state.db,
            "session-1",
            Some("run-1"),
            "tool_result",
            &json!({"type":"tool_result"}),
        )
        .unwrap();
        crate::db::input_requests::create_input_request(
            &state.db,
            crate::db::input_requests::NewInputRequest {
                id: "question-1",
                session_id: "session-1",
                run_id: Some("run-1"),
                user_id: "user-1",
                request: &json!({"question":"Choose next step"}),
            },
        )
        .unwrap();
        assert!(crate::db::realtime::create_run(
            &state.db,
            "queued-1",
            "session-1",
            "user-1",
            &json!({"message":"Next task"}),
            "queued",
        ));

        let (legacy, legacy_after) =
            snapshot_lines(&state, "user-1", "session-1", latest.sequence, true)
                .await
                .unwrap();
        let (control, control_after) =
            snapshot_lines(&state, "user-1", "session-1", latest.sequence, false)
                .await
                .unwrap();
        assert!(legacy.iter().any(|line| line["type"] == "message"));
        assert_eq!(
            control,
            legacy
                .into_iter()
                .filter(|line| line["type"] != "message")
                .collect::<Vec<_>>()
        );
        assert!(
            control
                .iter()
                .any(|line| line["type"] == "streaming" && line["active"] == true)
        );
        assert!(
            control
                .iter()
                .any(|line| line["type"] == "continuity" && line["activeRequestId"] == "run-1")
        );
        assert!(
            control
                .iter()
                .any(|line| line["type"] == "ask_question" && line["requestId"] == "question-1")
        );
        assert!(
            control
                .iter()
                .any(|line| line["type"] == "queue_updated" && line["requestId"] == "queued-1")
        );
        assert_eq!(legacy_after, current.sequence - 1);
        assert_eq!(control_after, legacy_after);
        assert_eq!(control_after, older.sequence);
        let replay = crate::db::realtime::replay(
            &state.db,
            "user-1",
            "session-1",
            control_after,
            latest.sequence,
        );
        assert_eq!(
            replay
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![current.sequence, latest.sequence]
        );
        assert!(
            snapshot_lines(&state, "other-user", "session-1", latest.sequence, false)
                .await
                .is_none()
        );
    }

    #[test]
    fn fresh_snapshot_never_disguises_user_prompts_as_tool_results() {
        assert_eq!(
            persisted_snapshot_event_type("assistant"),
            Some("assistant")
        );
        assert_eq!(
            persisted_snapshot_event_type("tool_result"),
            Some("tool_result")
        );
        assert_eq!(persisted_snapshot_event_type("user"), None);
        assert_eq!(persisted_snapshot_event_type("system"), None);
    }
}
