use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Notify, broadcast};
use tokio::time::{Duration, Instant};

use super::provider::{
    ImageBlock, ProviderCredentials as ProvCreds, ProviderKind, SpawnOpts, get_provider,
};
use crate::db::Db;
mod steering;
pub use steering::{SteeringResult, SteeringTarget};

const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60); // 10 minutes
const MAX_SESSIONS: usize = 3;
const SESSION_BUSY_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const CODEX_THREAD_START_TIMEOUT: Duration = Duration::from_secs(90);
const CODEX_THREAD_START_TIMEOUT_MARKER: &str = "__flamel_codex_thread_start_timeout";
const BUNDLED_MCP_SERVER_NAMES: &[&str] = &["flamel"];
static NEXT_SESSION_GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT_TURN_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderTurnOwner {
    generation: u64,
    turn: u64,
}

fn bundled_mcp_transport_closed(event: &super::provider::NormalizedEvent) -> Option<(&str, &str)> {
    let super::provider::NormalizedEvent::McpToolCall {
        server,
        tool,
        error: Some(error),
        ..
    } = event
    else {
        return None;
    };

    (BUNDLED_MCP_SERVER_NAMES.contains(&server.as_str())
        && error.to_ascii_lowercase().contains("transport closed"))
    .then_some((server.as_str(), tool.as_str()))
}

fn codex_event_requires_active_turn(event: &super::provider::NormalizedEvent) -> bool {
    use super::provider::NormalizedEvent;

    !matches!(
        event,
        NormalizedEvent::Init { .. }
            | NormalizedEvent::Error { .. }
            | NormalizedEvent::Usage { .. }
    )
}

fn system_prompt_changed(existing: Option<&str>, requested: Option<&str>) -> bool {
    existing.filter(|prompt| !prompt.is_empty()) != requested.filter(|prompt| !prompt.is_empty())
}

struct PooledSession {
    /// Unique process generation. Background tasks must match this before
    /// mutating the logical session because a replacement can reuse its ID.
    generation: u64,
    turn_generation: u64,
    child: Child,
    stdin: Option<tokio::process::ChildStdin>,
    tx: broadcast::Sender<serde_json::Value>,
    busy: bool,
    dead: bool,
    last_used: Instant,
    created_at: Instant,
    idle_cancel: Arc<Notify>,
    busy_notify: Arc<Notify>,
    provider: ProviderKind,
    model: String,
    reasoning_effort: Option<String>,
    permission_mode: String,
    ponytail_enabled: bool,
    system_prompt: Option<String>,
    /// Codex app-server thread ID (set after thread/start or thread/resume response)
    codex_thread_id: Option<String>,
    /// Codex app-server turn ID for the currently running turn (needed for turn/interrupt)
    codex_current_turn_id: Option<String>,
    control_replies: HashMap<String, tokio::sync::oneshot::Sender<serde_json::Value>>,
    /// First message queued during Codex init (sent after thread/start or thread/resume response)
    codex_pending_turn: Option<PendingCodexTurn>,
    /// Interrupt requested before Codex exposed the current turn ID.
    codex_interrupt_pending: bool,
    /// Whether this spawn is attempting to resume an existing Codex thread.
    codex_attempted_resume: bool,
    user_home: String,
}

fn configure_provider_process(command: &mut Command) {
    crate::mcp_parent_guard::configure_tokio_child(command);
}

async fn terminate_provider_process(session: &mut PooledSession) {
    // Closing stdin first gives the CLI a brief chance to shut down its MCP
    // servers cleanly before Flamel terminates the entire process group.
    #[cfg(unix)]
    let process_group = session.child.id();
    drop(session.stdin.take());
    let leader_exited = tokio::time::timeout(Duration::from_millis(750), session.child.wait())
        .await
        .is_ok();

    #[cfg(unix)]
    if let Some(pid) = process_group {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
        if !leader_exited {
            let _ = tokio::time::timeout(Duration::from_millis(500), session.child.wait()).await;
        }
        // The leader exiting does not prove its MCP servers or grandchildren
        // exited. Finish the group cleanup using the captured original PID.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        let _ = session.child.wait().await;
        return;
    }

    if !leader_exited {
        let _ = session.child.kill().await;
    }
    let _ = session.child.wait().await;
}

#[derive(Clone)]
struct PendingCodexTurn {
    message: String,
    images: Vec<ImageBlock>,
}

