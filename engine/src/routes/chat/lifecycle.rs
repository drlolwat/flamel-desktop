use crate::{AppState, db::run_lifecycle};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Owns one durable attempt even if its future panics or is cancelled.
pub(super) struct ExecutionGuard {
    state: Arc<AppState>,
    run: String,
    session: String,
    pub owner: String,
    channel: broadcast::Sender<String>,
    heartbeat: tokio::task::JoinHandle<()>,
    finished: bool,
}

impl ExecutionGuard {
    pub fn new(
        state: Arc<AppState>,
        run: String,
        session: String,
        owner: String,
        channel: broadcast::Sender<String>,
    ) -> Self {
        let (beat_state, beat_run, beat_owner) = (Arc::clone(&state), run.clone(), owner.clone());
        let heartbeat = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                if !run_lifecycle::heartbeat(&beat_state.db, &beat_run, &beat_owner)
                    .unwrap_or(false)
                {
                    break;
                }
            }
        });
        Self {
            state,
            run,
            session,
            owner,
            channel,
            heartbeat,
            finished: false,
        }
    }
    pub fn owns(&self) -> bool {
        run_lifecycle::owns(&self.state.db, &self.run, &self.owner)
    }
    pub fn finish(&mut self) {
        self.finished = true;
        self.heartbeat.abort();
    }
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        self.heartbeat.abort();
        if self.finished {
            return;
        }
        let state = Arc::clone(&self.state);
        let (run, session, owner, channel) = (
            self.run.clone(),
            self.session.clone(),
            self.owner.clone(),
            self.channel.clone(),
        );
        tokio::spawn(async move {
            if !run_lifecycle::owns(&state.db, &run, &owner) {
                return;
            }
            let provider_owner = state.session_pool.turn_owner(&session).await;
            if !run_lifecycle::owns(&state.db, &run, &owner) {
                return;
            }
            state
                .session_pool
                .kill_owned_turn(&session, provider_owner)
                .await;
            release_channel(&state, &session, &channel).await;
            match run_lifecycle::park(
                &state.db,
                &run,
                &owner,
                "Execution ended unexpectedly; automatic recovery is scheduled",
            ) {
                Ok(Some(retry)) => {
                    state.realtime.publish_value(&session,Some(&run),&serde_json::json!({
                    "type":"continuity","status":"recovering","activeRequestId":run,
                    "retryAt":retry,"completionManaged":"server","isSessionWarm":false,"requiresResume":false
                }));
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::error!(%error,run,"could not preserve interrupted execution")
                }
            }
        });
    }
}

pub(super) async fn release_channel(
    state: &AppState,
    session: &str,
    channel: &broadcast::Sender<String>,
) -> bool {
    let mut streams = state.live_streams.lock().await;
    if !streams
        .get(session)
        .is_some_and(|current| current.same_channel(channel))
    {
        return false;
    }
    streams.remove(session);
    state.active_sessions.lock().await.remove(session);
    true
}

pub(crate) fn start_dispatcher(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(5));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            if state.is_shutting_down() {
                break;
            }
            crate::routes::ask::recover_answered_input_requests(Arc::clone(&state)).await;
            let active: Vec<_> = state.active_sessions.lock().await.iter().cloned().collect();
            if let Err(error) = run_lifecycle::recover_orphans(&state.db, &active) {
                tracing::error!(%error,"orphan recovery scan failed");
            }
            match run_lifecycle::due(&state.db, &active) {
                Ok(runs) => {
                    for run in runs {
                        let next = Arc::clone(&state);
                        tokio::spawn(async move {
                            super::submit_durable_chat_run(next, run).await;
                        });
                    }
                }
                Err(error) => tracing::error!(%error,"durable recovery scan failed"),
            }
            // Covers a crash/Stop between durable completion and dispatch.
            let queued = (|| -> rusqlite::Result<Vec<String>> {
                let conn = state.db.conn();
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT session_id FROM chat_runs WHERE state='queued' LIMIT 64",
                )?;
                stmt.query_map([], |r| r.get(0))?.collect()
            })()
            .unwrap_or_else(|error| {
                tracing::error!(%error,"queue recovery scan failed");
                Vec::new()
            });
            for session in queued {
                if active.contains(&session) {
                    continue;
                }
                if let Some(run) = crate::db::realtime::claim_next_queued_run(&state.db, &session) {
                    let next = Arc::clone(&state);
                    tokio::spawn(async move {
                        super::submit_durable_chat_run(next, run).await;
                    });
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn dropped_executor_preserves_work_and_does_not_remove_replacement_channel() {
        let state = AppState::new(":memory:", 0);
        state.db.conn().execute("INSERT INTO sessions(id,project_path,created_at,updated_at,user_id) VALUES('s','/tmp','now','now','u')",[]).unwrap();
        crate::db::realtime::create_run(
            &state.db,
            "run",
            "s",
            "u",
            &serde_json::json!({"message":"finish"}),
            "preparing",
        );
        let owner = run_lifecycle::activate(&state.db, "run", "s")
            .unwrap()
            .unwrap();
        let old = broadcast::channel(4).0;
        let replacement = broadcast::channel(4).0;
        state
            .live_streams
            .lock()
            .await
            .insert("s".into(), replacement.clone());
        state.active_sessions.lock().await.insert("s".into());
        let guard = ExecutionGuard::new(
            Arc::clone(&state),
            "run".into(),
            "s".into(),
            owner.clone(),
            old,
        );
        drop(guard);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while run_lifecycle::owns(&state.db, "run", &owner) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            state
                .live_streams
                .lock()
                .await
                .get("s")
                .unwrap()
                .same_channel(&replacement)
        );
        assert!(state.active_sessions.lock().await.contains("s"));
        assert_eq!(
            run_lifecycle::recovery_metadata(&state.db, Some("run"))["completionManaged"],
            "server"
        );
        assert!(run_lifecycle::recovery_metadata(&state.db, Some("run"))["retryAt"].is_string());
    }
}
