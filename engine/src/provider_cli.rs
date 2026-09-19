//! Maintain provider CLIs separately from application releases and user auth.
//! Updates are staged and checked before new sessions select the new version.
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

mod probe;

const PROVIDERS: &[(&str, &str)] = &[
    ("codex", "@openai/codex"),
    ("claude", "@anthropic-ai/claude-code"),
];
const REGISTRY: &str = "https://registry.npmjs.org";

fn root() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".flamel/provider-clis"))
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn resolve_executable(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        return None;
    }
    let path = Path::new(value);
    if path.is_absolute() || path.components().count() > 1 {
        return executable(path).then(|| value.to_owned());
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = dir.join(value);
        if executable(&candidate) {
            return Some(candidate.to_string_lossy().into_owned());
        }
        #[cfg(windows)]
        for ext in ["exe", "cmd", "bat"] {
            let candidate = candidate.with_extension(ext);
            if executable(&candidate) {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn package_binary(prefix: &Path, name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        prefix.join(format!("{name}.cmd"))
    }
    #[cfg(not(windows))]
    {
        prefix.join("bin").join(name)
    }
}

fn managed_at(root: &Path, name: &str) -> Option<String> {
    let version = std::fs::read_to_string(root.join(name).join("current")).ok()?;
    let version = version.trim();
    // Never let a damaged pointer redirect a launch outside the managed tree.
    semver::Version::parse(version).ok()?;
    let binary = package_binary(&root.join(name).join(version), name);
    executable(&binary).then(|| binary.to_string_lossy().into_owned())
}

pub fn managed_binary(name: &str) -> Option<String> {
    managed_at(&root()?, name)
}

fn npm_command() -> Command {
    #[cfg(windows)]
    let mut command = {
        let mut c = Command::new("cmd");
        c.args(["/C", "npm.cmd"]);
        c
    };
    #[cfg(not(windows))]
    let mut command = Command::new("npm");
    // Registry downloads/install scripts do not need Flamel's service tokens.
    command.env_clear();
    for key in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "SystemRoot",
        "TEMP",
        "TMP",
        "TMPDIR",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "NO_PROXY",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.args([
        "--registry",
        REGISTRY,
        "--fetch-retries=1",
        "--fetch-timeout=30000",
    ]);
    command.kill_on_drop(true);
    command
}

async fn output(command: &mut Command) -> Result<String, String> {
    let result = tokio::time::timeout(Duration::from_secs(300), command.output())
        .await
        .map_err(|_| "provider CLI command timed out".to_string())?
        .map_err(|e| format!("provider CLI command could not start: {e}"))?;
    if !result.status.success() {
        // Avoid echoing npm configuration or credentials from an error body.
        return Err(format!("provider CLI command failed ({})", result.status));
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

async fn update_one(root: &Path, name: &str, package: &str) -> Result<(), String> {
    update_one_using(root, name, package, npm_command).await
}

async fn update_one_using(
    root: &Path,
    name: &str,
    package: &str,
    make_npm: impl Fn() -> Command,
) -> Result<(), String> {
    let version =
        output(make_npm().args(["view", &format!("{package}@latest"), "version"])).await?;
    semver::Version::parse(&version)
        .map_err(|_| "registry returned an invalid version".to_string())?;
    let provider_root = root.join(name);
    let destination = provider_root.join(&version);
    std::fs::create_dir_all(&provider_root).map_err(|e| e.to_string())?;
    if !executable(&package_binary(&destination, name)) {
        let staging = provider_root.join(format!("staging-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&staging).map_err(|e| e.to_string())?;
        let install = output(
            make_npm()
                .args([
                    "install",
                    "--global",
                    "--no-audit",
                    "--no-fund",
                    "--include=optional",
                    "--prefix",
                ])
                .arg(&staging)
                .arg(format!("{package}@{version}")),
        )
        .await;
        if let Err(error) = install {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        let binary = package_binary(&staging, name);
        if let Err(error) = probe::verify(&binary, name).await {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        // Existing releases are never edited; running processes keep their files.
        std::fs::rename(&staging, &destination).map_err(|e| e.to_string())?;
    } else {
        // A release installed by an older Flamel may only have passed --version.
        // Never promote an existing directory without checking its protocol too.
        probe::verify(&package_binary(&destination, name), name).await?;
    }
    let current = provider_root.join("current");
    if std::fs::read_to_string(&current).ok().as_deref() != Some(version.as_str()) {
        let temp = provider_root.join(format!("current-{}", uuid::Uuid::new_v4()));
        std::fs::write(&temp, &version).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &current).map_err(|e| e.to_string())?;
        tracing::info!(provider = name, %version, "provider CLI installed and verified");
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod update_tests;

/// Check at startup and every six hours. An offline registry never blocks the UI
/// or replaces the last working CLI; missing runtimes stay visibly unavailable.
pub fn start_updater() {
    tokio::spawn(async {
        let Some(root) = root() else {
            return;
        };
        loop {
            if let Err(error) = update_all(&root).await {
                tracing::warn!(%error, "provider CLI update check failed");
            }
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

async fn update_all(root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("update.lock"))
        .map_err(|e| e.to_string())?;
    // Server and worker may share a home. Never update that home concurrently.
    if lock.try_lock().is_err() {
        return Ok(());
    }
    for &(name, package) in PROVIDERS {
        if let Err(error) = update_one(root, name, package).await {
            tracing::warn!(provider = name, %error, "provider CLI update failed; retaining existing installation");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_commands_are_not_reported_as_installed() {
        assert!(resolve_executable("flamel-nonexistent-provider-4c72").is_none());
        assert!(resolve_executable("").is_none());
        assert!(resolve_executable("/definitely/missing/codex").is_none());
    }

    #[test]
    fn managed_pointer_cannot_escape_or_select_missing_release() {
        let dir = std::env::temp_dir().join(format!("flamel-cli-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("codex")).unwrap();
        for value in ["../../outside", "1.2.3/../../outside", "1.2.3"] {
            std::fs::write(dir.join("codex/current"), value).unwrap();
            assert!(managed_at(&dir, "codex").is_none());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
