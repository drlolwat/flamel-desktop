#![cfg(feature = "engine")]

#[cfg(target_os = "linux")]
fn main() {
    if let Some(mode) = std::env::var_os("FLAMEL_MCP_GUARD_TEST_HELPER") {
        run_parent_helper(mode.to_string_lossy().as_ref());
        return;
    }
    verify_parent_death("mcp");
    verify_parent_death("guarded-child");
}

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn run_parent_helper(mode: &str) {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut command = if mode == "mcp" {
        Command::new(env!("CARGO_BIN_EXE_flamel_mcp"))
    } else {
        let mut command = flamel::mcp_parent_guard::command("sleep");
        command.arg("60");
        command
    };
    let child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn guarded child");
    println!("{}", child.id());
    std::io::stdout().flush().expect("flush MCP pid");

    // Keep both the child handle and its stdin writer alive until the outer
    // test kills this helper process.
    let _child = child;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

#[cfg(target_os = "linux")]
fn verify_parent_death(mode: &str) {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let mut parent = Command::new(std::env::current_exe().expect("test executable"))
        .env("FLAMEL_MCP_GUARD_TEST_HELPER", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn synthetic MCP parent");
    let parent_pid = parent.id() as i32;
    let mut child_line = String::new();
    BufReader::new(parent.stdout.take().expect("parent stdout"))
        .read_line(&mut child_line)
        .expect("read MCP pid");
    let child_pid: i32 = child_line.trim().parse().expect("numeric MCP pid");

    let observed_parent = process_parent_pid(child_pid).expect("MCP process metadata");
    assert_eq!(
        observed_parent, parent_pid,
        "test helper was not the guarded process's direct parent"
    );

    unsafe {
        libc::kill(parent_pid, libc::SIGKILL);
    }
    let _ = parent.wait();

    let deadline = Instant::now() + Duration::from_secs(3);
    while process_is_alive(child_pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }

    let survived = process_is_alive(child_pid);
    if survived {
        unsafe {
            libc::kill(child_pid, libc::SIGKILL);
        }
    }
    assert!(
        !survived,
        "{mode} process {child_pid} survived parent death"
    );
}

#[cfg(target_os = "linux")]
fn process_parent_pid(pid: i32) -> Option<i32> {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
fn process_is_alive(pid: i32) -> bool {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    match stat {
        Ok(value) => value
            .split_whitespace()
            .nth(2)
            .is_some_and(|state| state != "Z"),
        Err(_) => false,
    }
}
