use super::*;
use serde_json::{Value, json};
use tokio::sync::oneshot;

#[derive(Clone, Debug)]
pub struct SteeringTarget {
    generation: u64,
    thread_id: String,
    turn_id: String,
}

#[derive(Debug, PartialEq)]
pub enum SteeringResult {
    Sent,
    NotSent,
    Uncertain,
}

fn steering_request(target: &SteeringTarget, id: &str, text: &str) -> Value {
    json!({"jsonrpc":"2.0", "id": id, "method":"turn/steer", "params": {
        "threadId": target.thread_id, "expectedTurnId": target.turn_id,
        "input": [{"type":"text", "text":text}],
    }})
}

fn steering_result(response: &Value, target: &SteeringTarget) -> SteeringResult {
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        // Internal errors may happen after acceptance. Only protocol rejection
        // proves it is safe to queue the input as a separate turn.
        if matches!(
            error.get("code").and_then(Value::as_i64),
            Some(-32600 | -32601 | -32602)
        ) {
            SteeringResult::NotSent
        } else {
            SteeringResult::Uncertain
        }
    } else if response.pointer("/result/turnId").and_then(Value::as_str) == Some(&target.turn_id) {
        SteeringResult::Sent
    } else {
        SteeringResult::Uncertain
    }
}

impl SessionPool {
    pub async fn steering_target(&self, session_id: &str) -> Option<SteeringTarget> {
        let sessions = self.sessions.lock().await;
        let session = sessions.get(session_id)?;
        if session.dead || !session.busy || session.provider != ProviderKind::Codex {
            return None;
        }
        Some(SteeringTarget {
            generation: session.generation,
            thread_id: session.codex_thread_id.clone()?,
            turn_id: session.codex_current_turn_id.clone()?,
        })
    }

    /// Acknowledgement is required. After a partial write or lost reply, callers
    /// must preserve the receipt as uncertain, never also queue the same input.
    pub async fn steer_session(
        &self,
        session_id: &str,
        target: &SteeringTarget,
        input_id: &str,
        text: &str,
    ) -> SteeringResult {
        let rpc_id = format!("flamel-steer-{input_id}");
        let (tx, rx) = oneshot::channel();
        let mut bytes = serde_json::to_vec(&steering_request(target, &rpc_id, text)).unwrap();
        bytes.push(b'\n');
        {
            let mut sessions = self.sessions.lock().await;
            let Some(session) = sessions.get_mut(session_id).filter(|session| {
                !session.dead
                    && session.busy
                    && session.generation == target.generation
                    && session.codex_current_turn_id.as_deref() == Some(target.turn_id.as_str())
            }) else {
                return SteeringResult::NotSent;
            };
            let Some(stdin) = session.stdin.as_mut() else {
                return SteeringResult::NotSent;
            };
            session.control_replies.insert(rpc_id.clone(), tx);
            // Bound a single writer; the existing pool serializes stdin writes.
            let write = tokio::time::timeout(Duration::from_secs(2), async {
                stdin.write_all(&bytes).await?;
                stdin.flush().await
            })
            .await;
            if !matches!(write, Ok(Ok(()))) {
                session.control_replies.remove(&rpc_id);
                return SteeringResult::Uncertain;
            }
        }
        let result = match tokio::time::timeout(Duration::from_secs(10), rx).await {
            Ok(Ok(response)) => steering_result(&response, target),
            _ => SteeringResult::Uncertain,
        };
        if let Some(session) = self
            .sessions
            .lock()
            .await
            .get_mut(session_id)
            .filter(|session| session.generation == target.generation)
        {
            session.control_replies.remove(&rpc_id);
        }
        result
    }

