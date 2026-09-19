use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub const BUILTIN_MCP_SERVER_NAMES: &[&str] = &["flamel"];
pub const BUILTIN_MCP_BINARY_NAMES: &[&str] = &[
    "flamel_mcp",
    "git_mcp",
    "docs_mcp",
    "db_mcp",
    "viz_mcp",
    "browser_mcp",
    "memory_mcp",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpRuntimeStatus {
    Connected,
    Failed,
    NeedsAuth,
    Pending,
    Running,
    Configured,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpSessionPhase {
    NoSession,
    Queued,
    Running,
    Idle,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerRuntimeStatus {
    pub name: String,
    pub status: McpRuntimeStatus,
    pub builtin: bool,
    pub enabled: bool,
    pub scope: String,
    pub transport: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env_keys: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub secret_env_keys: Vec<String>,
}

pub fn phase_from_session(
    continuity_status: &str,
    active: bool,
    last_error: Option<&str>,
) -> McpSessionPhase {
    if active {
        return McpSessionPhase::Running;
    }
    if last_error.is_some() && matches!(continuity_status, "error" | "failed" | "dead") {
        return McpSessionPhase::Failed;
    }
    match continuity_status {
        "queued" | "starting" => McpSessionPhase::Queued,
        "running" | "recovering" | "compacting" => McpSessionPhase::Running,
        "error" | "failed" | "dead" => McpSessionPhase::Failed,
        "idle" | "ready" => McpSessionPhase::Idle,
        _ => McpSessionPhase::NoSession,
    }
}

pub fn runtime_status_for_config(
    name: &str,
    config: &Value,
    builtin: bool,
    enabled: bool,
    scope: impl Into<String>,
    phase: McpSessionPhase,
) -> McpServerRuntimeStatus {
    let command = config.get("command").and_then(Value::as_str).unwrap_or("");
    let command_problem = command_problem(command);
    let (env_keys, secret_env_keys) = env_key_summary(config);

    let (status, detail) = if !enabled {
        (
            McpRuntimeStatus::Configured,
            Some("Server is disabled".to_string()),
        )
    } else if let Some(problem) = command_problem {
        (McpRuntimeStatus::Failed, Some(problem))
    } else {
        match phase {
            McpSessionPhase::NoSession => (
                McpRuntimeStatus::Configured,
                Some("Configured; starts with the next assistant turn".to_string()),
            ),
            McpSessionPhase::Queued => (
                McpRuntimeStatus::Pending,
                Some("Session is queued; MCP startup is pending".to_string()),
            ),
            McpSessionPhase::Running => (
                McpRuntimeStatus::Running,
                Some("Session is active; MCP servers should be available".to_string()),
            ),
            McpSessionPhase::Idle => (
                McpRuntimeStatus::Connected,
                Some("Session has a warm provider process".to_string()),
            ),
            McpSessionPhase::Failed => (
                McpRuntimeStatus::Failed,
                Some("Provider session failed; restart the session to reconnect MCP".to_string()),
            ),
        }
    };

    McpServerRuntimeStatus {
        name: name.to_string(),
        status,
        builtin,
        enabled,
        scope: scope.into(),
        transport: "stdio".to_string(),
        detail,
        env_keys,
        secret_env_keys,
    }
}

pub fn collect_mcp_process_env(mcp_servers: &Value) -> Vec<(String, String)> {
    let mut env = BTreeMap::new();
    let Some(servers) = mcp_servers.as_object() else {
        return Vec::new();
    };

    for config in servers.values() {
        let Some(server_env) = config.get("env").and_then(Value::as_object) else {
            continue;
        };
        for (key, value) in server_env {
            if let Some(value) = env_value_to_string(value) {
                env.insert(key.clone(), value);
            }
        }
    }

    env.into_iter().collect()
}

pub fn is_sensitive_mcp_env_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    key.contains("TOKEN")
        || key.contains("SECRET")
        || key.contains("PASSWORD")
        || key.contains("API_KEY")
        || key.contains("DATABASE_URL")
        || key.contains("AUTH")
        || key.contains("CREDENTIAL")
        || key == "FLAMEL_SYSTEM_PROMPT"
        || key == "FLAMEL_MCP_SERVERS"
}

fn command_problem(command: &str) -> Option<String> {
    let command = command.trim();
    if command.is_empty() {
        return Some("Missing command".to_string());
    }

    let path_like =
        command.contains('/') || command.contains('\\') || Path::new(command).is_absolute();
    if path_like && !Path::new(command).exists() {
        return Some("Configured command path is not available".to_string());
    }

    None
}

fn env_key_summary(config: &Value) -> (Vec<String>, Vec<String>) {
    let Some(env) = config.get("env").and_then(Value::as_object) else {
        return (Vec::new(), Vec::new());
    };

    let mut env_keys = Vec::new();
    let mut secret_env_keys = Vec::new();
    for key in env.keys() {
        if is_sensitive_mcp_env_key(key) {
            secret_env_keys.push(key.clone());
        } else {
            env_keys.push(key.clone());
        }
    }
    env_keys.sort();
    secret_env_keys.sort();
    (env_keys, secret_env_keys)
}

fn env_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        other => Some(other.to_string()),
    }
}

