//! Git process ownership and mutation safety shared by the UI and MCP adapters.
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

const MAX_OUTPUT: u64 = 16 * 1024 * 1024;

pub mod workspaces;

#[derive(Debug)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

struct OwnedCommand(Child);

impl Drop for OwnedCommand {
    fn drop(&mut self) {
        // Kill the group as well as Git: hooks and SSH must not survive a timeout.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &self.0.id().to_string(), "/T", "/F"])
                .output();
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn capture(mut pipe: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    pipe.by_ref()
        .take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_OUTPUT {
        // Drain without retaining more memory; never present partial Git state as complete.
        std::io::copy(&mut pipe, &mut std::io::sink()).map_err(|error| error.to_string())?;
        return Err("Git output is too large; select a smaller file or comparison".into());
    }
    Ok(bytes)
}

pub fn command_output(
    program: &str,
    args: &[&str],
    cwd: &str,
    env: &[(&str, &str)],
    stdin: Option<&str>,
    timeout: Duration,
) -> Result<CommandOutput, String> {
    let mut command = crate::mcp_parent_guard::command(program);
    command
        .args(args)
        .current_dir(cwd)
        .envs(env.iter().copied());
    command.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if program == "git" {
        command
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = OwnedCommand(
        command
            .spawn()
            .map_err(|error| format!("Could not start {program}: {error}"))?,
    );
    let stdout = child.0.stdout.take().ok_or("Missing command stdout")?;
    let stderr = child.0.stderr.take().ok_or("Missing command stderr")?;
    let stdout = std::thread::spawn(move || capture(stdout));
    let stderr = std::thread::spawn(move || capture(stderr));
    let input = stdin.map(str::to_owned);
    let pipe = child.0.stdin.take();
    let writer = std::thread::spawn(move || -> Result<(), String> {
        if let (Some(input), Some(mut pipe)) = (input, pipe) {
            pipe.write_all(input.as_bytes())
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.0.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            // OwnedCommand cleans up the whole process group, even on error paths.
            return Err(format!(
                "{program} timed out after {}s; its processes were stopped",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(child);
    let stdout = stdout
        .join()
        .map_err(|_| "Command stdout reader failed")??;
    let stderr = stderr
        .join()
        .map_err(|_| "Command stderr reader failed")??;
    writer.join().map_err(|_| "Command input writer failed")??;
    Ok(CommandOutput {
        stdout: String::from_utf8(stdout)
            .map_err(|_| "Git returned non-UTF-8 data; text preview unavailable")?,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        code: status.code().unwrap_or(1),
    })
}

pub fn git_output(path: &str, args: &[&str]) -> Result<String, String> {
    let result = command_output("git", args, path, &[], None, Duration::from_secs(10))?;
    if result.code == 0 {
        Ok(result.stdout)
    } else {
        Err(if result.stderr.is_empty() {
            "Git operation failed".into()
        } else {
            result.stderr
        })
    }
}

/// Native Git protects individual writes. This lock additionally prevents UI/MCP
/// multi-command mutations from interleaving, including across sibling worktrees.
pub struct MutationGuard(File);

impl MutationGuard {
    pub fn acquire(path: &str) -> Result<Self, String> {
        Self::acquire_wait(path, Duration::ZERO)
    }

    pub fn acquire_wait(path: &str, timeout: Duration) -> Result<Self, String> {
        let directory = git_output(
            path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        let directory = directory.strip_suffix('\n').unwrap_or(&directory);
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(Path::new(directory).join("flamel-operation.lock"))
            .map_err(|error| format!("Could not lock repository: {error}"))?;
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(
                        "Another Flamel Git operation is running; wait for it to finish".into(),
                    );
                }
                Err(error) => return Err(format!("Could not lock repository: {error}")),
            }
        }
        Ok(Self(file))
    }
}

impl Drop for MutationGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub fn validate_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.starts_with('-')
        || value.contains("..")
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        Err("Invalid Git branch or remote name".into())
    } else {
        Ok(())
    }
}

/// Pin the lease to the tracking ref the user has actually fetched, even when
/// provider authentication requires pushing to a direct HTTPS URL.
pub fn force_lease(path: &str, remote: &str, branch: &str) -> Result<String, String> {
    validate_name(remote)?;
    validate_name(branch)?;
    let expected = git_output(
        path,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/remotes/{remote}/{branch}"),
        ],
    )
    .map_err(|_| {
        "Fetch this branch and review its changes before replacing remote history".to_string()
    })?;
    Ok(format!(
        "--force-with-lease=refs/heads/{branch}:{}",
        expected.trim()
    ))
}

pub fn branch_needs_upstream(path: &str, branch: &str) -> Result<bool, String> {
    validate_name(branch)?;
    Ok(git_output(
        path,
        &[
            "for-each-ref",
            "--format=%(upstream)",
            &format!("refs/heads/{branch}"),
        ],
    )?
    .trim()
    .is_empty())
}

/// A review token for committing exactly the branch/index the UI displayed.
/// Each worktree owns its index even though mutation locks share the common dir.
pub fn index_revision(path: &str) -> Result<String, String> {
    let mut hash = Sha256::new();
    for args in [
        &["symbolic-ref", "--quiet", "HEAD"][..],
        &["rev-parse", "--verify", "--quiet", "HEAD"][..],
    ] {
        let result = command_output("git", args, path, &[], None, Duration::from_secs(5))?;
        // Exit 1 means a detached HEAD or an unborn branch; both are valid states.
        if result.code != 0 && result.code != 1 {
            return Err(result.stderr);
        }
        hash.update(result.stdout.as_bytes());
        hash.update([0]);
    }
    let index = git_output(
        path,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?;
    let index = index.strip_suffix('\n').unwrap_or(&index);
    match std::fs::read(index) {
        Ok(bytes) => hash.update(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not read Git index: {error}")),
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn repository_web_url(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let address = if let Some(remote) = remote.strip_prefix("git@") {
        let (host, path) = remote.split_once(':')?;
        format!("https://{host}/{path}")
    } else {
        remote.to_owned()
    };
    let mut url = reqwest::Url::parse(&address).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    url.set_username("").ok()?;
    url.set_password(None).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    let path = url
        .path()
        .trim_end_matches('/')
        .strip_suffix(".git")
        .unwrap_or(url.path().trim_end_matches('/'))
        .to_owned();
    url.set_path(&path);
    Some(url.to_string().trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_preserves_filenames_and_rejects_failed_commands() {
        let out = command_output(
            "git",
            &["--version"],
            ".",
            &[],
            None,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(out.code, 0);
        assert!(out.stdout.starts_with("git version "));
        assert!(git_output(".", &["not-a-git-command"]).is_err());
        assert!(validate_name("--force").is_err());
        assert_eq!(
            repository_web_url("https://user:secret@git.flamel.dev/a/repo.git?token=secret"),
            Some("https://git.flamel.dev/a/repo".into())
        );
        assert_eq!(
            repository_web_url("git@github.com:a/repo.git"),
            Some("https://github.com/a/repo".into())
        );
        assert_eq!(repository_web_url("file:///private/repo"), None);
    }

    #[cfg(unix)]
    #[test]
    fn timeout_stops_descendants_without_waiting_for_their_pipes() {
        let start = Instant::now();
        let result = command_output(
            "sh",
            &["-c", "sleep 30 & wait"],
            ".",
            &[],
            None,
            Duration::from_millis(40),
        );
        assert!(result.unwrap_err().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn repository_lock_is_shared_and_released() {
        let directory =
            std::env::temp_dir().join(format!("flamel-git-lock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.to_str().unwrap();
        git_output(path, &["init", "--quiet"]).unwrap();
        let before = index_revision(path).unwrap();
        std::fs::write(directory.join("new.txt"), "stage me").unwrap();
        git_output(path, &["add", "--", "new.txt"]).unwrap();
        assert_ne!(index_revision(path).unwrap(), before);
        let guard = MutationGuard::acquire(path).unwrap();
        assert!(MutationGuard::acquire(path).is_err());
        drop(guard);
        assert!(MutationGuard::acquire(path).is_ok());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