fn codex_image_blocks(images: Option<&[serde_json::Value]>) -> Vec<ImageBlock> {
    let Some(images) = images else {
        return Vec::new();
    };

    images
        .iter()
        .filter_map(|img| {
            let data = img.get("data").and_then(|v| v.as_str())?.trim().to_string();
            if data.is_empty() {
                return None;
            }
            Some(ImageBlock {
                media_type: img
                    .get("mediaType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("image/png")
                    .to_string(),
                data,
            })
        })
        .collect()
}

fn codex_thread_start_timeout_error() -> String {
    format!(
        "{marker}: Codex app-server did not produce a thread ID within {seconds}s. Flamel killed the stuck provider process so the session can recover instead of waiting forever.",
        marker = CODEX_THREAD_START_TIMEOUT_MARKER,
        seconds = CODEX_THREAD_START_TIMEOUT.as_secs(),
    )
}

pub struct SessionPool {
    backend: Arc<dyn crate::execution::ExecutionBackend>,
    sessions: Mutex<HashMap<String, PooledSession>>,
    /// Maps our session ID → Claude's assigned session_id (persists across process restarts)
    claude_session_ids: Mutex<HashMap<String, String>>,
}

fn format_provider_recovery_context(
    _provider_kind: ProviderKind,
    parts: &crate::db::sessions::RecoveryContextParts,
) -> Option<String> {
    crate::db::sessions::format_recovery_context_from_parts(parts)
}

pub async fn build_recovery_context_for_provider(
    provider_kind: ProviderKind,
    _cwd: &str,
    _user_home: &str,
    _credentials: Option<&ProvCreds>,
    db: Option<&Db>,
    session_id: &str,
) -> Option<String> {
    let parts =
        db.and_then(|db| crate::db::sessions::load_session_recovery_context_parts(db, session_id))?;
    format_provider_recovery_context(provider_kind, &parts)
}

impl SessionPool {
    pub fn new() -> Self {
        Self::with_backend(Arc::new(crate::execution::LocalExecution))
    }

    pub fn with_backend(backend: Arc<dyn crate::execution::ExecutionBackend>) -> Self {
        SessionPool {
            backend,
            sessions: Mutex::new(HashMap::new()),
            claude_session_ids: Mutex::new(HashMap::new()),
        }
    }

    fn spawn_codex_thread_start_watchdog(pool: Arc<Self>, session_id: String, generation: u64) {
        tokio::spawn(async move {
            tokio::time::sleep(CODEX_THREAD_START_TIMEOUT).await;

            let timed_out_session = {
                let mut sessions = pool.sessions.lock().await;
                let should_timeout = sessions.get(&session_id).is_some_and(|session| {
                    session.generation == generation
                        && session.provider == ProviderKind::Codex
                        && !session.dead
                        && session.codex_thread_id.is_none()
                        && session.codex_pending_turn.is_some()
                });

                if should_timeout {
                    sessions.remove(&session_id)
                } else {
                    None
                }
            };

            if let Some(mut session) = timed_out_session {
                tracing::error!(
                    "codex app-server did not produce a thread ID for session {} generation {} within {}s; killing stuck process",
                    session_id,
                    generation,
                    CODEX_THREAD_START_TIMEOUT.as_secs()
                );
                session.dead = true;
                session.busy = false;
                session.stdin.take();
                session.codex_pending_turn = None;
                session.idle_cancel.notify_one();
                session.busy_notify.notify_one();

                let error = codex_thread_start_timeout_error();
                let _ = session.tx.send(serde_json::json!({
                    "type": "result",
                    "subtype": "error",
                    "is_error": true,
                    "stop_reason": "error",
                    "result": error.clone(),
                    "error": error,
                }));
                let _ = session.tx.send(serde_json::json!({"type": "__pool_dead"}));
                let _ = session.child.kill().await;
                pool.claude_session_ids.lock().await.remove(&session_id);
            }
        });
    }

    pub async fn is_session_warm(&self, session_id: &str) -> bool {
        let sessions = self.sessions.lock().await;
        sessions.get(session_id).is_some_and(|s| !s.dead)
    }

    pub async fn warm_session_ids(&self) -> HashSet<String> {
        let sessions = self.sessions.lock().await;
        sessions
            .iter()
            .filter(|(_, session)| !session.dead)
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub async fn is_session_busy(&self, session_id: &str) -> bool {
        let sessions = self.sessions.lock().await;
        sessions
            .get(session_id)
            .is_some_and(|session| session.busy && !session.dead)
    }

    pub async fn session_provider(&self, session_id: &str) -> ProviderKind {
        let sessions = self.sessions.lock().await;
        sessions
            .get(session_id)
            .map(|s| s.provider)
            .unwrap_or(ProviderKind::Codex)
    }

    pub async fn send_to_session(
        self: &Arc<Self>,
        session_id: &str,
        cwd: &str,
        message: &str,
        images: Option<&[serde_json::Value]>,
        model: Option<&str>,
        reasoning_effort: Option<&str>,
        permission_mode: &str,
        system_prompt_append: Option<&str>,
        resume_session_id: Option<&str>,
        allow_persisted_recovery_context: bool,
        mcp_servers: Option<&serde_json::Value>,
        abort_rx: tokio::sync::watch::Receiver<bool>,
        credentials: Option<&ProvCreds>,
        db: Option<&Db>,
        user_home: &str,
        skip_sandbox: bool,
        provider_kind: ProviderKind,
    ) -> broadcast::Receiver<serde_json::Value> {
        self.send_to_session_with_env(
            session_id,
            cwd,
            message,
            images,
            model,
            reasoning_effort,
            permission_mode,
            system_prompt_append,
            resume_session_id,
            allow_persisted_recovery_context,
            mcp_servers,
            abort_rx,
            credentials,
            db,
            user_home,
            skip_sandbox,
            provider_kind,
            None,
        )
        .await
    }

    /// Send a turn while adding ephemeral process-only environment values.
    /// Values supplied here are never written into provider or MCP config.
    pub async fn send_to_session_with_env(
        self: &Arc<Self>,
        session_id: &str,
        cwd: &str,
        message: &str,
        images: Option<&[serde_json::Value]>,
        model: Option<&str>,
        reasoning_effort: Option<&str>,
        permission_mode: &str,
        system_prompt_append: Option<&str>,
        resume_session_id: Option<&str>,
        allow_persisted_recovery_context: bool,
        mcp_servers: Option<&serde_json::Value>,
        abort_rx: tokio::sync::watch::Receiver<bool>,
        credentials: Option<&ProvCreds>,
        db: Option<&Db>,
        user_home: &str,
        skip_sandbox: bool,
        provider_kind: ProviderKind,
        process_env: Option<&BTreeMap<String, String>>,
    ) -> broadcast::Receiver<serde_json::Value> {
        let mut sessions = self.sessions.lock().await;
        let ponytail_enabled = system_prompt_append
            .is_some_and(|prompt| prompt.contains(crate::ponytail::ACTIVE_MARKER));
        let requested_system_prompt = system_prompt_append
            .filter(|prompt| !prompt.is_empty())
            .map(str::to_owned);
        let requested_model = model
            .map(str::to_string)
            .unwrap_or_else(|| get_provider(provider_kind).default_model().to_string());
        let requested_reasoning_effort = match provider_kind {
            ProviderKind::Codex => Some(reasoning_effort.unwrap_or("high").to_string()),
            ProviderKind::Claude => reasoning_effort.map(str::to_owned),
        };

        // Evict dead sessions (both Claude and Codex app-server stay warm)
        if let Some(session) = sessions.get_mut(session_id) {
            if session.dead {
                let cancel = session.idle_cancel.clone();
                cancel.notify_one();
                sessions.remove(session_id);
            }
        }

        let mut session_to_restart = None;
        let scoped_mcp = mcp_servers
            .and_then(|servers| servers.as_object())
            .is_some_and(|servers| {
                servers.values().any(|server| {
                    server
                        .pointer("/env/FLAMEL_TOKEN")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(crate::mcp_auth::is_scoped)
                })
            })
            || process_env
                .and_then(|env| env.get("FLAMEL_TOKEN"))
                .is_some_and(|token| crate::mcp_auth::is_scoped(token));
        if let Some(existing) = sessions.get(session_id) {
            if existing.provider != provider_kind
                || existing.model != requested_model
                || existing.reasoning_effort != requested_reasoning_effort
                || existing.permission_mode != permission_mode
                || existing.ponytail_enabled != ponytail_enabled
                || (scoped_mcp
                    && existing.created_at.elapsed()
                        >= Duration::from_secs(crate::mcp_auth::REFRESH_AFTER_SECONDS))
                || system_prompt_changed(existing.system_prompt.as_deref(), system_prompt_append)
            {
                tracing::info!(
                    "restarting warm session {} due to provider/model/reasoning/permission/Ponytail/runtime-instruction change ({}:{}:{:?}:{}:{} -> {}:{}:{:?}:{}:{})",
                    session_id,
                    existing.provider,
                    existing.model,
                    existing.reasoning_effort,
                    existing.permission_mode,
                    existing.ponytail_enabled,
                    provider_kind,
                    requested_model,
                    requested_reasoning_effort,
                    permission_mode,
                    ponytail_enabled
                );
                session_to_restart = sessions.remove(session_id);
            }
        }
        if let Some(mut session) = session_to_restart {
            session.dead = true;
            session.busy_notify.notify_one();
            session.idle_cancel.notify_one();
            drop(sessions);
            terminate_provider_process(&mut session).await;
            let _ = session.tx.send(serde_json::json!({"type": "__pool_dead"}));
            self.claude_session_ids.lock().await.remove(session_id);
            sessions = self.sessions.lock().await;
        }

        let needs_spawn = !sessions.contains_key(session_id);
        let mut pre_subscribed_rx: Option<broadcast::Receiver<serde_json::Value>> = None;

        if needs_spawn && provider_kind == ProviderKind::Codex && !user_home.is_empty() {
            let idle_codex_ids = sessions
                .iter()
                .filter(|(id, session)| {
                    id.as_str() != session_id
                        && session.provider == ProviderKind::Codex
                        && !session.busy
                        && !session.dead
                        && session.user_home.as_str() == user_home
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            let mut idle_codex_sessions = idle_codex_ids
                .into_iter()
                .filter_map(|id| sessions.remove(&id).map(|session| (id, session)))
                .collect::<Vec<_>>();

            if !idle_codex_sessions.is_empty() {
                drop(sessions);
                for (id, mut session) in idle_codex_sessions.drain(..) {
                    tracing::info!(
                        "reaping idle Codex app-server {} before starting another session for the same user home",
                        id
                    );
                    session.dead = true;
                    session.idle_cancel.notify_one();
                    session.busy_notify.notify_one();
                    let _ = session.child.kill().await;
                    let _ = session.tx.send(serde_json::json!({"type": "__pool_dead"}));
                    self.claude_session_ids.lock().await.remove(&id);
                }
                sessions = self.sessions.lock().await;
            }
        }

        if needs_spawn {
            // Evict least-recently-used idle sessions if we're at capacity
            while sessions.len() >= MAX_SESSIONS {
                let lru_idle = sessions
                    .iter()
                    .filter(|(_, s)| !s.busy)
                    .min_by_key(|(_, s)| s.last_used)
                    .map(|(id, _)| id.clone());
                if let Some(evict_id) = lru_idle {
                    tracing::info!(
                        "evicting LRU session {} to stay under cap ({})",
                        evict_id,
                        MAX_SESSIONS
                    );
                    if let Some(mut s) = sessions.remove(&evict_id) {
                        s.dead = true;
                        s.idle_cancel.notify_one();
                        let _ = s.child.kill().await;
                        let _ = s.tx.send(serde_json::json!({"type": "__pool_dead"}));
                    }
                } else {
                    // All sessions busy — can't evict, just proceed over cap
                    break;
                }
            }

            // Check for provider resume ID.
            //
            // Both Claude and Codex can resume persisted provider-side conversations across
            // fresh spawns when we have a stored session/thread identifier.
            let stored_resume_id = {
                let ids = self.claude_session_ids.lock().await;
                ids.get(session_id).cloned()
            };
            let effective_resume_id = match provider_kind {
                ProviderKind::Claude | ProviderKind::Codex => {
                    stored_resume_id.as_deref().or(resume_session_id)
                }
            };

            // When a session must start fresh without a valid provider resume ID,
            // inject persisted context so the model can continue without re-deriving
            // the task from scratch.
            let recovery_context =
                if allow_persisted_recovery_context && effective_resume_id.is_none() {
                    build_recovery_context_for_provider(
                        provider_kind,
                        cwd,
                        user_home,
                        credentials,
                        db,
                        session_id,
                    )
                    .await
                } else {
                    None
                };
            let effective_system_prompt_owned =
                recovery_context
                    .as_deref()
                    .map(|context| match system_prompt_append {
                        Some(sp) if !sp.trim().is_empty() => format!("{sp}\n\n{context}"),
                        _ => context.to_string(),
                    });
            let effective_system_prompt = effective_system_prompt_owned
                .as_deref()
                .or(system_prompt_append);

            let provider = get_provider(provider_kind);
            let cli = match provider.find_binary() {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("Failed to find {} CLI: {e}", provider_kind);
                    let (tx, rx) = broadcast::channel(1);
                    let _ = tx.send(serde_json::json!({"type": "__pool_dead"}));
                    return rx;
                }
            };

            let provider_base_url = provider_base_url_for_launch(provider_kind, process_env);
            let opts = SpawnOpts {
                model,
                reasoning_effort,
                permission_mode,
                system_prompt_append: effective_system_prompt,
                resume_session_id: effective_resume_id,
                mcp_servers,
                user_home,
                session_id,
                provider_base_url,
                message: Some(message),
            };
            let args = provider.build_args(&opts);

            let mut env_vars =
                provider.build_env(credentials, db, user_home, mcp_servers, reasoning_effort);
            let isolated_process = process_env.is_some();
            if let Some(process_env) = process_env {
                env_vars = isolated_provider_process_env(env_vars, process_env);
            } else {
                env_vars = self.backend.environment(
                    env_vars,
                    mcp_servers,
                    user_home,
                    provider_kind,
                    db,
                    credentials.is_some(),
                );
            }

            tracing::info!(
                "spawning {} CLI for session {} (resume: {:?})",
                provider_kind,
                session_id,
                effective_resume_id
            );
            let mut cmd = match self
                .backend
                .command(&cli, &args, user_home, cwd, process_env)
            {
                Ok(command) => command,
                Err(error) => {
                    let (tx, rx) = broadcast::channel(2);
                    let _=tx.send(serde_json::json!({"type":"result","is_error":true,"error":error,"stop_reason":"error"}));
                    let _ = tx.send(serde_json::json!({"type":"__pool_dead"}));
                    return rx;
                }
            };
            cmd.current_dir(cwd);
            cmd.stdin(std::process::Stdio::piped());
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());
            if isolated_process || !self.backend.inherit_ambient_environment() {
                cmd.env_clear();
            }
            cmd.envs(env_vars);
            configure_provider_process(&mut cmd);

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Failed to spawn Claude CLI at {:?}: {e}", cli);
                    let (tx, rx) = broadcast::channel(1);
                    let _ = tx.send(serde_json::json!({"type": "__pool_dead"}));
                    return rx;
                }
            };
            let mut stdin = match child.stdin.take() {
                Some(s) => s,
                None => {
                    tracing::error!("Failed to get stdin pipe from Claude CLI process");
                    let _ = child.kill().await;
                    let (tx, rx) = broadcast::channel(1);
                    let _ = tx.send(serde_json::json!({"type": "__pool_dead"}));
                    return rx;
                }
            };
            let stdout = match child.stdout.take() {
                Some(s) => s,
                None => {
                    tracing::error!("Failed to get stdout pipe from Claude CLI process");
                    let _ = child.kill().await;
                    let (tx, rx) = broadcast::channel(1);
                    let _ = tx.send(serde_json::json!({"type": "__pool_dead"}));
                    return rx;
                }
            };
            let stderr = match child.stderr.take() {
                Some(s) => s,
                None => {
                    tracing::error!("Failed to get stderr pipe from Claude CLI process");
                    let _ = child.kill().await;
                    let (tx, rx) = broadcast::channel(1);
                    let _ = tx.send(serde_json::json!({"type": "__pool_dead"}));
                    return rx;
                }
            };

            let (tx, _) = broadcast::channel(2048);
            let created_rx = tx.subscribe();
            let idle_cancel = Arc::new(Notify::new());

            // Codex app-server: write init sequence + thread/start or thread/resume, queue first message
            let codex_pending_turn = if provider_kind == ProviderKind::Codex {
                use super::codex_provider;
                let init_bytes = codex_provider::build_init_sequence();
                let thread_start_bytes = if let Some(thread_id) = effective_resume_id {
                    codex_provider::build_thread_resume(
                        thread_id,
                        model,
                        cwd,
                        effective_system_prompt,
                        reasoning_effort,
                        permission_mode,
                    )
                } else {
                    codex_provider::build_thread_start(
                        model,
                        cwd,
                        effective_system_prompt,
                        reasoning_effort,
                        permission_mode,
                    )
                };
                if let Err(e) = stdin.write_all(&init_bytes).await {
                    tracing::error!("failed to write codex init sequence: {e}");
                }
                if let Err(e) = stdin.write_all(&thread_start_bytes).await {
                    tracing::error!("failed to write codex thread/start: {e}");
                }
                if let Err(e) = stdin.flush().await {
                    tracing::error!("failed to flush codex init: {e}");
                }
                tracing::info!(
                    "wrote codex app-server init + thread/start for session {}",
                    session_id
                );
                Some(PendingCodexTurn {
                    message: message.to_string(),
                    images: codex_image_blocks(images),
                })
            } else {
                None
            };

            let generation = NEXT_SESSION_GENERATION.fetch_add(1, Ordering::Relaxed);
            let session = PooledSession {
                generation,
                child,
                stdin: Some(stdin),
                tx: tx.clone(),
                busy: false,
                turn_generation: 0,
                dead: false,
                last_used: Instant::now(),
                created_at: Instant::now(),
                idle_cancel: idle_cancel.clone(),
                busy_notify: Arc::new(Notify::new()),
                provider: provider_kind,
                model: requested_model.clone(),
                reasoning_effort: requested_reasoning_effort.clone(),
                permission_mode: permission_mode.to_string(),
                ponytail_enabled,
                system_prompt: requested_system_prompt,
                codex_thread_id: None,
                codex_current_turn_id: None,
                control_replies: HashMap::new(),
                codex_pending_turn,
                codex_interrupt_pending: false,
                codex_attempted_resume: provider_kind == ProviderKind::Codex
                    && effective_resume_id.is_some(),
                user_home: user_home.to_string(),
            };

            sessions.insert(session_id.to_string(), session);
            pre_subscribed_rx = Some(created_rx);
            if provider_kind == ProviderKind::Codex {
                Self::spawn_codex_thread_start_watchdog(
                    Arc::clone(self),
                    session_id.to_string(),
                    generation,
                );
            }

            // Spawn stdout reader task
            let tx_clone = tx.clone();
            let sid = session_id.to_string();
            let pool = Arc::clone(self);
            let reader_provider_kind = provider_kind;
            let reader_generation = generation;
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                // For non-Claude providers, create a provider instance to normalize events
                let normalizer = if reader_provider_kind != ProviderKind::Claude {
                    Some(get_provider(reader_provider_kind))
                } else {
                    None
                };

                while let Ok(Some(line)) = lines.next_line().await {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    if reader_provider_kind == ProviderKind::Codex
                        && pool
                            .consume_control_reply(&sid, reader_generation, trimmed)
                            .await
                    {
                        continue;
                    }
                    if let Some(ref prov) = normalizer {
                        // Non-Claude: parse → normalize → re-serialize as Claude-format JSON
                        if let Some(normalized) = prov.parse_event(trimmed) {
                            // Codex app-server: handle thread/start or thread/resume response and
                            // send the first queued turn once the active thread ID is available.
                            if reader_provider_kind == ProviderKind::Codex {
                                if let super::provider::NormalizedEvent::Init { ref session_id } =
                                    normalized
                                {
                                    let thread_id = session_id.clone();
                                    let mut resumed_existing_thread = false;
                                    let mut sessions = pool.sessions.lock().await;
                                    if let Some(session) = sessions
                                        .get_mut(&sid)
                                        .filter(|session| session.generation == reader_generation)
                                    {
                                        resumed_existing_thread = session.codex_attempted_resume;
                                        session.codex_attempted_resume = false;
                                        session.codex_thread_id = Some(thread_id.clone());
                                        // Send the queued first turn
                                        if let Some(pending_turn) =
                                            session.codex_pending_turn.take()
                                        {
                                            let turn_model = session.model.clone();
                                            let turn_reasoning = session.reasoning_effort.clone();
                                            let turn_permission = session.permission_mode.clone();
                                            let turn_bytes =
                                                super::codex_provider::build_turn_start(
                                                    &thread_id,
                                                    &pending_turn.message,
                                                    &pending_turn.images,
                                                    Some(&turn_model),
                                                    turn_reasoning.as_deref(),
                                                    &turn_permission,
                                                );
                                            if let Some(ref mut stdin) = session.stdin {
                                                if let Err(e) = stdin.write_all(&turn_bytes).await {
                                                    tracing::error!(
                                                        "failed to write first codex turn/start: {e}"
                                                    );
                                                }
                                                let _ = stdin.flush().await;
                                            }
                                            tracing::info!(
                                                "sent first codex turn/start on thread {}",
                                                thread_id
                                            );
                                        }
                                    }
                                    drop(sessions);
                                    if resumed_existing_thread {
                                        let _ = tx_clone.send(serde_json::json!({
                                            "type": "__codex_resume_succeeded",
                                            "thread_id": thread_id,
                                        }));
                                    }
                                }
                                // Codex provider-private state updates and approval requests.
                                if let super::provider::NormalizedEvent::Raw { ref data } =
                                    normalized
                                {
                                    if data
                                        .get("__codex_turn_started")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false)
                                    {
                                        if let Some(turn_id) =
                                            data.get("turn_id").and_then(|v| v.as_str())
                                        {
                                            let should_send_pending_interrupt = {
                                                let mut sessions = pool.sessions.lock().await;
                                                if let Some(session) =
                                                    sessions.get_mut(&sid).filter(|session| {
                                                        session.generation == reader_generation
                                                    })
                                                {
                                                    session.codex_current_turn_id =
                                                        Some(turn_id.to_string());
                                                    session.codex_interrupt_pending
                                                } else {
                                                    false
                                                }
                                            };
                                            if should_send_pending_interrupt {
                                                if pool.sigint_session(&sid).await {
                                                    tracing::info!(
                                                        "sent pending turn/interrupt to codex session {}",
                                                        sid
                                                    );
                                                }
                                            }
                                            tracing::debug!(
                                                "tracked codex turn {} for session {}",
                                                turn_id,
                                                sid
                                            );
                                        }
                                        continue;
                                    }
                                    if data
                                        .get("__codex_approval_request")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false)
                                    {
                                        let mut should_forward = true;
                                        let mut response_bytes: Option<Vec<u8>> = None;
                                        if let (Some(method), Some(request_id), Some(params)) = (
                                            data.get("method").and_then(|v| v.as_str()),
                                            data.get("request_id"),
                                            data.get("params"),
                                        ) {
                                            let sessions = pool.sessions.lock().await;
                                            if let Some(session) =
                                                sessions.get(&sid).filter(|session| {
                                                    session.generation == reader_generation
                                                })
                                            {
                                                if matches!(
                                                    session.permission_mode.as_str(),
                                                    "bypassPermissions" | "never"
                                                ) {
                                                    response_bytes = super::codex_provider::build_approval_response(
                                                        method,
                                                        request_id,
                                                        params,
                                                        true,
                                                    );
                                                    should_forward = false;
                                                }
                                            }
                                        }
                                        if let Some(bytes) = response_bytes {
                                            pool.write_raw_to_stdin(&sid, bytes).await;
                                            tracing::info!(
                                                "auto-approved codex request for session {}",
                                                sid
                                            );
                                        }
                                        if !should_forward {
                                            continue;
                                        }
                                    }
                                    if data
                                        .get("__codex_resume_rejected")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false)
                                    {
                                        let error = data
                                            .get("error")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("Codex rejected thread/resume");
                                        tracing::info!(
                                            "codex resume rejected for session {} — forwarding fresh-start fallback: {}",
                                            sid,
                                            error
                                        );
                                        pool.clear_claude_session_id(&sid).await;
                                        let _ = tx_clone.send(serde_json::json!({
                                            "type": "__codex_resume_rejected",
                                            "error": error,
                                        }));
                                        continue;
                                    }
                                    if data
                                        .get("__codex_thread_closed")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false)
                                    {
                                        let thread_id = data
                                            .get("thread_id")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        tracing::info!(
                                            "codex thread closed for session {} ({}) — forwarding to run_session (not killing)",
                                            sid,
                                            if thread_id.is_empty() {
                                                "unknown"
                                            } else {
                                                thread_id
                                            }
                                        );
                                        pool.clear_claude_session_id(&sid).await;
                                        let _ = tx_clone.send(serde_json::json!({
                                            "type": "__codex_thread_closed",
                                            "thread_id": thread_id,
                                        }));
                                        continue;
                                    }
                                }
                            }
                            if reader_provider_kind == ProviderKind::Codex
                                && codex_event_requires_active_turn(&normalized)
                            {
                                let has_active_turn = {
                                    let sessions = pool.sessions.lock().await;
                                    sessions
                                        .get(&sid)
                                        .filter(|session| session.generation == reader_generation)
                                        .and_then(|session| session.codex_current_turn_id.as_ref())
                                        .is_some()
                                };
                                if !has_active_turn {
                                    tracing::debug!(
                                        "discarded pre-turn Codex replay event for session {}",
                                        sid
                                    );
                                    continue;
                                }
                            }
                            if reader_provider_kind == ProviderKind::Codex
                                && matches!(
                                    &normalized,
                                    super::provider::NormalizedEvent::Result { .. }
                                )
                            {
                                let mut sessions = pool.sessions.lock().await;
                                if let Some(session) = sessions
                                    .get_mut(&sid)
                                    .filter(|session| session.generation == reader_generation)
                                {
                                    session.codex_current_turn_id = None;
                                }
                            }
                            if reader_provider_kind == ProviderKind::Codex {
                                if let Some((server, tool)) =
                                    bundled_mcp_transport_closed(&normalized)
                                {
                                    tracing::warn!(
                                        "bundled MCP transport closed for session {} ({}/{}); restarting Codex and all sidecars",
                                        sid,
                                        server,
                                        tool
                                    );
                                    let _ = tx_clone.send(serde_json::json!({
                                        "type": "__pool_dead",
                                        "reason": "mcp_transport_closed",
                                        "server": server,
                                        "tool": tool,
                                    }));
                                    continue;
                                }
                            }
                            if reader_provider_kind == ProviderKind::Codex {
                                if let super::provider::NormalizedEvent::McpToolCall {
                                    server,
                                    tool,
                                    arguments,
                                    status,
                                    id,
                                    ..
                                } = &normalized
                                {
                                    if status != "in_progress"
                                        && status != "inProgress"
                                        && !arguments.is_null()
                                    {
                                        let input_snapshot =
                                            super::provider::NormalizedEvent::McpToolCall {
                                                server: server.clone(),
                                                tool: tool.clone(),
                                                arguments: arguments.clone(),
                                                result: None,
                                                error: None,
                                                status: "in_progress".to_string(),
                                                id: id.clone(),
                                            };
                                        if let Some(claude_json) =
                                            prov.serialize_for_frontend(&input_snapshot)
                                        {
                                            let _ = tx_clone.send(claude_json);
                                        }
                                    }
                                }
                            }

                            if let Some(claude_json) = prov.serialize_for_frontend(&normalized) {
                                let msg_type = claude_json
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                tracing::debug!(
                                    "CLI stdout [{}] ({}→claude): type={}",
                                    sid,
                                    reader_provider_kind,
                                    msg_type
                                );
                                let _ = tx_clone.send(claude_json);
                            }
                        }
                    } else {
                        // Claude: forward raw JSON as before
                        match serde_json::from_str::<serde_json::Value>(trimmed) {
                            Ok(msg) => {
                                let msg_type = msg
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                tracing::debug!("CLI stdout [{}]: type={}", sid, msg_type);
                                let receivers = tx_clone.send(msg);
                                tracing::debug!(
                                    "broadcast sent to {} receivers",
                                    receivers.unwrap_or(0)
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "CLI stdout [{}]: non-JSON line ({}): {}",
                                    sid,
                                    e,
                                    &trimmed[..trimmed.len().min(200)]
                                );
                            }
                        }
                    }
                }

                // Process exited — mark dead and wake any busy-waiters
                tracing::info!(
                    "CLI process exited for session {} generation {}",
                    sid,
                    reader_generation
                );
                let mut sessions = pool.sessions.lock().await;
                let is_current_generation = sessions
                    .get(&sid)
                    .is_some_and(|session| session.generation == reader_generation);
                if let Some(s) = sessions
                    .get_mut(&sid)
                    .filter(|session| session.generation == reader_generation)
                {
                    s.dead = true;
                    s.busy_notify.notify_one(); // unblock send_to_session busy-waits
                }
                drop(sessions);
                if is_current_generation {
                    let _ = tx_clone.send(serde_json::json!({"type": "__pool_dead"}));
                } else {
                    tracing::debug!(
                        "ignored stale CLI exit for session {} generation {}",
                        sid,
                        reader_generation
                    );
                }
            });

            // Spawn stderr reader task
            let sid2 = session_id.to_string();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        tracing::warn!("CLI stderr [{}]: {}", sid2, trimmed);
                    }
                }
            });

            // Spawn idle timer task. This must only kill sessions that have been
            // idle for the full timeout; long active tool runs can exceed it.
            let pool = Arc::clone(self);
            let sid = session_id.to_string();
            let idle_generation = generation;
            tokio::spawn(async move {
                let mut sleep_for = IDLE_TIMEOUT;
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(sleep_for) => {
                            let next_sleep = {
                                let sessions = pool.sessions.lock().await;
                                match sessions.get(&sid) {
                                    Some(session)
                                        if session.generation != idle_generation =>
                                    {
                                        None
                                    }
                                    Some(session) if session.dead => None,
                                    Some(session) if session.busy => Some(IDLE_TIMEOUT),
                                    Some(session) => {
                                        let idle_for = Instant::now().duration_since(session.last_used);
                                        if idle_for >= IDLE_TIMEOUT {
                                            None
                                        } else {
                                            Some(IDLE_TIMEOUT - idle_for)
                                        }
                                    }
                                    None => None,
                                }
                            };

                            match next_sleep {
                                Some(delay) => sleep_for = delay,
                                None => {
                                    if pool
                                        .kill_session_generation(&sid, idle_generation)
                                        .await
                                    {
                                        tracing::info!(
                                            "idle timeout — killed session {} generation {}",
                                            sid,
                                            idle_generation
                                        );
                                    }
                                    break;
                                }
                            }
                        }
                        _ = idle_cancel.notified() => {
                            break;
                        }
                    }
                }
            });
        } else {
            tracing::info!("reusing warm CLI for session {}", session_id);
        }

        // Wait until the session finishes its current turn before writing.
        // Release the HashMap lock while waiting to avoid deadlock. A session that remains
        // busy past the handoff deadline is killed; writing another turn into it is unsafe.
        loop {
            let session = match sessions.get_mut(session_id) {
                Some(s) => s,
                None => break, // session was killed by kill_session while we waited
            };
            if !session.busy {
                break;
            }
            tracing::info!(
                "session {} is busy — waiting for current turn to finish",
                session_id
            );
            let busy_notify = session.busy_notify.clone();
            drop(sessions);
            let timed_out = tokio::time::timeout(SESSION_BUSY_WAIT_TIMEOUT, busy_notify.notified())
                .await
                .is_err();
            sessions = self.sessions.lock().await;
            if timed_out {
                tracing::warn!(
                    "session {} remained busy for {}s — killing the stuck provider",
                    session_id,
                    SESSION_BUSY_WAIT_TIMEOUT.as_secs(),
                );
                let receiver = sessions
                    .get(session_id)
                    .map(|session| session.tx.subscribe());
                drop(sessions);
                self.kill_session(session_id).await;
                return receiver.unwrap_or_else(|| {
                    let (tx, rx) = broadcast::channel(1);
                    let _ = tx.send(serde_json::json!({"type": "__pool_dead"}));
                    rx
                });
            }
        }

        // If session was removed (killed) during busy-wait, return a dead channel so the
        // stream handler receives __pool_dead and ends gracefully. The next message will
        // spawn a fresh process.
        if !sessions.contains_key(session_id) {
            let (tx, rx) = broadcast::channel(1);
            let _ = tx.send(serde_json::json!({"type": "__pool_dead"}));
            return rx;
        }

        // Build the stdin message — format depends on the provider
        if let Some(session) = sessions.get_mut(session_id) {
            session.permission_mode = permission_mode.to_string();
        }
        let session_provider = sessions
            .get(session_id)
            .map(|s| s.provider)
            .unwrap_or(ProviderKind::Codex);
        let msg_bytes = match session_provider {
            ProviderKind::Claude => {
                // Claude stream-json format
                let stdin_content = if let Some(imgs) = images {
                    if !imgs.is_empty() {
                        let mut parts = Vec::new();
                        if !message.is_empty() {
                            parts.push(serde_json::json!({"type": "text", "text": message}));
                        }
                        for img in imgs {
                            parts.push(serde_json::json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": img.get("mediaType").and_then(|v| v.as_str()).unwrap_or("image/png"),
                                    "data": img.get("data").and_then(|v| v.as_str()).unwrap_or(""),
                                }
                            }));
                        }
                        serde_json::json!(parts)
                    } else {
                        serde_json::json!(message)
                    }
                } else {
                    serde_json::json!(message)
                };
                let stdin_msg = serde_json::json!({
                    "type": "user",
                    "message": {"role": "user", "content": stdin_content}
                });
                match serde_json::to_vec(&stdin_msg) {
                    Ok(mut b) => {
                        b.push(b'\n');
                        b
                    }
                    Err(e) => {
                        tracing::error!("Failed to serialize stdin message: {e}");
                        return sessions
                            .get(&session_id.to_string())
                            .map(|s| s.tx.subscribe())
                            .unwrap_or_else(|| {
                                let (tx, rx) = broadcast::channel(1);
                                let _ = tx.send(serde_json::json!({"type": "__pool_dead"}));
                                rx
                            });
                    }
                }
            }
            ProviderKind::Codex => {
                // Codex app-server: Turn/start JSON-RPC on existing thread
                let image_blocks = codex_image_blocks(images);
                if let Some(session) = sessions.get_mut(session_id) {
                    if let Some(thread_id) = session.codex_thread_id.clone() {
                        session.codex_current_turn_id = None;
                        super::codex_provider::build_turn_start(
                            &thread_id,
                            message,
                            &image_blocks,
                            Some(&requested_model),
                            requested_reasoning_effort.as_deref(),
                            permission_mode,
                        )
                    } else {
                        // Thread not ready yet — queue as pending (shouldn't happen for warm sessions)
                        tracing::warn!(
                            "codex thread_id not yet available for session {}",
                            session_id
                        );
                        session.codex_pending_turn = Some(PendingCodexTurn {
                            message: message.to_string(),
                            images: image_blocks,
                        });
                        Vec::new()
                    }
                } else {
                    // Thread not ready yet — queue as pending (shouldn't happen for warm sessions)
                    tracing::warn!(
                        "codex thread_id not yet available for session {}",
                        session_id
                    );
                    if let Some(s) = sessions.get_mut(session_id) {
                        s.codex_pending_turn = Some(PendingCodexTurn {
                            message: message.to_string(),
                            images: image_blocks,
                        });
                    }
                    Vec::new()
                }
            }
        };

        let session = match sessions.get_mut(session_id) {
            Some(s) => s,
            None => {
                let (tx, rx) = broadcast::channel(1);
                let _ = tx.send(serde_json::json!({"type": "__pool_dead"}));
                return rx;
            }
        };
        session.last_used = Instant::now();
        session.busy = true;
        session.turn_generation = NEXT_TURN_GENERATION.fetch_add(1, Ordering::Relaxed);

        // CRITICAL: Subscribe BEFORE writing to stdin so we don't miss early responses.
        // For newly spawned Codex app-server sessions, subscribe before the stdout reader
        // starts broadcasting init/resume events.
        let rx = pre_subscribed_rx.unwrap_or_else(|| session.tx.subscribe());

        // Write to stdin — Claude uses stdin; Codex uses CLI args (stdin already closed)
        if !msg_bytes.is_empty() {
            if let Some(ref mut stdin) = session.stdin {
                tracing::info!(
                    "writing {} bytes to stdin for session {}",
                    msg_bytes.len(),
                    session_id
                );
                if let Err(e) = stdin.write_all(&msg_bytes).await {
                    tracing::error!("failed to write to stdin: {}", e);
                }
                if let Err(e) = stdin.flush().await {
                    tracing::error!("failed to flush stdin: {}", e);
                }
            }
        }

        // Drop the lock — we have rx and don't need the session anymore
        drop(sessions);

        // Abort watcher — run_session handles the abort gracefully (silences output,
        // waits for Claude to finish its turn so the session stays warm).
        // We no longer kill here; run_session has a 30s deadline fallback.
        let _ = abort_rx; // consumed — run_session receives its own clone

        rx
    }

    /// Write raw bytes directly to an active session's stdin.
    /// Used to inject tool_result messages for built-in tools (e.g. AskUserQuestion fallback).
    /// Returns true if the write succeeded.
    pub async fn write_raw_to_stdin(&self, session_id: &str, mut data: Vec<u8>) -> bool {
        use tokio::io::AsyncWriteExt;
        if !data.ends_with(b"\n") {
            data.push(b'\n');
        }
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(session_id) {
            if session.dead {
                return false;
            }
            if let Some(ref mut stdin) = session.stdin {
                if let Err(e) = stdin.write_all(&data).await {
                    tracing::error!("write_raw_to_stdin: write failed for {}: {}", session_id, e);
                    return false;
                }
                if let Err(e) = stdin.flush().await {
                    tracing::error!("write_raw_to_stdin: flush failed for {}: {}", session_id, e);
                    return false;
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Send SIGINT to the Claude process (or Turn/interrupt to Codex) for this session.
    /// Claude will abort its in-flight API request and emit a `result` event,
    /// leaving the session warm and ready for the next message.
    pub async fn sigint_session(&self, session_id: &str) -> bool {
        self.sigint_owned_turn(session_id, self.turn_owner(session_id).await)
            .await
    }

    pub async fn sigint_owned_turn(
        &self,
        session_id: &str,
        owner: Option<ProviderTurnOwner>,
    ) -> bool {
        let Some(owner) = owner else {
            return false;
        };
        enum InterruptTarget {
            Codex {
                thread_id: String,
                turn_id: String,
                stdin: tokio::process::ChildStdin,
            },
            Claude {
                pid: u32,
            },
        }

        let target = {
            let mut sessions = self.sessions.lock().await;
            let Some(session) = sessions.get_mut(session_id) else {
                return false;
            };

            if session.generation != owner.generation || session.turn_generation != owner.turn {
                return false;
            }
            if session.provider == ProviderKind::Codex {
                let Some(thread_id) = session.codex_thread_id.clone() else {
                    session.codex_interrupt_pending = true;
                    tracing::warn!("codex interrupt requested before a thread ID was available");
                    return true;
                };
                let Some(turn_id) = session.codex_current_turn_id.clone() else {
                    session.codex_interrupt_pending = true;
                    tracing::warn!("codex interrupt queued until a turn ID is available");
                    return true;
                };
                session.codex_interrupt_pending = false;
                let Some(stdin) = session.stdin.take() else {
                    return false;
                };
                InterruptTarget::Codex {
                    thread_id,
                    turn_id,
                    stdin,
                }
            } else if let Some(pid) = session.child.id() {
                InterruptTarget::Claude { pid }
            } else {
                return false;
            }
        };

        match target {
            InterruptTarget::Codex {
                thread_id,
                turn_id,
                mut stdin,
            } => {
                let interrupt_bytes =
                    super::codex_provider::build_turn_interrupt(&thread_id, &turn_id);
                let write_ok = if let Err(e) = stdin.write_all(&interrupt_bytes).await {
                    tracing::error!("failed to write codex turn/interrupt: {e}");
                    false
                } else if let Err(e) = stdin.flush().await {
                    tracing::error!("failed to flush codex turn/interrupt: {e}");
                    false
                } else {
                    true
                };

                // Release the global pool lock while writing to one session so
                // interrupting a Codex turn does not stall unrelated sessions.
                let mut sessions = self.sessions.lock().await;
                if let Some(session) = sessions.get_mut(session_id) {
                    if session.generation == owner.generation
                        && session.turn_generation == owner.turn
                        && session.stdin.is_none()
                    {
                        session.stdin = Some(stdin);
                    }
                }

                if write_ok {
                    tracing::info!("sent turn/interrupt to codex session {}", session_id);
                }
                write_ok
            }
            InterruptTarget::Claude { pid } => {
                #[cfg(unix)]
                {
                    use std::process::Command;
                    let _ = Command::new("kill")
                        .args(["-s", "INT", &pid.to_string()])
                        .output();
                    let _ = Command::new("pkill")
                        .args(["-INT", "-P", &pid.to_string()])
                        .output();
                    tracing::info!(
                        "sent SIGINT to session {} pid {} + children",
                        session_id,
                        pid
                    );
                    true
                }
                #[cfg(not(unix))]
                {
                    let _ = pid;
                    tracing::warn!("sigint_session: SIGINT not supported on this platform");
                    false
                }
            }
        }
    }

    pub async fn kill_session(&self, session_id: &str) {
        let mut session = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions.remove(session_id);
            if session.is_some() {
                self.claude_session_ids.lock().await.remove(session_id);
            }
            session
        };
        if let Some(ref mut session) = session {
            session.dead = true;
            session.busy_notify.notify_one(); // wake any send_to_session busy-waiting
            session.idle_cancel.notify_one();
            terminate_provider_process(session).await;
            let _ = session.tx.send(serde_json::json!({"type": "__pool_dead"}));
        }
    }

    /// Kill only the process generation that scheduled the cleanup. A logical
    /// session ID can already point at a replacement by the time an old
    /// watchdog, stdout reader, or idle timer wakes up.
    async fn kill_session_generation(&self, session_id: &str, generation: u64) -> bool {
        let mut session = {
            let mut sessions = self.sessions.lock().await;
            let is_current = sessions.get(session_id).is_some_and(|session| {
                session.generation == generation
                    && !session.busy
                    && session.last_used.elapsed() >= IDLE_TIMEOUT
            });
            if !is_current {
                return false;
            }
            let session = sessions.remove(session_id);
            self.claude_session_ids.lock().await.remove(session_id);
            session
        };
        let Some(ref mut session) = session else {
            return false;
        };
        session.dead = true;
        session.busy_notify.notify_one();
        session.idle_cancel.notify_one();
        terminate_provider_process(session).await;
        let _ = session.tx.send(serde_json::json!({"type": "__pool_dead"}));
        true
    }

    pub async fn set_claude_session_id(&self, our_id: &str, claude_id: &str) {
        let mut ids = self.claude_session_ids.lock().await;
        ids.insert(our_id.to_string(), claude_id.to_string());
    }

    pub async fn clear_claude_session_id(&self, our_id: &str) {
        let mut ids = self.claude_session_ids.lock().await;
        ids.remove(our_id);
    }

    pub async fn turn_owner(&self, session_id: &str) -> Option<ProviderTurnOwner> {
        self.sessions
            .lock()
            .await
            .get(session_id)
            .map(|s| ProviderTurnOwner {
                generation: s.generation,
                turn: s.turn_generation,
            })
    }

    pub async fn kill_owned_turn(
        &self,
        session_id: &str,
        owner: Option<ProviderTurnOwner>,
    ) -> bool {
        let Some(owner) = owner else {
            return false;
        };
        let mut session = {
            let mut sessions = self.sessions.lock().await;
            if !sessions.get(session_id).is_some_and(|s| {
                s.generation == owner.generation && s.turn_generation == owner.turn
            }) {
                return false;
            }
            let session = sessions.remove(session_id);
            self.claude_session_ids.lock().await.remove(session_id);
            session
        };
        if let Some(ref mut session) = session {
            session.dead = true;
            session.busy_notify.notify_one();
            session.idle_cancel.notify_one();
            terminate_provider_process(session).await;
            let _ = session.tx.send(serde_json::json!({"type":"__pool_dead"}));
        }
        true
    }

    // Compatibility for the separately serialized personal-assistant driver.
    pub async fn mark_session_idle(&self, session_id: &str) {
        if let Some(owner) = self.turn_owner(session_id).await {
            self.mark_owned_turn_idle(session_id, owner).await;
        }
    }

    pub async fn mark_owned_turn_idle(&self, session_id: &str, owner: ProviderTurnOwner) {
        let mut sessions = self.sessions.lock().await;
        if let Some(s) = sessions
            .get_mut(session_id)
            .filter(|s| s.generation == owner.generation && s.turn_generation == owner.turn)
        {
            s.busy = false;
            s.last_used = Instant::now();
            if s.provider == ProviderKind::Codex {
                s.codex_current_turn_id = None;
                s.codex_interrupt_pending = false;
            }
            s.busy_notify.notify_one();
        }
    }

    pub async fn kill_idle_sessions(&self) {
        let mut sessions = self.sessions.lock().await;
        let now = Instant::now();
        let grace = Duration::from_secs(120);
        let idle_ids: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| !s.busy && now.duration_since(s.last_used) > grace)
            .map(|(id, _)| id.clone())
            .collect();
        if idle_ids.is_empty() {
            return;
        }
        tracing::info!(
            "memory watchdog: killing {} sessions idle for >{}s",
            idle_ids.len(),
            grace.as_secs()
        );
        for id in idle_ids {
            if let Some(mut s) = sessions.remove(&id) {
                s.dead = true;
                s.idle_cancel.notify_one();
                terminate_provider_process(&mut s).await;
                let _ = s.tx.send(serde_json::json!({"type": "__pool_dead"}));
                tracing::info!("killed idle session {}", id);
            }
        }
    }

    pub async fn shutdown_all(&self) {
        let mut sessions = self.sessions.lock().await;
        for (_, mut session) in sessions.drain() {
            session.dead = true;
            session.busy_notify.notify_one();
            session.idle_cancel.notify_one();
            let _ = session.tx.send(serde_json::json!({
                "type": "__pool_dead",
                "reason": "server_restart"
            }));
            terminate_provider_process(&mut session).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stuck_busy_sessions_are_bounded_to_fifteen_seconds() {
        assert_eq!(SESSION_BUSY_WAIT_TIMEOUT, Duration::from_secs(15));
    }

    #[test]
    fn warm_sessions_restart_when_runtime_instructions_change() {
        assert!(!system_prompt_changed(None, None));
        assert!(!system_prompt_changed(None, Some("")));
        assert!(!system_prompt_changed(Some("current"), Some("current")));
        assert!(system_prompt_changed(Some("old"), Some("current")));
        assert!(system_prompt_changed(None, Some("current")));
    }

    #[test]
    fn codex_turn_output_is_gated_until_turn_start() {
        use super::super::provider::NormalizedEvent;

        assert!(!codex_event_requires_active_turn(&NormalizedEvent::Init {
            session_id: "thread-1".to_string(),
        }));
        assert!(!codex_event_requires_active_turn(&NormalizedEvent::Error {
            message: "turn/start failed".to_string(),
        }));
        assert!(!codex_event_requires_active_turn(&NormalizedEvent::Usage {
            usage: super::super::provider::TokenUsage {
                input_tokens: 42,
                output_tokens: 7,
                cached_input_tokens: Some(10),
            },
            context_window: Some(272_000),
        }));
        assert!(codex_event_requires_active_turn(
            &NormalizedEvent::AssistantText {
                text: "historical replay".to_string(),
                complete: true,
            }
        ));
        assert!(codex_event_requires_active_turn(&NormalizedEvent::Result {
            session_id: None,
            usage: None,
            stop_reason: Some("end_turn".to_string()),
            is_error: false,
            error_message: None,
        }));
    }

    #[test]
    fn detects_closed_bundled_mcp_transport_for_codex_recovery() {
        let event = super::super::provider::NormalizedEvent::McpToolCall {
            server: "flamel".to_string(),
            tool: "file_read".to_string(),
            arguments: serde_json::json!({"action": "read"}),
            result: None,
            error: Some(
                "tool call failed for `flamel/file_read`\n\nCaused by:\n    Transport closed"
                    .to_string(),
            ),
            status: "failed".to_string(),
            id: Some("call-1".to_string()),
        };

        assert_eq!(
            bundled_mcp_transport_closed(&event),
            Some(("flamel", "file_read"))
        );
    }

    #[test]
    fn ignores_external_or_non_transport_mcp_errors() {
        for (server, error) in [
            ("external", "Transport closed"),
            ("flamel", "request timed out"),
        ] {
            let event = super::super::provider::NormalizedEvent::McpToolCall {
                server: server.to_string(),
                tool: "read".to_string(),
                arguments: serde_json::Value::Null,
                result: None,
                error: Some(error.to_string()),
                status: "failed".to_string(),
                id: None,
            };
            assert_eq!(bundled_mcp_transport_closed(&event), None);
        }
    }

    #[test]
    fn codex_recovery_context_reuses_durable_state_without_another_model_call() {
        let parts = crate::db::sessions::RecoveryContextParts {
            compaction_summary: Some("Earlier durable context".to_string()),
            plan_context: Some("Accepted plan item".to_string()),
            recent_turns: "USER: Raw turn\n\nASSISTANT: Raw recap".to_string(),
            recent_turn_count: 4,
        };

        let context = format_provider_recovery_context(ProviderKind::Codex, &parts)
            .expect("codex recovery context");

        assert!(context.contains("Earlier durable context"));
        assert!(context.contains("Accepted plan item"));
        assert!(
            context
                .contains("NEWEST PERSISTED TURNS (authoritative for completed and current work)")
        );
        assert!(context.contains("USER: Raw turn"));
        assert!(!context.contains("NEWEST RECOVERY DIGEST"));
    }

    #[test]
    fn recovery_context_still_works_without_a_checkpoint() {
        let parts = crate::db::sessions::RecoveryContextParts {
            compaction_summary: None,
            plan_context: None,
            recent_turns: "USER: Raw turn\n\nASSISTANT: Raw recap".to_string(),
            recent_turn_count: 2,
        };

        let context = format_provider_recovery_context(ProviderKind::Codex, &parts)
            .expect("fallback recovery context");

        assert!(
            context
                .contains("NEWEST PERSISTED TURNS (authoritative for completed and current work)")
        );
        assert!(context.contains("USER: Raw turn"));
        assert!(!context.contains("NEWEST RECOVERY DIGEST"));
    }

    #[test]
    fn native_runtime_process_env_drops_inherited_secrets() {
        let provider_env = vec![
            ("PATH".into(), "/usr/bin".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            ("CODEX_HOME".into(), "/isolated/.codex".into()),
            ("DATABASE_URL".into(), "postgres://secret".into()),
            ("OPENAI_API_KEY".into(), "raw-server-secret".into()),
            ("FLAMEL_SIGNING_KEY".into(), "raw-signing-secret".into()),
        ];
        let process_env = BTreeMap::from([
            ("HOME".into(), "/isolated".into()),
            ("OPENAI_API_KEY".into(), "provider-capability-ref".into()),
            (
                "FLAMEL_TOOL_CAPABILITY_REF".into(),
                "tool-capability-ref".into(),
            ),
        ]);

        let isolated = isolated_provider_process_env(provider_env, &process_env)
            .into_iter()
            .collect::<BTreeMap<_, _>>();

        assert_eq!(isolated.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(
            isolated.get("OPENAI_API_KEY").map(String::as_str),
            Some("provider-capability-ref")
        );
        assert_eq!(isolated.get("HOME").map(String::as_str), Some("/isolated"));
        assert!(!isolated.contains_key("DATABASE_URL"));
        assert!(!isolated.contains_key("FLAMEL_SIGNING_KEY"));
        assert!(!isolated.values().any(|value| value == "raw-server-secret"));
    }

    #[test]
    fn codex_job_launch_reads_only_the_broker_base_url_from_process_env() {
        let process_env = BTreeMap::from([
            ("OPENAI_BASE_URL".into(), "http://127.0.0.1:43123/v1".into()),
            ("OPENAI_API_KEY".into(), "fjp_secret-capability".into()),
        ]);

        assert_eq!(
            provider_base_url_for_launch(ProviderKind::Codex, Some(&process_env)),
            Some("http://127.0.0.1:43123/v1")
        );
        assert_eq!(
            provider_base_url_for_launch(ProviderKind::Claude, Some(&process_env)),
            None
        );
        assert_eq!(
            provider_base_url_for_launch(ProviderKind::Codex, None),
            None
        );
    }
}

fn provider_base_url_for_launch<'a>(
    provider_kind: ProviderKind,
    process_env: Option<&'a BTreeMap<String, String>>,
) -> Option<&'a str> {
    match provider_kind {
        ProviderKind::Codex => process_env
            .and_then(|env| env.get("OPENAI_BASE_URL"))
            .map(String::as_str),
        ProviderKind::Claude => None,
    }
}

/// Retain only the operating-system values required to launch a provider, then
/// add the explicit opaque references granted to this native Job process.
pub fn isolated_provider_process_env(
    provider_env: Vec<(String, String)>,
    process_env: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut isolated = provider_env
        .into_iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "PATH"
                    | "USER"
                    | "LOGNAME"
                    | "SHELL"
                    | "LANG"
                    | "TZ"
                    | "TERM"
                    | "TMPDIR"
                    | "SSL_CERT_FILE"
                    | "SSL_CERT_DIR"
                    | "NODE_PATH"
                    | "PUPPETEER_SKIP_DOWNLOAD"
                    | "PUPPETEER_SKIP_CHROMIUM_DOWNLOAD"
                    | "PUPPETEER_EXECUTABLE_PATH"
                    | "CHROME_BIN"
                    | "PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD"
                    | "CODEX_HOME"
                    | "CLAUDE_CONFIG_DIR"
                    | "CLAUDE_CODE_ENTRYPOINT"
                    | "RUSTUP_HOME"
                    | "CARGO_HOME"
            ) || key.starts_with("LC_")
        })
        .collect::<Vec<_>>();
    for (key, value) in process_env {
        isolated.retain(|(existing, _)| existing != key);
        isolated.push((key.clone(), value.clone()));
    }
    isolated
}
