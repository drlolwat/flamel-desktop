use super::*;
use crate::console::{CommandEventKind, CommandOrigin, CommandSourceKind};

async fn test_terminal(command: &str) -> (TerminalSession, Arc<CommandEventStore>) {
    let events = Arc::new(CommandEventStore::new());
    let session = events
        .start_session(
            CommandOrigin::new(CommandSourceKind::Pty),
            None,
            BTreeMap::new(),
        )
        .await;
    let started = events
        .start_command(
            &session.id,
            Some(command.into()),
            None,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    let terminal = spawn_terminal(
        "test".into(),
        command,
        &std::env::temp_dir().to_string_lossy(),
        80,
        24,
        None,
        Arc::clone(&events),
        session.id,
        started.run.id,
        started.block.id,
    )
    .unwrap();
    (terminal, events)
}

async fn wait_for_exit(terminal: &TerminalSession, events: &CommandEventStore) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if terminal.exited.load(Ordering::Acquire)
                && events
                    .session(&terminal.command_session_id)
                    .await
                    .unwrap()
                    .closed_at
                    .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("PTY did not exit");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_prompts_without_newline_and_secret_input_never_enters_replay() {
    let (mut terminal, events) =
        test_terminal("stty -echo; printf READY; read secret; printf '\naccepted\n'; exit 7").await;
    let ready = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let (frames, _) = terminal.output_history.lock().unwrap().replay(0);
            let bytes: Vec<_> = frames
                .iter()
                .flat_map(|frame| frame.data.iter().copied())
                .collect();
            if String::from_utf8_lossy(&bytes).contains("READY") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    if ready.is_err() {
        kill_terminal(&mut terminal);
    }
    ready.expect("prompt was not streamed before newline");
    resize_terminal(&terminal, 100, 40);
    write_terminal(&terminal, b"do-not-record-this-secret\n").unwrap();
    wait_for_exit(&terminal, &events).await;
    assert_eq!(*terminal.exit_code.lock().unwrap(), Some(7));
    let replay = events.replay(&terminal.command_session_id).await;
    assert!(
        !replay
            .iter()
            .any(|event| event.kind == CommandEventKind::InputSent)
    );
    assert!(
        !serde_json::to_string(&replay)
            .unwrap()
            .contains("do-not-record-this-secret")
    );
    assert!(replay.iter().any(|event| {
        event.block.as_ref().is_some_and(|block| {
            block.exit_code == Some(7) && block.status == CommandLifecycleStatus::Failed
        })
    }));
    assert_eq!(replay.last().unwrap().kind, CommandEventKind::SessionClosed);
    assert!(terminal.child.lock().unwrap().try_wait().unwrap().is_some());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_pty_reaps_the_command_group() {
    let (mut terminal, events) = test_terminal("sleep 30 & wait").await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    kill_terminal(&mut terminal);
    wait_for_exit(&terminal, &events).await;
    assert!(terminal.child.lock().unwrap().try_wait().unwrap().is_some());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_closes_pty_when_a_descendant_ignores_term() {
    let (mut terminal, events) =
        test_terminal("trap 'exit 0' TERM; sh -c 'trap \"\" TERM; sleep 30' & wait").await;
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    kill_terminal(&mut terminal);
    wait_for_exit(&terminal, &events).await;
}
