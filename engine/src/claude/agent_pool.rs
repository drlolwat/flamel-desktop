/// Agent pool — semaphore-based concurrency control + global SSE broadcast for agent run events.
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore, broadcast};

/// Maximum number of background agent runs executing simultaneously.
pub const MAX_CONCURRENT_AGENTS: usize = 4;
const BROADCAST_CAP: usize = 256;

pub struct AgentPool {
    /// Fan-out channel — agent_runner broadcasts events here; SSE subscribers read from it.
    pub events_tx: broadcast::Sender<String>,
    /// PID registry for running agents (for SIGINT-based kill).
    running: Mutex<HashMap<String, u32>>,
    /// Backpressure: at most MAX_CONCURRENT_AGENTS running simultaneously.
    pub semaphore: Arc<Semaphore>,
}

impl AgentPool {
    pub fn new() -> Self {
        let (events_tx, _) = broadcast::channel(BROADCAST_CAP);
        Self {
            events_tx,
            running: Mutex::new(HashMap::new()),
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_AGENTS)),
        }
    }

    /// Subscribe to the agent event stream (for SSE endpoints).
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.events_tx.subscribe()
    }

    /// Broadcast a JSON event to all SSE subscribers.
    pub fn emit(&self, msg: serde_json::Value) {
        let _ = self.events_tx.send(format!("{}\n", msg));
    }

    /// Register a running agent subprocess PID.
    pub async fn register(&self, run_id: &str, pid: u32) {
        self.running.lock().await.insert(run_id.to_string(), pid);
    }

    /// Unregister a completed agent subprocess.
    pub async fn unregister(&self, run_id: &str) {
        self.running.lock().await.remove(run_id);
    }

    /// Send SIGINT to the running process for this agent run. Returns true if the PID was found.
    pub async fn kill_run(&self, run_id: &str) -> bool {
        let pid = self.running.lock().await.remove(run_id);
        if let Some(pid) = pid {
            #[cfg(unix)]
            {
                use nix::sys::signal::{self, Signal};
                use nix::unistd::Pid;
                let _ = signal::kill(Pid::from_raw(-(pid as i32)), Signal::SIGINT);
                tracing::info!(
                    "agent_pool: sent SIGINT to process group {} (run {})",
                    pid,
                    run_id
                );
                return true;
            }
            #[allow(unreachable_code)]
            false
        } else {
            false
        }
    }

    /// Stop all agent provider process groups and prevent queued runs from starting.
    pub async fn shutdown_all(&self) {
        self.semaphore.close();
        let running = {
            let mut guard = self.running.lock().await;
            guard.drain().collect::<Vec<_>>()
        };
        if running.is_empty() {
            return;
        }

        #[cfg(unix)]
        {
            use nix::sys::signal::{self, Signal};
            use nix::unistd::Pid;

            for (run_id, pid) in &running {
                let _ = signal::kill(Pid::from_raw(-(*pid as i32)), Signal::SIGTERM);
                tracing::info!(
                    "agent_pool: sent SIGTERM to process group {} (run {})",
                    pid,
                    run_id
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(750)).await;
            for (run_id, pid) in &running {
                if signal::kill(Pid::from_raw(*pid as i32), None).is_ok() {
                    let _ = signal::kill(Pid::from_raw(-(*pid as i32)), Signal::SIGKILL);
                    tracing::warn!(
                        "agent_pool: sent SIGKILL to lingering process group {} (run {})",
                        pid,
                        run_id
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_closes_agent_admission() {
        let pool = AgentPool::new();
        pool.shutdown_all().await;

        assert!(pool.semaphore.acquire().await.is_err());
    }
}
