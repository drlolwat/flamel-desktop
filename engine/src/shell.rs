//! Interactive terminal sessions backed by native PTYs and Windows ConPTY.

use crate::console::{CommandEventStore, CommandLifecycleStatus, CommandStreamKind};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

pub mod output;
use output::{OutputFrame, OutputHistory, TranscriptDecoder};

#[cfg(test)]
mod tests;

pub type TerminalSessions = Arc<Mutex<HashMap<String, TerminalSession>>>;

pub struct TerminalSession {
    /// Flamel account that owns this terminal session.
    pub owner_user_id: String,
    /// PTY master — owns resize and platform-specific terminal state.
    pub master: Box<dyn MasterPty + Send>,
    /// Serialized input stream for the PTY master.
    pub writer: Mutex<Box<dyn Write + Send>>,
    /// The child process (e.g. `sh -c <command>`).
    pub child: Arc<Mutex<Box<dyn portable_pty::Child + Send>>>,
    /// Broadcast channel for live output push to WebSocket subscribers.
    pub output_tx: broadcast::Sender<OutputFrame>,
    /// Latest byte frames, bounded independently of whether anyone reads them.
    pub output_history: Arc<Mutex<OutputHistory>>,
    /// Set true by the reader thread when the child process exits (EIO on master).
    pub exited: Arc<AtomicBool>,
    pub exit_code: Arc<Mutex<Option<i32>>>,
    pub started_at: std::time::Instant,
    pub cwd: String,
    pub command: String,
    /// Canonical command-event session id for unified console replay.
    pub command_session_id: String,
    /// Canonical command run id for output/input chunks.
    pub run_id: String,
    /// Canonical command block id for UI grouping.
    pub block_id: String,
    /// Prevent duplicate lifecycle events from EOF and explicit deletion.
    pub event_closed: Arc<AtomicBool>,
}