pub fn cleanup_stale_mcp_processes_on_startup() -> usize {
    if std::env::var("FLAMEL_SKIP_MCP_STARTUP_CLEANUP")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
    {
        tracing::info!("mcp startup cleanup skipped by FLAMEL_SKIP_MCP_STARTUP_CLEANUP");
        return 0;
    }

    cleanup_stale_mcp_processes_impl()
}

#[cfg(unix)]
fn cleanup_stale_mcp_processes_impl() -> usize {
    let current_pid = std::process::id();
    let pids = find_stale_mcp_processes(Path::new("/proc"), current_pid);
    if pids.is_empty() {
        return 0;
    }

    let mut signaled = Vec::new();
    for pid in pids {
        match signal_pid(pid, libc::SIGTERM) {
            Ok(()) => signaled.push(pid),
            Err(e) => tracing::warn!("mcp startup cleanup: failed to SIGTERM pid {pid}: {e}"),
        }
    }

    if signaled.is_empty() {
        return 0;
    }

    std::thread::sleep(std::time::Duration::from_millis(250));
    for pid in &signaled {
        if proc_pid_alive(Path::new("/proc"), *pid) {
            if let Err(e) = signal_pid(*pid, libc::SIGKILL) {
                tracing::warn!("mcp startup cleanup: failed to SIGKILL pid {pid}: {e}");
            }
        }
    }

    tracing::warn!(
        count = signaled.len(),
        "mcp startup cleanup: signaled stale MCP processes"
    );
    signaled.len()
}

#[cfg(not(unix))]
fn cleanup_stale_mcp_processes_impl() -> usize {
    0
}

#[cfg(unix)]
fn find_stale_mcp_processes(proc_root: &Path, current_pid: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return Vec::new();
    };

    let mut pids = Vec::new();
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if pid == 0 || pid == current_pid {
            continue;
        }
        if mcp_process_name_from_proc_dir(&entry.path()).is_some()
            && mcp_process_is_orphaned(proc_root, &entry.path())
        {
            pids.push(pid);
        }
    }
    pids
}

#[cfg(unix)]
fn mcp_process_is_orphaned(proc_root: &Path, proc_dir: &Path) -> bool {
    let Some(parent_pid) = std::fs::read_to_string(proc_dir.join("status"))
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("PPid:")
                    .and_then(|value| value.trim().parse::<u32>().ok())
            })
        })
    else {
        // A live CLI-owned sidecar must never be killed merely because process
        // metadata raced or was unavailable.
        return false;
    };
    parent_pid <= 1 || !proc_root.join(parent_pid.to_string()).exists()
}

#[cfg(unix)]
fn mcp_process_name_from_proc_dir(proc_dir: &Path) -> Option<&'static str> {
    std::fs::read_link(proc_dir.join("exe"))
        .ok()
        .and_then(|path| mcp_process_name_from_path(&path))
        .or_else(|| {
            std::fs::read_to_string(proc_dir.join("comm"))
                .ok()
                .and_then(|comm| normalize_mcp_process_name(comm.trim()))
        })
        .or_else(|| {
            std::fs::read(proc_dir.join("cmdline"))
                .ok()
                .and_then(|cmdline| mcp_process_name_from_cmdline(&cmdline))
        })
}

fn mcp_process_name_from_path(path: &Path) -> Option<&'static str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(normalize_mcp_process_name)
}

fn mcp_process_name_from_cmdline(cmdline: &[u8]) -> Option<&'static str> {
    cmdline
        .split(|byte| *byte == 0)
        .find(|part| !part.is_empty())
        .and_then(|part| std::str::from_utf8(part).ok())
        .and_then(|arg0| mcp_process_name_from_path(Path::new(arg0)))
}

fn normalize_mcp_process_name(name: &str) -> Option<&'static str> {
    let name = name.trim().trim_end_matches(" (deleted)");
    let name = name.strip_suffix(".exe").unwrap_or(name);
    BUILTIN_MCP_BINARY_NAMES
        .iter()
        .copied()
        .find(|candidate| *candidate == name)
}

