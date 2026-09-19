//! Process-lifetime helpers for MCP sidecars and their long-lived children.

/// Install top-level MCP lifetime handling.
///
/// Stdio MCP servers are owned by their input pipe and exit when that pipe
/// closes. They must not use a direct-parent death signal: Codex may
/// legitimately reparent a running sidecar while rebuilding its app-server
/// transport, which would otherwise kill a healthy MCP process. Flamel still
/// owns the provider process group, while children spawned by an MCP retain
/// the stricter parent-death guard configured below.
pub fn install() {
    // Intentionally empty. Dropping/closing stdin is the portable MCP
    // lifecycle signal; the provider process group is the crash backstop.
}

/// Give a long-lived child the same Linux parent-death guarantee.
#[cfg(target_os = "linux")]
pub fn configure_child(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt as _;

    unsafe {
        command.pre_exec(|| {
            if prctl(PR_SET_PDEATHSIG, SIGKILL, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
pub fn configure_child(_command: &mut std::process::Command) {}

/// Construct a standard-library child process that cannot outlive this process
/// on Linux.
pub fn command<S: AsRef<std::ffi::OsStr>>(program: S) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    configure_child(&mut command);
    command
}

/// Configure a Tokio child as an owned process-group leader that dies with its
/// host process and is killed if its Rust handle is dropped.
pub fn configure_tokio_child(command: &mut tokio::process::Command) {
    command.kill_on_drop(true);
    configure_child(command.as_std_mut());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.as_std_mut().process_group(0);
    }
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn prctl(option: i32, arg2: i32, arg3: usize, arg4: usize, arg5: usize) -> i32;
}

#[cfg(target_os = "linux")]
const PR_SET_PDEATHSIG: i32 = 1;
#[cfg(target_os = "linux")]
const SIGKILL: i32 = 9;

/// Linux binds PDEATHSIG to the spawning *thread*, not merely its process.
/// A gateway worker must therefore outlive an individual tool-request thread.
/// Keep a dedicated spawning thread alive until its owned child is released.
pub struct ChildThreadOwner {
    release: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for ChildThreadOwner {
    fn drop(&mut self) {
        self.release.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub fn spawn_long_lived(
    mut command: std::process::Command,
) -> std::io::Result<(std::process::Child, ChildThreadOwner)> {
    let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("mcp-worker-owner".into())
        .spawn(move || match command.spawn() {
            Ok(child) => {
                if result_tx.send(Ok(child)).is_ok() {
                    let _ = release_rx.recv();
                }
            }
            Err(error) => {
                let _ = result_tx.send(Err(error));
            }
        })?;
    let owner = ChildThreadOwner {
        release: Some(release_tx),
        thread: Some(thread),
    };
    let child = result_rx
        .recv()
        .map_err(|_| std::io::Error::other("MCP owner thread ended before spawning"))??;
    Ok((child, owner))
}

#[cfg(all(test, target_os = "linux"))]
mod lifetime_tests {
    use super::*;
    #[test]
    fn a_gateway_worker_survives_the_request_thread_that_created_it() {
        let (mut child, owner) = std::thread::spawn(|| {
            let mut command = command("sleep");
            command.arg("30");
            spawn_long_lived(command).unwrap()
        })
        .join()
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(40));
        assert!(
            child.try_wait().unwrap().is_none(),
            "short-lived request thread killed the worker"
        );
        // Releasing its actual owner still enforces parent-death cleanup.
        drop(owner);
        assert!(!child.wait().unwrap().success());
    }
}