pub fn new_sessions() -> TerminalSessions {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Spawn a new interactive terminal session backed by a real PTY.
pub fn spawn_terminal(
    owner_user_id: String,
    command: &str,
    cwd: &str,
    cols: u16,
    rows: u16,
    sandbox_root: Option<&str>,
    command_events: Arc<CommandEventStore>,
    command_session_id: String,
    run_id: String,
    block_id: String,
) -> Result<TerminalSession, String> {
    let pair = portable_pty::native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("open PTY failed: {error}"))?;

    #[cfg(windows)]
    let mut cmd = {
        if sandbox_root.is_some() {
            return Err("Hosted account terminal isolation is unavailable on this platform".into());
        }
        let mut command_builder = CommandBuilder::new("powershell.exe");
        command_builder.args(["-NoLogo", "-NoProfile", "-Command", command]);
        command_builder
    };
    #[cfg(not(windows))]
    let cmd = {
        if sandbox_root.is_some() {
            return Err("A hosted execution backend is required".into());
        }
        let mut command_builder = CommandBuilder::new("sh");
        command_builder.args(["-c", command]);
        command_builder.cwd(cwd);
        command_builder
    };
    #[cfg(windows)]
    cmd.cwd(cwd);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("clone PTY reader failed: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("open PTY writer failed: {error}"))?;
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|error| format!("spawn failed: {error}"))?;
    let child: Arc<Mutex<Box<dyn portable_pty::Child + Send>>> = Arc::new(Mutex::new(child));
    drop(pair.slave);

    let (output_tx, _) = broadcast::channel::<OutputFrame>(2048);
    let output_history = Arc::new(Mutex::new(OutputHistory::default()));
    let exited = Arc::new(AtomicBool::new(false));
    let exit_code = Arc::new(Mutex::new(None));
    let event_closed = Arc::new(AtomicBool::new(false));
    let runtime_handle = tokio::runtime::Handle::try_current().ok();

    // Drain PTY bytes and publish ordered transcript/lifecycle events.
    {
        let tx = output_tx.clone();
        let history = Arc::clone(&output_history);
        let child = Arc::clone(&child);
        let exit_code = Arc::clone(&exit_code);
        let done = Arc::clone(&exited);
        let closed = Arc::clone(&event_closed);
        let command_events = Arc::clone(&command_events);
        let command_session_id = command_session_id.clone();
        let run_id = run_id.clone();
        let handle = runtime_handle.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut chunk = [0u8; 4096];
            let mut decoder = TranscriptDecoder::default();
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let data = chunk[..n].to_vec();
                        let frame = history.lock().unwrap().push(data.clone());
                        let _ = tx.send(frame);
                        if let Some(handle) = handle.as_ref() {
                            let text = decoder.push(&data);
                            if text.is_empty() {
                                continue;
                            }
                            let mut metadata = BTreeMap::new();
                            metadata.insert("pty_byte_len".to_string(), n.to_string());
                            // One reader serializes output before the terminal exit event.
                            handle.block_on(async {
                                let _ = command_events
                                    .append_output(&run_id, CommandStreamKind::Pty, text, metadata)
                                    .await;
                            });
                        }
                    }
                }
            }
            let trailing_text = decoder.finish();
            if !trailing_text.is_empty() {
                if let Some(handle) = handle.as_ref() {
                    handle.block_on(async {
                        let _ = command_events
                            .append_output(
                                &run_id,
                                CommandStreamKind::Pty,
                                trailing_text,
                                BTreeMap::new(),
                            )
                            .await;
                    });
                }
            }
            let code = loop {
                match child.lock().unwrap().try_wait() {
                    Ok(Some(status)) => break i32::try_from(status.exit_code()).ok(),
                    Err(_) => break None,
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(25)),
                }
            };
            *exit_code.lock().unwrap() = code;
            done.store(true, Ordering::Release);
            if !closed.swap(true, Ordering::Relaxed) {
                if let Some(handle) = handle.as_ref() {
                    handle.block_on(async {
                        let _ = command_events
                            .finish_command(
                                &run_id,
                                match code {
                                    Some(0) => CommandLifecycleStatus::Succeeded,
                                    Some(_) => CommandLifecycleStatus::Failed,
                                    None => CommandLifecycleStatus::Closed,
                                },
                                code,
                            )
                            .await;
                        let _ = command_events
                            .close_session(&command_session_id, CommandLifecycleStatus::Closed)
                            .await;
                    });
                }
            }
            let frame = history.lock().unwrap().push(Vec::new());
            let _ = tx.send(frame);
        });
    }

    let session = TerminalSession {
        owner_user_id,
        master: pair.master,
        writer: Mutex::new(writer),
        child,
        output_tx,
        output_history,
        exited,
        exit_code,
        started_at: std::time::Instant::now(),
        cwd: cwd.to_string(),
        command: command.to_string(),
        command_session_id,
        run_id,
        block_id,
        event_closed,
    };

    Ok(session)
}

/// Write bytes to the PTY master (stdin for the child process).
pub fn write_terminal(session: &TerminalSession, data: &[u8]) -> Result<(), String> {
    let mut writer = session
        .writer
        .lock()
        .map_err(|_| "terminal writer lock poisoned".to_string())?;
    writer
        .write_all(data)
        .and_then(|_| writer.flush())
        .map_err(|error| format!("write failed: {error}"))
}

/// Resize the PTY window (TIOCSWINSZ).
pub fn resize_terminal(session: &TerminalSession, cols: u16, rows: u16) {
    let _ = session.master.resize(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    });
}

/// Kill the session's child and reap the zombie.
pub fn kill_terminal(session: &mut TerminalSession) {
    #[cfg(unix)]
    let pid = session
        .child
        .lock()
        .unwrap()
        .process_id()
        .and_then(|pid| i32::try_from(pid).ok());
    #[cfg(unix)]
    if let Some(pid) = pid {
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
        for _ in 0..10 {
            if session
                .child
                .lock()
                .unwrap()
                .try_wait()
                .ok()
                .flatten()
                .is_some()
            {
                // The shell can exit before a child that ignores SIGTERM.
                // Still send the final group kill so that child cannot hold
                // the PTY open forever and leak the terminal quota.
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = session.child.lock().unwrap().kill();
    // The reader/supervisor reaps the process and reports its final status.
}
