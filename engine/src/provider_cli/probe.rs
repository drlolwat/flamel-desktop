//! No-auth, no-inference protocol checks before a CLI becomes the launch target.
//! https://learn.chatgpt.com/docs/app-server
//! https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/_internal/query.py
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

const REQUEST_ID: &str = "flamel-provider-compatibility";
const MAX_REPLY_BYTES: u64 = 512 * 1024;

struct ProbeHome(PathBuf);

impl ProbeHome {
    fn create() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("flamel-cli-probe-{}", uuid::Uuid::new_v4()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).map_err(|e| e.to_string())?;
        let home = Self(path);
        for dir in ["codex", "claude", "config", "cache", "data"] {
            std::fs::create_dir(home.0.join(dir)).map_err(|e| e.to_string())?;
        }
        Ok(home)
    }
}

impl Drop for ProbeHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct ProbeProcess(Child);

impl ProbeProcess {
    fn kill_group(&mut self) {
        if let Some(pid) = self.0.id() {
            // npm's Codex launcher can have a native child. Closing only the
            // launcher must not leave a detached probe running after a timeout.
            let _ = crate::console::kill_process_group(pid as i32);
        }
        let _ = self.0.start_kill();
    }
}

impl Drop for ProbeProcess {
    fn drop(&mut self) {
        self.kill_group();
    }
}

fn isolated_command(binary: &Path, home: &Path) -> Command {
    let mut command = Command::new(binary);
    command.env_clear();
    for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .current_dir(home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CODEX_HOME", home.join("codex"))
        .env("CLAUDE_CONFIG_DIR", home.join("claude"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("DISABLE_AUTOUPDATER", "1")
        .env("DISABLE_TELEMETRY", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    command
}

fn handshake_response(name: &str, value: &Value) -> Option<bool> {
    match name {
        "codex" if value.get("id").and_then(Value::as_str) == Some(REQUEST_ID) => Some(
            value.get("error").is_none()
                && value
                    .pointer("/result/userAgent")
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.is_empty()),
        ),
        "claude"
            if value.get("type").and_then(Value::as_str) == Some("control_response")
                && value
                    .pointer("/response/request_id")
                    .and_then(Value::as_str)
                    == Some(REQUEST_ID) =>
        {
            Some(
                value.pointer("/response/subtype").and_then(Value::as_str) == Some("success")
                    && value
                        .pointer("/response/response")
                        .is_some_and(Value::is_object),
            )
        }
        _ => None,
    }
}

pub(super) async fn verify(binary: &Path, name: &str) -> Result<(), String> {
    verify_with_timeout(binary, name, Duration::from_secs(15)).await
}

pub(super) async fn verify_with_timeout(
    binary: &Path,
    name: &str,
    deadline: Duration,
) -> Result<(), String> {
    let home = ProbeHome::create()?;
    let mut command = isolated_command(binary, &home.0);
    let request = match name {
        "codex" => {
            command.arg("app-server");
            json!({"id":REQUEST_ID,"method":"initialize","params":{"clientInfo":{"name":"flamel","version":env!("CARGO_PKG_VERSION")}}})
        }
        "claude" => {
            // ponytail: reuse actual launch arguments, including hidden flags;
            // --help omission is not evidence that a CLI rejects an option.
            command.args(crate::claude::spawn::build_args(
                None, "default", None, None, None,
            ));
            command.args([
                "--setting-sources",
                "",
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
                "--no-session-persistence",
                "--settings",
                "{\"disableAllHooks\":true}",
            ]);
            json!({"type":"control_request","request_id":REQUEST_ID,"request":{"subtype":"initialize","hooks":{}}})
        }
        _ => return Err("Unknown provider protocol".to_owned()),
    };
    let mut process = ProbeProcess(
        command
            .spawn()
            .map_err(|e| format!("CLI protocol probe could not start: {e}"))?,
    );
    let result = tokio::time::timeout(deadline, async {
        let stdin = process
            .0
            .stdin
            .as_mut()
            .ok_or("CLI probe has no input pipe")?;
        stdin
            .write_all(format!("{request}\n").as_bytes())
            .await
            .map_err(|_| "CLI probe input failed")?;
        let stdout = process
            .0
            .stdout
            .take()
            .ok_or("CLI probe has no output pipe")?;
        let mut lines = BufReader::new(stdout.take(MAX_REPLY_BYTES)).lines();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|_| "CLI probe output failed")?
        {
            let value: Value = serde_json::from_str(&line)
                .map_err(|_| "CLI returned malformed protocol output")?;
            match handshake_response(name, &value) {
                Some(true) => {
                    if name == "codex" {
                        stdin
                            .write_all(b"{\"method\":\"initialized\"}\n")
                            .await
                            .map_err(|_| "CLI initialization acknowledgement failed")?;
                    }
                    return Ok(());
                }
                Some(false) => return Err("CLI initialization was rejected or incompatible"),
                None => {}
            }
        }
        Err("CLI exited or exceeded its reply limit before initialization")
    })
    .await
    .map_err(|_| "CLI protocol probe timed out".to_owned())
    .and_then(|result| result.map_err(str::to_owned));
    process.kill_group();
    let _ = process.0.wait().await;
    result
}
