use super::credential_files::{
    CredentialMaterializationError, ensure_private_directory, write_private_file,
};
use crate::db::Db;
use crate::db::claude_auth::ClaudeCredentials;
use crate::db::settings::{get_anthropic_api_key, get_claude_access_mode};
use std::path::PathBuf;

/// Built-in Claude CLI tools that flamel replaces with its own MCP equivalents.
/// These are passed to every `claude` spawn via `--disallowedTools` so the model
/// only sees Flamel's single public MCP namespace (`mcp__flamel__*`).
///
/// The built-in `Agent` is blocked here and replaced by `mcp__flamel__Agent`,
/// which recursively spawns `claude` with the same restrictions — so sub-agents
/// inherit the same tool discipline as the parent.
pub const DISALLOWED_BUILTIN_TOOLS: &[&str] = &[
    "Agent",     // → mcp__flamel__Agent (enforces same restrictions on sub-agents)
    "Task",      // alias for Agent in some CLI versions
    "Read",      // → mcp__flamel__read_file / flamel_read
    "Write",     // → mcp__flamel__create_text_file
    "Edit",      // → mcp__flamel__replace_content / flamel_replace
    "Bash",      // → flamel_shell / shell_run
    "Grep",      // → flamel_search
    "Glob",      // → flamel_glob
    "TodoWrite", // → mcp__flamel__plan
    "NotebookEdit",
];

/// Claude Code's largest supported native auto-compaction window. Flamel owns
/// compaction: it writes a validated durable checkpoint, preserves recent
/// source events, restores accepted Plan state, and then starts a fresh
/// provider thread. Keeping Claude's opaque compactor at its maximum while
/// Flamel checkpoints earlier prevents it from replacing that recovery path.
pub const CLAUDE_NATIVE_COMPACTION_CEILING: &str = "1000000";

/// Comma-joined form, ready to pass to `--disallowedTools`.
pub fn disallowed_tools_csv() -> String {
    DISALLOWED_BUILTIN_TOOLS.join(",")
}

/// Find the Claude CLI binary path
pub fn find_claude_cli() -> String {
    if let Ok(p) = std::env::var("CLAUDE_CLI_PATH") {
        return p;
    }

    if let Some(path) = crate::provider_cli::managed_binary("claude") {
        return path;
    }

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let candidates = [
        home.join(".claude").join("local").join("claude"),
        PathBuf::from("/usr/local/bin/claude"),
        PathBuf::from("/usr/bin/claude"),
        home.join(".flamel").join("bin").join("claude"),
        home.join(".flamel")
            .join("npm-global")
            .join("bin")
            .join("claude"),
        home.join(".npm-global").join("bin").join("claude"),
    ];

    for c in &candidates {
        if c.exists() {
            let path = c.to_string_lossy().to_string();
            tracing::debug!("found claude CLI at: {}", path);
            return path;
        }
    }

    // Fall back to PATH
    tracing::debug!("using claude CLI from PATH");
    "claude".to_string()
}

/// Build CLI arguments for the Claude CLI
pub fn build_args(
    model: Option<&str>,
    permission_mode: &str,
    system_prompt_append: Option<&str>,
    resume: Option<&str>,
    mcp_servers: Option<&serde_json::Value>,
) -> Vec<String> {
    let mut args = vec![
        "--print".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--include-partial-messages".to_string(),
        "--autocompact".to_string(),
        CLAUDE_NATIVE_COMPACTION_CEILING.to_string(),
        // Block built-in tools that flamel replaces with MCP equivalents.
        // The built-in Agent is replaced by mcp__flamel__Agent so sub-agents
        // inherit flamel's tool restrictions recursively.
        "--disallowedTools".to_string(),
        disallowed_tools_csv(),
    ];

    if let Some(m) = model {
        args.push("--model".to_string());
        args.push(m.to_string());
    }

    if let Some(sp) = system_prompt_append {
        args.push("--append-system-prompt".to_string());
        args.push(sp.to_string());
    }

    if permission_mode == "bypassPermissions" {
        args.push("--dangerously-skip-permissions".to_string());
    } else if permission_mode != "default" {
        args.push("--permission-mode".to_string());
        args.push(permission_mode.to_string());
    }

    if let Some(r) = resume {
        args.push("--resume".to_string());
        args.push(r.to_string());
    }

    if let Some(servers) = mcp_servers {
        if let Some(obj) = servers.as_object() {
            if !obj.is_empty() {
                let config = serde_json::json!({"mcpServers": servers});
                args.push("--mcp-config".to_string());
                args.push(config.to_string());
            }
        }
    }

    tracing::debug!("CLI args: {:?}", args);
    args
}