#[cfg(unix)]
fn proc_pid_alive(proc_root: &Path, pid: u32) -> bool {
    proc_root.join(pid.to_string()).exists()
}

#[cfg(unix)]
fn signal_pid(pid: u32, signal: libc::c_int) -> std::io::Result<()> {
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if result == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sensitive_env_key_detection_covers_tokens_and_prompts() {
        assert!(is_sensitive_mcp_env_key("FLAMEL_TOKEN"));
        assert!(is_sensitive_mcp_env_key("DATABASE_URL"));
        assert!(is_sensitive_mcp_env_key("FLAMEL_SYSTEM_PROMPT"));
        assert!(!is_sensitive_mcp_env_key("GIT_WORK_DIR"));
    }

    #[test]
    fn process_env_collection_keeps_values_out_of_status_summary() {
        let config = json!({
            "flamel": {"command": "flamel_mcp", "env": {"FLAMEL_TOKEN": "secret", "FLAMEL_URL": "http://localhost:8181"}},
            "git": {"command": "git_mcp", "env": {"GIT_WORK_DIR": "/repo"}}
        });
        let env = collect_mcp_process_env(&config);
        assert!(env.contains(&("FLAMEL_TOKEN".to_string(), "secret".to_string())));
        assert!(env.contains(&("GIT_WORK_DIR".to_string(), "/repo".to_string())));

        let status = runtime_status_for_config(
            "flamel",
            &config["flamel"],
            true,
            true,
            "built-in",
            McpSessionPhase::Running,
        );
        assert_eq!(status.status, McpRuntimeStatus::Running);
        assert!(status.secret_env_keys.contains(&"FLAMEL_TOKEN".to_string()));
        assert!(!status.env_keys.contains(&"FLAMEL_TOKEN".to_string()));
    }

    #[test]
    fn invalid_path_marks_server_failed() {
        let status = runtime_status_for_config(
            "missing",
            &json!({"command": "/definitely/not/a/flamel/mcp"}),
            false,
            true,
            "user",
            McpSessionPhase::Idle,
        );
        assert_eq!(status.status, McpRuntimeStatus::Failed);
    }

    #[test]
    fn phase_mapping_distinguishes_pending_running_and_idle() {
        assert_eq!(
            phase_from_session("queued", false, None),
            McpSessionPhase::Queued
        );
        assert_eq!(
            phase_from_session("idle", true, None),
            McpSessionPhase::Running
        );
        assert_eq!(
            phase_from_session("idle", false, None),
            McpSessionPhase::Idle
        );
        assert_eq!(
            phase_from_session("failed", false, Some("boom")),
            McpSessionPhase::Failed
        );
    }

    #[test]
    fn mcp_process_name_detection_matches_only_bundled_mcp_binaries() {
        assert_eq!(normalize_mcp_process_name("flamel_mcp"), Some("flamel_mcp"));
        assert_eq!(
            normalize_mcp_process_name("flamel_mcp (deleted)"),
            Some("flamel_mcp")
        );
        assert_eq!(normalize_mcp_process_name("git_mcp.exe"), Some("git_mcp"));
        assert_eq!(normalize_mcp_process_name("some_other_mcp"), None);
        assert_eq!(normalize_mcp_process_name("flamel_server"), None);
    }

    #[test]
    fn mcp_process_name_detection_reads_arg0_from_cmdline() {
        let cmdline = b"/opt/flamel/bin/browser_mcp\0--flag\0value";
        assert_eq!(mcp_process_name_from_cmdline(cmdline), Some("browser_mcp"));
        assert_eq!(mcp_process_name_from_cmdline(b"/usr/bin/codex\0exec"), None);
    }

    #[cfg(unix)]
    #[test]
    fn startup_cleanup_preserves_sidecars_with_a_live_cli_parent() {
        let proc_root =
            std::env::temp_dir().join(format!("flamel_mcp_parent_test_{}", std::process::id()));
        let child = proc_root.join("200");
        std::fs::create_dir_all(proc_root.join("100")).unwrap();
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("status"), "Name:\tflamel_mcp\nPPid:\t100\n").unwrap();
        assert!(!mcp_process_is_orphaned(&proc_root, &child));

        std::fs::remove_dir_all(proc_root.join("100")).unwrap();
        assert!(mcp_process_is_orphaned(&proc_root, &child));
        let _ = std::fs::remove_dir_all(proc_root);
    }
}