    pub(super) async fn consume_control_reply(
        &self,
        session_id: &str,
        generation: u64,
        line: &str,
    ) -> bool {
        let Ok(response) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        let Some(id) = response
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("flamel-steer-"))
        else {
            return false;
        };
        if response.get("method").is_some() {
            return false;
        }
        if let Some(session) = self
            .sessions
            .lock()
            .await
            .get_mut(session_id)
            .filter(|session| session.generation == generation)
        {
            if let Some(reply) = session.control_replies.remove(id) {
                let _ = reply.send(response);
            }
        }
        // A late control reply must not become a provider error/assistant turn.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> SteeringTarget {
        SteeringTarget {
            generation: 1,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
        }
    }
    #[test]
    fn steering_targets_exact_turn_without_overrides_or_interrupt() {
        let request = steering_request(&target(), "request", "keep going");
        assert_eq!(request["method"], "turn/steer");
        assert_eq!(request["params"]["expectedTurnId"], "turn");
        assert!(request["params"].get("model").is_none());
        assert_eq!(
            steering_result(&json!({"result":{"turnId":"turn"}}), &target()),
            SteeringResult::Sent
        );
        assert_eq!(
            steering_result(&json!({"result":{}}), &target()),
            SteeringResult::Uncertain
        );
        assert_eq!(
            steering_result(
                &json!({"error":{"code":-32600,"message":"no active turn"}}),
                &target()
            ),
            SteeringResult::NotSent
        );
    }

    #[cfg(unix)]
    async fn fixture(
        reply: &str,
    ) -> (
        Arc<SessionPool>,
        tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
    ) {
        // Echo the received request, then a fake provider response. No real
        // account, model request, repository, or provider config is touched.
        let mut child = Command::new("sh")
            .args([
                "-c",
                "while IFS= read -r line; do printf '%s\\n' \"$line\" \"$1\"; done",
                "fake-provider",
                reply,
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let output = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
        let pool = Arc::new(SessionPool::new());
        pool.sessions.lock().await.insert(
            "session".into(),
            PooledSession {
                generation: 1,
                turn_generation: 1,
                child,
                stdin,
                tx: broadcast::channel(4).0,
                busy: true,
                dead: false,
                last_used: Instant::now(),
                created_at: Instant::now(),
                idle_cancel: Arc::new(Notify::new()),
                busy_notify: Arc::new(Notify::new()),
                provider: ProviderKind::Codex,
                model: "test-model".into(),
                reasoning_effort: None,
                permission_mode: "default".into(),
                ponytail_enabled: false,
                system_prompt: None,
                codex_thread_id: Some("thread".into()),
                codex_current_turn_id: Some("turn".into()),
                control_replies: HashMap::new(),
                codex_pending_turn: None,
                codex_interrupt_pending: false,
                codex_attempted_resume: false,
                user_home: String::new(),
            },
        );
        (pool, output)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stale_cleanup_cannot_idle_or_kill_a_successor_turn() {
        let (pool, _output) = fixture("{}").await;
        let previous = pool.turn_owner("session").await.unwrap();
        pool.sessions
            .lock()
            .await
            .get_mut("session")
            .unwrap()
            .turn_generation += 1;
        pool.mark_owned_turn_idle("session", previous).await;
        assert!(pool.sessions.lock().await.get("session").unwrap().busy);
        assert!(!pool.kill_owned_turn("session", Some(previous)).await);
        let current = pool.turn_owner("session").await.unwrap();
        pool.mark_owned_turn_idle("session", current).await;
        assert!(!pool.sessions.lock().await.get("session").unwrap().busy);
        pool.sessions
            .lock()
            .await
            .get_mut("session")
            .unwrap()
            .generation += 1;
        assert!(!pool.kill_owned_turn("session", Some(current)).await);
        let replacement = pool.turn_owner("session").await.unwrap();
        assert!(pool.kill_owned_turn("session", Some(replacement)).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn native_steer_requires_ack_and_refuses_a_replaced_turn() {
        let (pool, mut output) =
            fixture(r#"{"id":"flamel-steer-guidance","result":{"turnId":"turn"}}"#).await;
        let captured = pool.steering_target("session").await.unwrap();
        let mut stale = captured.clone();
        stale.generation = 0;
        assert_eq!(
            pool.steer_session("session", &stale, "stale", "not sent")
                .await,
            SteeringResult::NotSent
        );
        let mut stale = captured.clone();
        stale.turn_id = "previous".into();
        assert_eq!(
            pool.steer_session("session", &stale, "stale", "not sent")
                .await,
            SteeringResult::NotSent
        );
        let task_pool = Arc::clone(&pool);
        let sending = tokio::spawn(async move {
            task_pool
                .steer_session("session", &captured, "guidance", "keep going")
                .await
        });
        let request = tokio::time::timeout(Duration::from_secs(2), output.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let parsed: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(parsed["method"], "turn/steer");
        assert_eq!(parsed["params"]["input"][0]["text"], "keep going");
        assert!(!sending.is_finished());
        assert!(!pool.consume_control_reply("session", 1, &request).await);
        let reply = output.next_line().await.unwrap().unwrap();
        assert!(pool.consume_control_reply("session", 1, &reply).await);
        assert_eq!(sending.await.unwrap(), SteeringResult::Sent);
        assert!(
            pool.sessions.lock().await["session"]
                .control_replies
                .is_empty()
        );
        // Late acknowledgement is consumed rather than becoming an assistant
        // response/error or a new turn.
        assert!(pool.consume_control_reply("session", 1, &reply).await);
        pool.kill_session("session").await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mismatched_ack_is_uncertain_and_explicit_rejection_is_not_sent() {
        for (reply, expected) in [
            (
                r#"{"id":"flamel-steer-guidance","result":{"turnId":"other-turn"}}"#,
                SteeringResult::Uncertain,
            ),
            (
                r#"{"id":"flamel-steer-guidance","error":{"code":-32600,"message":"turn ended"}}"#,
                SteeringResult::NotSent,
            ),
        ] {
            let (pool, mut output) = fixture(reply).await;
            let captured = pool.steering_target("session").await.unwrap();
            let task_pool = Arc::clone(&pool);
            let sending = tokio::spawn(async move {
                task_pool
                    .steer_session("session", &captured, "guidance", "keep going")
                    .await
            });
            let _request = tokio::time::timeout(Duration::from_secs(2), output.next_line())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let response = output.next_line().await.unwrap().unwrap();
            pool.consume_control_reply("session", 1, &response).await;
            assert_eq!(sending.await.unwrap(), expected);
            pool.kill_session("session").await;
        }
    }
}
