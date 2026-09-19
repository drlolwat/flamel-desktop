use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, VecDeque};
use tokio::sync::{Mutex, broadcast};

const DEFAULT_MAX_COMMAND_EVENTS_PER_SESSION: usize = 2_000;
const DEFAULT_MAX_COMMAND_TEXT_CHARS: usize = 16_000;
const DEFAULT_MAX_COMMAND_SESSIONS: usize = 200;
const MAX_COMMAND_REPLAY_BYTES_PER_SESSION: usize = 1_048_576;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandSourceKind {
    McpShell,
    Runner,
    Deploy,
    Provider,
    Agent,
    Pty,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandStreamKind {
    Stdout,
    Stderr,
    System,
    Stdin,
    Pty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandLifecycleStatus {
    Pending,
    Running,
    WaitingForInput,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandEventKind {
    SessionStarted,
    CommandStarted,
    OutputChunk,
    InputRequested,
    InputSent,
    CommandFinished,
    SessionClosed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandOrigin {
    pub kind: CommandSourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandSession {
    pub id: String,
    pub origin: CommandOrigin,
    pub status: CommandLifecycleStatus,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandRun {
    pub id: String,
    pub session_id: String,
    pub origin: CommandOrigin,
    pub status: CommandLifecycleStatus,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandBlock {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub origin: CommandOrigin,
    pub status: CommandLifecycleStatus,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandOutputChunk {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub block_id: String,
    pub sequence: u64,
    pub stream: CommandStreamKind,
    pub text: String,
    pub received_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_len: Option<usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandInputRequest {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub block_id: String,
    pub requested_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default)]
    pub secret: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandEvent {
    pub event_id: String,
    pub kind: CommandEventKind,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    pub sequence: u64,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<CommandSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<CommandRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<CommandBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk: Option<CommandOutputChunk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_request: Option<CommandInputRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CommandEvent {
    pub fn to_ndjson_line(&self) -> String {
        format!("{}\n", json!({ "type": "command_event", "event": self }))
    }
}

impl CommandOrigin {
    pub fn new(kind: CommandSourceKind) -> Self {
        Self {
            kind,
            runner_id: None,
            task_id: None,
            tool_call_id: None,
            agent_run_id: None,
            label: None,
        }
    }

    pub fn mcp_shell(tool_call_id: Option<String>) -> Self {
        Self {
            tool_call_id,
            ..Self::new(CommandSourceKind::McpShell)
        }
    }

    pub fn runner(runner_id: impl Into<String>, task_id: Option<String>) -> Self {
        Self {
            runner_id: Some(runner_id.into()),
            task_id,
            ..Self::new(CommandSourceKind::Runner)
        }
    }

    pub fn deploy(
        runner_id: impl Into<String>,
        task_id: Option<String>,
        label: Option<String>,
    ) -> Self {
        Self {
            runner_id: Some(runner_id.into()),
            task_id,
            label,
            ..Self::new(CommandSourceKind::Deploy)
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_agent_run_id(mut self, agent_run_id: impl Into<String>) -> Self {
        self.agent_run_id = Some(agent_run_id.into());
        self
    }
}

impl CommandStreamKind {
    pub fn from_runner_stream(stream: &str) -> Self {
        match stream.trim().to_ascii_lowercase().as_str() {
            "stderr" | "error" => Self::Stderr,
            "stdin" => Self::Stdin,
            "pty" => Self::Pty,
            "system" => Self::System,
            _ => Self::Stdout,
        }
    }
}

impl CommandLifecycleStatus {
    pub fn from_exit_code(exit_code: Option<i32>) -> Self {
        match exit_code {
            Some(0) | None => Self::Succeeded,
            Some(_) => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartedCommand {
    pub run: CommandRun,
    pub block: CommandBlock,
}

#[derive(Debug, Clone)]
struct CommandRunContext {
    session_id: String,
    run_id: String,
}

struct CommandEventState {
    next_sequence: u64,
    session_order: VecDeque<String>,
    sessions: HashMap<String, CommandSession>,
    runs: HashMap<String, CommandRun>,
    blocks: HashMap<String, CommandBlock>,
    block_id_by_run_id: HashMap<String, String>,
    runner_console_contexts: HashMap<String, CommandRunContext>,
    events: HashMap<String, VecDeque<CommandEvent>>,
    event_bytes: HashMap<String, usize>,
    discarded_through: HashMap<String, u64>,
}

pub struct CommandEventStore {
    epoch: String,
    state: Mutex<CommandEventState>,
    tx: broadcast::Sender<CommandEvent>,
    max_events_per_session: usize,
    max_text_chars: usize,
    max_sessions: usize,
}

impl CommandEventStore {
    pub fn new() -> Self {
        Self::with_limits(
            DEFAULT_MAX_COMMAND_EVENTS_PER_SESSION,
            DEFAULT_MAX_COMMAND_TEXT_CHARS,
            DEFAULT_MAX_COMMAND_SESSIONS,
        )
    }

    pub fn with_limits(
        max_events_per_session: usize,
        max_text_chars: usize,
        max_sessions: usize,
    ) -> Self {
        let max_events_per_session = max_events_per_session.max(1);
        let max_text_chars = max_text_chars.max(1);
        let max_sessions = max_sessions.max(1);
        let broadcast_capacity = max_events_per_session
            .saturating_mul(max_sessions)
            .clamp(128, 8_192);
        let (tx, _) = broadcast::channel(broadcast_capacity);

        Self {
            epoch: uuid::Uuid::new_v4().to_string(),
            state: Mutex::new(CommandEventState {
                next_sequence: 1,
                session_order: VecDeque::new(),
                sessions: HashMap::new(),
                runs: HashMap::new(),
                blocks: HashMap::new(),
                block_id_by_run_id: HashMap::new(),
                runner_console_contexts: HashMap::new(),
                events: HashMap::new(),
                event_bytes: HashMap::new(),
                discarded_through: HashMap::new(),
            }),
            tx,
            max_events_per_session,
            max_text_chars,
            max_sessions,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CommandEvent> {
        self.tx.subscribe()
    }

    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    pub async fn replay_was_truncated(
        &self,
        session_id: Option<&str>,
        runner_id: Option<&str>,
        after: u64,
    ) -> bool {
        let state = self.state.lock().await;
        state.discarded_through.iter().any(|(id, discarded)| {
            *discarded > after
                && (session_id == Some(id.as_str())
                    || runner_id.is_some_and(|runner| {
                        state.sessions.get(id).is_some_and(|session| {
                            session.origin.runner_id.as_deref() == Some(runner)
                        })
                    }))
        })
    }

    pub async fn replay(&self, session_id: &str) -> Vec<CommandEvent> {
        let state = self.state.lock().await;
        state
            .events
            .get(session_id)
            .map(|events| events.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn replay_since(
        &self,
        session_id: &str,
        after_sequence: Option<u64>,
    ) -> Vec<CommandEvent> {
        let state = self.state.lock().await;
        state
            .events
            .get(session_id)
            .map(|events| {
                events
                    .iter()
                    .filter(|event| {
                        after_sequence
                            .map(|cursor| event.sequence > cursor)
                            .unwrap_or(true)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub async fn session(&self, session_id: &str) -> Option<CommandSession> {
        let state = self.state.lock().await;
        state.sessions.get(session_id).cloned()
    }

    pub async fn start_session(
        &self,
        origin: CommandOrigin,
        cwd: Option<String>,
        metadata: BTreeMap<String, String>,
    ) -> CommandSession {
        let (session, event) = {
            let mut state = self.state.lock().await;
            let (sequence, timestamp) = reserve_event_metadata(&mut state);
            let session = CommandSession {
                id: uuid::Uuid::new_v4().to_string(),
                origin,
                status: CommandLifecycleStatus::Running,
                created_at: timestamp.clone(),
                closed_at: None,
                cwd,
                metadata,
            };
            state.session_order.push_back(session.id.clone());
            state.sessions.insert(session.id.clone(), session.clone());
            self.trim_sessions_locked(&mut state);

            let event = self.persist_event_locked(
                &mut state,
                CommandEvent {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    kind: CommandEventKind::SessionStarted,
                    session_id: session.id.clone(),
                    run_id: None,
                    block_id: None,
                    sequence,
                    timestamp,
                    session: Some(session.clone()),
                    run: None,
                    block: None,
                    chunk: None,
                    input_request: None,
                    error: None,
                },
            );
            (session, event)
        };
        let _ = self.tx.send(event);
        session
    }

    pub async fn start_command(
        &self,
        session_id: &str,
        command: Option<String>,
        cwd: Option<String>,
        title: Option<String>,
        metadata: BTreeMap<String, String>,
    ) -> Result<StartedCommand, String> {
        let (started, event) = {
            let mut state = self.state.lock().await;
            let (sequence, timestamp) = reserve_event_metadata(&mut state);
            let origin = {
                let session = state
                    .sessions
                    .get_mut(session_id)
                    .ok_or_else(|| format!("command session not found: {session_id}"))?;
                session.status = CommandLifecycleStatus::Running;
                session.origin.clone()
            };

            let run = CommandRun {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: session_id.to_string(),
                origin: origin.clone(),
                status: CommandLifecycleStatus::Running,
                started_at: timestamp.clone(),
                finished_at: None,
                command: command.clone(),
                cwd: cwd.clone(),
                exit_code: None,
                metadata: metadata.clone(),
            };
            let block = CommandBlock {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: session_id.to_string(),
                run_id: run.id.clone(),
                origin,
                status: CommandLifecycleStatus::Running,
                started_at: timestamp.clone(),
                finished_at: None,
                title,
                command,
                exit_code: None,
                metadata,
            };
            state
                .block_id_by_run_id
                .insert(run.id.clone(), block.id.clone());
            state.runs.insert(run.id.clone(), run.clone());
            state.blocks.insert(block.id.clone(), block.clone());

            let started = StartedCommand { run, block };
            let event = self.persist_event_locked(
                &mut state,
                CommandEvent {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    kind: CommandEventKind::CommandStarted,
                    session_id: session_id.to_string(),
                    run_id: Some(started.run.id.clone()),
                    block_id: Some(started.block.id.clone()),
                    sequence,
                    timestamp,
                    session: None,
                    run: Some(started.run.clone()),
                    block: Some(started.block.clone()),
                    chunk: None,
                    input_request: None,
                    error: None,
                },
            );
            (started, event)
        };
        let _ = self.tx.send(event);
        Ok(started)
    }

    pub async fn append_output(
        &self,
        run_id: &str,
        stream: CommandStreamKind,
        text: impl Into<String>,
        metadata: BTreeMap<String, String>,
    ) -> Result<CommandOutputChunk, String> {
        let (chunk, event) = {
            let mut state = self.state.lock().await;
            let (session_id, block_id) = self.run_and_block_ids_locked(&state, run_id)?;
            let (sequence, timestamp) = reserve_event_metadata(&mut state);
            let raw_text = text.into();
            let chunk = CommandOutputChunk {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: session_id.clone(),
                run_id: run_id.to_string(),
                block_id: block_id.clone(),
                sequence,
                stream,
                byte_len: Some(raw_text.len()),
                text: truncate_chars(raw_text, self.max_text_chars),
                received_at: timestamp.clone(),
                metadata,
            };
            let event = self.persist_event_locked(
                &mut state,
                CommandEvent {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    kind: CommandEventKind::OutputChunk,
                    session_id,
                    run_id: Some(run_id.to_string()),
                    block_id: Some(block_id),
                    sequence,
                    timestamp,
                    session: None,
                    run: None,
                    block: None,
                    chunk: Some(chunk.clone()),
                    input_request: None,
                    error: None,
                },
            );
            (chunk, event)
        };
        let _ = self.tx.send(event);
        Ok(chunk)
    }

    pub async fn request_input(
        &self,
        run_id: &str,
        prompt: Option<String>,
        secret: bool,
        metadata: BTreeMap<String, String>,
    ) -> Result<CommandInputRequest, String> {
        let (input_request, event) = {
            let mut state = self.state.lock().await;
            let (session_id, block_id) = self.run_and_block_ids_locked(&state, run_id)?;
            let (sequence, timestamp) = reserve_event_metadata(&mut state);
            if let Some(run) = state.runs.get_mut(run_id) {
                run.status = CommandLifecycleStatus::WaitingForInput;
            }
            if let Some(block) = state.blocks.get_mut(&block_id) {
                block.status = CommandLifecycleStatus::WaitingForInput;
            }
            let input_request = CommandInputRequest {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: session_id.clone(),
                run_id: run_id.to_string(),
                block_id: block_id.clone(),
                requested_at: timestamp.clone(),
                prompt,
                secret,
                metadata,
            };
            let event = self.persist_event_locked(
                &mut state,
                CommandEvent {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    kind: CommandEventKind::InputRequested,
                    session_id,
                    run_id: Some(run_id.to_string()),
                    block_id: Some(block_id),
                    sequence,
                    timestamp,
                    session: None,
                    run: None,
                    block: None,
                    chunk: None,
                    input_request: Some(input_request.clone()),
                    error: None,
                },
            );
            (input_request, event)
        };
        let _ = self.tx.send(event);
        Ok(input_request)
    }

    pub async fn mark_input_sent(
        &self,
        run_id: &str,
        text: impl Into<String>,
        metadata: BTreeMap<String, String>,
    ) -> Result<CommandOutputChunk, String> {
        let (chunk, event) = {
            let mut state = self.state.lock().await;
            let (session_id, block_id) = self.run_and_block_ids_locked(&state, run_id)?;
            let (sequence, timestamp) = reserve_event_metadata(&mut state);
            if let Some(run) = state.runs.get_mut(run_id) {
                run.status = CommandLifecycleStatus::Running;
            }
            if let Some(block) = state.blocks.get_mut(&block_id) {
                block.status = CommandLifecycleStatus::Running;
            }
            let raw_text = text.into();
            let chunk = CommandOutputChunk {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: session_id.clone(),
                run_id: run_id.to_string(),
                block_id: block_id.clone(),
                sequence,
                stream: CommandStreamKind::Stdin,
                text: truncate_chars(raw_text, self.max_text_chars),
                received_at: timestamp.clone(),
                byte_len: None,
                metadata,
            };
            let event = self.persist_event_locked(
                &mut state,
                CommandEvent {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    kind: CommandEventKind::InputSent,
                    session_id,
                    run_id: Some(run_id.to_string()),
                    block_id: Some(block_id),
                    sequence,
                    timestamp,
                    session: None,
                    run: None,
                    block: None,
                    chunk: Some(chunk.clone()),
                    input_request: None,
                    error: None,
                },
            );
            (chunk, event)
        };
        let _ = self.tx.send(event);
        Ok(chunk)
    }

    pub async fn finish_command(
        &self,
        run_id: &str,
        status: CommandLifecycleStatus,
        exit_code: Option<i32>,
    ) -> Result<CommandBlock, String> {
        let (block, event) = {
            let mut state = self.state.lock().await;
            let (sequence, timestamp) = reserve_event_metadata(&mut state);
            let (session_id, run_clone) = {
                let run = state
                    .runs
                    .get_mut(run_id)
                    .ok_or_else(|| format!("command run not found: {run_id}"))?;
                run.status = status.clone();
                run.finished_at = Some(timestamp.clone());
                run.exit_code = exit_code;
                (run.session_id.clone(), run.clone())
            };
            let block_id = state
                .block_id_by_run_id
                .get(run_id)
                .cloned()
                .ok_or_else(|| format!("command block not found for run: {run_id}"))?;
            let block = {
                let block = state
                    .blocks
                    .get_mut(&block_id)
                    .ok_or_else(|| format!("command block not found: {block_id}"))?;
                block.status = status;
                block.finished_at = Some(timestamp.clone());
                block.exit_code = exit_code;
                block.clone()
            };
            let event = self.persist_event_locked(
                &mut state,
                CommandEvent {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    kind: CommandEventKind::CommandFinished,
                    session_id,
                    run_id: Some(run_id.to_string()),
                    block_id: Some(block_id),
                    sequence,
                    timestamp,
                    session: None,
                    run: Some(run_clone),
                    block: Some(block.clone()),
                    chunk: None,
                    input_request: None,
                    error: None,
                },
            );
            (block, event)
        };
        let _ = self.tx.send(event);
        Ok(block)
    }

    pub async fn close_session(
        &self,
        session_id: &str,
        status: CommandLifecycleStatus,
    ) -> Result<CommandSession, String> {
        let (session, event) = {
            let mut state = self.state.lock().await;
            let (sequence, timestamp) = reserve_event_metadata(&mut state);
            let session = {
                let session = state
                    .sessions
                    .get_mut(session_id)
                    .ok_or_else(|| format!("command session not found: {session_id}"))?;
                session.status = status;
                session.closed_at = Some(timestamp.clone());
                session.clone()
            };
            let event = self.persist_event_locked(
                &mut state,
                CommandEvent {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    kind: CommandEventKind::SessionClosed,
                    session_id: session_id.to_string(),
                    run_id: None,
                    block_id: None,
                    sequence,
                    timestamp,
                    session: Some(session.clone()),
                    run: None,
                    block: None,
                    chunk: None,
                    input_request: None,
                    error: None,
                },
            );
            (session, event)
        };
        let _ = self.tx.send(event);
        Ok(session)
    }

    pub async fn emit_error(
        &self,
        session_id: &str,
        run_id: Option<&str>,
        error: impl Into<String>,
    ) -> CommandEvent {
        let event = {
            let mut state = self.state.lock().await;
            let (sequence, timestamp) = reserve_event_metadata(&mut state);
            let block_id = run_id.and_then(|id| state.block_id_by_run_id.get(id).cloned());
            if let Some(session) = state.sessions.get_mut(session_id) {
                session.status = CommandLifecycleStatus::Failed;
            }
            if let Some(run_id) = run_id {
                if let Some(run) = state.runs.get_mut(run_id) {
                    run.status = CommandLifecycleStatus::Failed;
                    run.finished_at.get_or_insert_with(|| timestamp.clone());
                }
            }
            if let Some(block_id) = block_id.as_deref() {
                if let Some(block) = state.blocks.get_mut(block_id) {
                    block.status = CommandLifecycleStatus::Failed;
                    block.finished_at.get_or_insert_with(|| timestamp.clone());
                }
            }
            self.persist_event_locked(
                &mut state,
                CommandEvent {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    kind: CommandEventKind::Error,
                    session_id: session_id.to_string(),
                    run_id: run_id.map(str::to_string),
                    block_id,
                    sequence,
                    timestamp,
                    session: None,
                    run: None,
                    block: None,
                    chunk: None,
                    input_request: None,
                    error: Some(error.into()),
                },
            )
        };
        let _ = self.tx.send(event.clone());
        event
    }

    pub async fn record_one_shot_command(
        &self,
        origin: CommandOrigin,
        command: impl Into<String>,
        cwd: Option<String>,
        stdout: Option<String>,
        stderr: Option<String>,
        exit_code: Option<i32>,
    ) -> Result<CommandBlock, String> {
        let command = command.into();
        let session = self
            .start_session(origin, cwd.clone(), BTreeMap::new())
            .await;
        let started = self
            .start_command(
                &session.id,
                Some(command.clone()),
                cwd,
                Some(command),
                BTreeMap::new(),
            )
            .await?;
        if let Some(stdout) = stdout {
            if !stdout.is_empty() {
                self.append_output(
                    &started.run.id,
                    CommandStreamKind::Stdout,
                    stdout,
                    BTreeMap::new(),
                )
                .await?;
            }
        }
        if let Some(stderr) = stderr {
            if !stderr.is_empty() {
                self.append_output(
                    &started.run.id,
                    CommandStreamKind::Stderr,
                    stderr,
                    BTreeMap::new(),
                )
                .await?;
            }
        }
        let status = CommandLifecycleStatus::from_exit_code(exit_code);
        let block = self
            .finish_command(&started.run.id, status, exit_code)
            .await?;
        self.close_session(&session.id, CommandLifecycleStatus::Closed)
            .await?;
        Ok(block)
    }

    fn run_and_block_ids_locked(
        &self,
        state: &CommandEventState,
        run_id: &str,
    ) -> Result<(String, String), String> {
        let run = state
            .runs
            .get(run_id)
            .ok_or_else(|| format!("command run not found: {run_id}"))?;
        let block_id = state
            .block_id_by_run_id
            .get(run_id)
            .cloned()
            .ok_or_else(|| format!("command block not found for run: {run_id}"))?;
        Ok((run.session_id.clone(), block_id))
    }

    fn persist_event_locked(
        &self,
        state: &mut CommandEventState,
        event: CommandEvent,
    ) -> CommandEvent {
        let queue = state.events.entry(event.session_id.clone()).or_default();
        let bytes = state
            .event_bytes
            .entry(event.session_id.clone())
            .or_default();
        *bytes += serde_json::to_vec(&event).map_or(0, |encoded| encoded.len());
        queue.push_back(event.clone());
        while queue.len() > self.max_events_per_session
            || (*bytes > MAX_COMMAND_REPLAY_BYTES_PER_SESSION && queue.len() > 1)
        {
            if let Some(discarded) = queue.pop_front() {
                *bytes = bytes.saturating_sub(
                    serde_json::to_vec(&discarded).map_or(0, |encoded| encoded.len()),
                );
                state
                    .discarded_through
                    .insert(event.session_id.clone(), discarded.sequence);
            }
        }
        event
    }

    fn trim_sessions_locked(&self, state: &mut CommandEventState) {
        while state.session_order.len() > self.max_sessions {
            // Live processes must keep their lifecycle records. Producer quotas bound
            // active sessions; this limit applies to completed console history.
            let Some(index) = state.session_order.iter().position(|id| {
                state
                    .sessions
                    .get(id)
                    .is_some_and(|session| session.closed_at.is_some())
            }) else {
                break;
            };
            let session_id = state.session_order.remove(index).unwrap();
            state.events.remove(&session_id);
            state.event_bytes.remove(&session_id);
            state.discarded_through.remove(&session_id);
            state.sessions.remove(&session_id);

            let removed_run_ids: Vec<String> = state
                .runs
                .iter()
                .filter_map(|(run_id, run)| {
                    (run.session_id == session_id).then_some(run_id.clone())
                })
                .collect();
            for run_id in removed_run_ids {
                state.runs.remove(&run_id);
                state.block_id_by_run_id.remove(&run_id);
            }

            let removed_block_ids: Vec<String> = state
                .blocks
                .iter()
                .filter_map(|(block_id, block)| {
                    (block.session_id == session_id).then_some(block_id.clone())
                })
                .collect();
            for block_id in removed_block_ids {
                state.blocks.remove(&block_id);
            }
            state
                .runner_console_contexts
                .retain(|_, context| context.session_id != session_id);
        }
    }
}

fn reserve_event_metadata(state: &mut CommandEventState) -> (u64, String) {
    let sequence = state.next_sequence;
    state.next_sequence = state.next_sequence.saturating_add(1);
    (sequence, chrono::Utc::now().to_rfc3339())
}

fn truncate_chars(mut text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }
    text = text.chars().take(max_chars).collect();
    text.push_str("...[truncated]");
    text
}

#[cfg(unix)]
pub fn terminate_process_group(process_group_id: i32) -> std::io::Result<()> {
    send_process_group_signal(process_group_id, libc::SIGTERM)
}

#[cfg(unix)]
pub fn kill_process_group(process_group_id: i32) -> std::io::Result<()> {
    send_process_group_signal(process_group_id, libc::SIGKILL)
}

#[cfg(unix)]
fn send_process_group_signal(process_group_id: i32, signal: i32) -> std::io::Result<()> {
    if process_group_id <= 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "process_group_id must be positive",
        ));
    }
    let result = unsafe { libc::kill(-process_group_id, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
pub fn terminate_process_group(_process_group_id: i32) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process-group cleanup is only implemented on Unix platforms",
    ))
}

#[cfg(not(unix))]
pub fn kill_process_group(_process_group_id: i32) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process-group cleanup is only implemented on Unix platforms",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_one_shot_command_lifecycle() {
        let store = CommandEventStore::new();
        let block = store
            .record_one_shot_command(
                CommandOrigin::mcp_shell(Some("tool-1".to_string())).with_label("shell"),
                "printf hello",
                Some("/tmp".to_string()),
                Some("hello".to_string()),
                None,
                Some(0),
            )
            .await
            .unwrap();

        assert_eq!(block.status, CommandLifecycleStatus::Succeeded);
        let replay = store.replay(&block.session_id).await;
        let kinds: Vec<CommandEventKind> = replay.iter().map(|event| event.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                CommandEventKind::SessionStarted,
                CommandEventKind::CommandStarted,
                CommandEventKind::OutputChunk,
                CommandEventKind::CommandFinished,
                CommandEventKind::SessionClosed,
            ]
        );
        assert_eq!(
            replay[2].chunk.as_ref().map(|chunk| chunk.text.as_str()),
            Some("hello")
        );
    }

    #[tokio::test]
    async fn trims_replay_per_session() {
        let store = CommandEventStore::with_limits(3, 128, 4);
        let session = store
            .start_session(
                CommandOrigin::new(CommandSourceKind::System),
                None,
                BTreeMap::new(),
            )
            .await;
        let started = store
            .start_command(
                &session.id,
                Some("cmd".to_string()),
                None,
                None,
                BTreeMap::new(),
            )
            .await
            .unwrap();

        for idx in 0..5 {
            store
                .append_output(
                    &started.run.id,
                    CommandStreamKind::Stdout,
                    format!("line-{idx}"),
                    BTreeMap::new(),
                )
                .await
                .unwrap();
        }

        let replay = store.replay(&session.id).await;
        assert_eq!(replay.len(), 3);
        assert!(store.replay_was_truncated(Some(&session.id), None, 0).await);
        assert!(
            !store
                .replay_was_truncated(Some(&session.id), None, replay.last().unwrap().sequence)
                .await
        );
        assert_eq!(
            replay
                .last()
                .and_then(|event| event.chunk.as_ref())
                .map(|chunk| chunk.text.as_str()),
            Some("line-4")
        );
    }

    #[tokio::test]
    async fn replay_has_a_byte_budget_as_well_as_an_event_limit() {
        let store = CommandEventStore::new();
        let session = store
            .start_session(
                CommandOrigin::new(CommandSourceKind::Pty),
                None,
                BTreeMap::new(),
            )
            .await;
        let started = store
            .start_command(
                &session.id,
                Some("noisy".into()),
                None,
                None,
                BTreeMap::new(),
            )
            .await
            .unwrap();
        for _ in 0..100 {
            store
                .append_output(
                    &started.run.id,
                    CommandStreamKind::Pty,
                    "x".repeat(16_000),
                    BTreeMap::new(),
                )
                .await
                .unwrap();
        }
        let replay = store.replay(&session.id).await;
        let retained_bytes: usize = replay
            .iter()
            .map(|event| serde_json::to_vec(event).unwrap().len())
            .sum();
        assert!(retained_bytes <= MAX_COMMAND_REPLAY_BYTES_PER_SESSION);
        assert!(store.replay_was_truncated(Some(&session.id), None, 0).await);
        assert_eq!(replay.last().unwrap().sequence, 102);
    }

    #[tokio::test]
    async fn retention_never_evicts_a_running_command() {
        let store = CommandEventStore::with_limits(10, 128, 1);
        let live = store
            .start_session(
                CommandOrigin::new(CommandSourceKind::Pty),
                None,
                BTreeMap::new(),
            )
            .await;
        let first = store
            .start_command(&live.id, Some("live".into()), None, None, BTreeMap::new())
            .await
            .unwrap();
        for _ in 0..3 {
            store
                .record_one_shot_command(
                    CommandOrigin::new(CommandSourceKind::System),
                    "done",
                    None,
                    None,
                    None,
                    Some(0),
                )
                .await
                .unwrap();
        }
        assert!(store.session(&live.id).await.is_some());
        store
            .append_output(
                &first.run.id,
                CommandStreamKind::Stdout,
                "still running",
                BTreeMap::new(),
            )
            .await
            .unwrap();
    }
}