/// Build environment variables for the Claude CLI subprocess.
/// If `creds` is provided, writes a per-user `.credentials.json` and sets `CLAUDE_CONFIG_DIR`.
/// If `db` is provided and no creds, injects a stored ANTHROPIC_API_KEY if present.
/// `user_home` should be the sandboxed user home dir (for bwrap); creds are written there
/// so the sandboxed CLI process can access them.
pub fn build_env(
    creds: Option<&ClaudeCredentials>,
    db: Option<&Db>,
    user_home: &str,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            k != "CLAUDECODE"
                && k != "NODE_OPTIONS"
                && k != "CLAUDE_CONFIG_DIR"
                && k != "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE"
        })
        .collect();

    crate::claude::provider::add_browser_automation_env(&mut env);
    env.push(("CLAUDE_CODE_ENTRYPOINT".to_string(), "sdk-ts".to_string()));

    // Org API key mode: all sessions use the shared org key, ignoring individual creds
    if let Some(db) = db {
        if get_claude_access_mode(db) == "org_api_key" {
            if let Some(key) = get_anthropic_api_key(db) {
                if !key.is_empty() {
                    env.retain(|(k, _)| k != "ANTHROPIC_API_KEY");
                    env.push(("ANTHROPIC_API_KEY".to_string(), key));
                    return env;
                }
            }
        }
    }

    // Individual mode: use per-user claude.ai OAuth creds or fallback to API key
    if let Some(c) = creds {
        match write_user_credentials(c, user_home) {
            Ok(config_dir) => env.push(("CLAUDE_CONFIG_DIR".to_string(), config_dir)),
            Err(error) => {
                tracing::error!("failed to materialize Claude credentials: {error}");
            }
        }
    } else if let Some(db) = db {
        if !env
            .iter()
            .any(|(k, v)| k == "ANTHROPIC_API_KEY" && !v.is_empty())
        {
            if let Some(key) = get_anthropic_api_key(db) {
                if !key.is_empty() {
                    env.push(("ANTHROPIC_API_KEY".to_string(), key));
                }
            }
        }
    }

    env
}

/// Write `.credentials.json` to a per-user config dir and return the dir path.
/// When `user_home` is non-empty (bwrap sandbox), writes under `{user_home}/.claude/`
/// so the sandboxed process can access the file. Otherwise falls back to data_local_dir.
pub(crate) fn write_user_credentials(
    creds: &ClaudeCredentials,
    user_home: &str,
) -> Result<String, CredentialMaterializationError> {
    let config_dir = if !user_home.is_empty() {
        // Write inside user_home so bwrap-sandboxed CLI can see it
        PathBuf::from(user_home).join(".claude")
    } else {
        let data_dir = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        data_dir
            .join("flamel")
            .join("users")
            .join(&creds.user_id)
            .join("claude")
    };

    ensure_private_directory(&config_dir)?;

    // expires_at in the credentials file is in milliseconds
    let expires_at_ms = creds.expires_at * 1000;
    let scopes: Vec<&str> = creds.scopes.split_whitespace().collect();

    let mut oauth = serde_json::json!({
        "accessToken": creds.access_token,
        "expiresAt": expires_at_ms,
        "scopes": scopes,
    });
    if let Some(ref rt) = creds.refresh_token {
        oauth["refreshToken"] = serde_json::Value::String(rt.clone());
    }

    let credentials = serde_json::json!({"claudeAiOauth": oauth});
    let serialized = serde_json::to_vec_pretty(&credentials).map_err(|error| {
        CredentialMaterializationError::new("failed to serialize Claude credentials", None, error)
    })?;
    let creds_path = config_dir.join(".credentials.json");
    write_private_file(&creds_path, &serialized)?;

    tracing::info!("wrote Claude credentials to {}", creds_path.display());
    Ok(config_dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod credential_tests {
    use super::*;

    #[test]
    fn claude_sessions_defer_native_compaction_to_flamel() {
        let args = build_args(None, "default", None, None, None);
        let index = args
            .iter()
            .position(|arg| arg == "--autocompact")
            .expect("Claude autocompact ceiling");
        assert_eq!(
            args.get(index + 1).map(String::as_str),
            Some(CLAUDE_NATIVE_COMPACTION_CEILING)
        );
    }

    #[test]
    fn materializes_native_claude_oauth_with_private_permissions() {
        let user_home = std::env::temp_dir().join(format!(
            "flamel-claude-credentials-{}",
            uuid::Uuid::new_v4()
        ));
        let config_dir = user_home.join(".claude");
        let credentials_path = config_dir.join(".credentials.json");
        std::fs::create_dir_all(&config_dir).expect("create Claude test directory");
        std::fs::write(&credentials_path, b"stale")
            .expect("create existing Claude credentials file");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o755))
                .expect("set broad Claude test directory mode");
            std::fs::set_permissions(&credentials_path, std::fs::Permissions::from_mode(0o644))
                .expect("set broad Claude test file mode");
        }

        let credentials = ClaudeCredentials {
            user_id: "test-user".to_string(),
            access_token: "synthetic-access-token".to_string(),
            refresh_token: Some("synthetic-refresh-token".to_string()),
            expires_at: 1_900_000_000,
            scopes: "user:inference user:profile".to_string(),
            updated_at: "2026-08-17T00:00:00Z".to_string(),
        };

        let materialized_dir = write_user_credentials(
            &credentials,
            user_home.to_str().expect("UTF-8 Claude test path"),
        )
        .expect("materialize Claude credentials");
        assert_eq!(PathBuf::from(materialized_dir), config_dir);

        let stored: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&credentials_path).expect("read materialized Claude credentials"),
        )
        .expect("parse materialized Claude credentials");
        let oauth = &stored["claudeAiOauth"];
        assert!(oauth["accessToken"].is_string());
        assert!(oauth["refreshToken"].is_string());
        assert!(oauth["expiresAt"].is_number());
        assert!(oauth["scopes"].is_array());
        assert!(stored.get("ANTHROPIC_API_KEY").is_none());

        let env = build_env(
            Some(&credentials),
            None,
            user_home.to_str().expect("UTF-8 Claude test path"),
        );
        assert_eq!(
            env.iter()
                .find(|(key, _)| key == "CLAUDE_CONFIG_DIR")
                .map(|(_, value)| value.as_str()),
            config_dir.to_str()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                std::fs::metadata(&config_dir)
                    .expect("read Claude directory metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&credentials_path)
                    .expect("read Claude credentials metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        std::fs::remove_dir_all(&user_home).expect("remove Claude test directory");
    }
}
