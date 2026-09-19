use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
mod installation;

const MANAGED_SERVERS: [(&str, &str); 1] = [("flamel", "flamel_mcp")];
const RETIRED_PUBLIC_SERVERS: [&str; 6] = ["git", "docs", "db", "viz", "browser", "memory"];
pub const MCP_PROFILE_ENV: &str = "FLAMEL_MCP_PROFILE";
pub const PERSONAL_ASSISTANT_PROFILE: &str = "personal_assistant";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalCliRuntime {
    pub pid: u32,
    pub api_base: String,
    pub mcp_token: String,
    pub user_id: String,
}

pub fn current_project_path() -> String {
    project_path_from(
        std::env::var("FLAMEL_PROJECT_PATH").ok(),
        std::env::current_dir().ok(),
    )
}

fn project_path_from(configured: Option<String>, cwd: Option<PathBuf>) -> String {
    configured
        .filter(|path| !path.trim().is_empty())
        .or_else(|| cwd.map(|path| path.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".to_string())
}

pub fn standalone_mcp_servers() -> Value {
    let project_path = PathBuf::from(current_project_path());
    let user_home = dirs::home_dir().unwrap_or_default();
    mcp_servers_for(&project_path, &user_home, None)
}

pub(crate) fn mcp_servers_for(
    project_path: &Path,
    user_home: &Path,
    profile: Option<&str>,
) -> Value {
    let mut servers = Map::new();
    for (name, binary) in MANAGED_SERVERS {
        let mut config = json!({
            "type": "stdio",
            "command": crate::bundled_binary_path(binary).to_string_lossy(),
            "args": [],
            "env_vars": ["FLAMEL_TOKEN", "FLAMEL_URL", "FLAMEL_USER_ID"],
            "env": {
                "FLAMEL_PROJECT_PATH": project_path.to_string_lossy(),
                "GIT_WORK_DIR": project_path.to_string_lossy(),
                "FLAMEL_USER_HOME": user_home.to_string_lossy(),
                "FLAMEL_SHELL_UNRESTRICTED": "1",
            },
        });
        if let Some(profile) = profile.filter(|value| !value.trim().is_empty()) {
            config["env"][MCP_PROFILE_ENV] = json!(profile);
        }
        if name == "flamel" {
            config["tool_timeout_sec"] = json!(900);
            config["startup_timeout_sec"] = json!(30);
        } else if name == "git" {
            config["tool_timeout_sec"] = json!(600);
        }
        servers.insert(name.to_string(), config);
    }
    Value::Object(servers)
}

pub fn install_cli_mcp_integrations() {
    let Some(home) = dirs::home_dir() else {
        tracing::warn!("CLI MCP integration: cannot determine home directory");
        return;
    };
    let mut servers = standalone_mcp_servers();
    if let Err(error) = make_commands_durable(&mut servers, &home) {
        tracing::warn!("CLI MCP integration left existing config intact: {error}");
        return;
    }
    if let Err(error) = merge_codex_config(&home.join(".codex/config.toml"), &servers) {
        tracing::warn!("CLI MCP integration: Codex config failed: {error}");
    }
    if let Err(error) = merge_claude_config(&home.join(".claude.json"), &servers) {
        tracing::warn!("CLI MCP integration: Claude config failed: {error}");
    }
}

pub fn install_cli_mcp_integrations_at(
    codex_home: &Path,
    claude_home: &Path,
    project_path: &Path,
    user_home: &Path,
    profile: Option<&str>,
) -> Result<(), String> {
    let servers = mcp_servers_for(project_path, user_home, profile);
    merge_codex_config(&codex_home.join("config.toml"), &servers)?;
    merge_claude_config(&claude_home.join(".claude.json"), &servers)
}

fn make_commands_durable(servers: &mut Value, home: &Path) -> Result<(), String> {
    for (name, binary) in MANAGED_SERVERS {
        let installed = installation::durable_binary(&crate::bundled_binary_path(binary), home)?;
        servers[name]["command"] = json!(installed.to_string_lossy());
    }
    Ok(())
}

fn read_config(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!(
            "Cannot read {}; existing config was not changed: {error}",
            path.display()
        )),
    }
}

fn is_retired_managed_command(command: Option<&str>) -> bool {
    command
        .and_then(|command| Path::new(command).file_stem())
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            [
                "git_mcp",
                "docs_mcp",
                "db_mcp",
                "viz_mcp",
                "browser_mcp",
                "memory_mcp",
            ]
            .contains(&name)
        })
}

fn merge_codex_config(config_path: &Path, servers: &Value) -> Result<(), String> {
    if let Some(config_dir) = config_path.parent() {
        std::fs::create_dir_all(config_dir).map_err(|error| error.to_string())?;
    }
    let existing = read_config(config_path)?;
    let mut document = if existing.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        existing
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| {
                let backup = config_path.with_extension("toml.flamel-invalid");
                let _ = std::fs::copy(&config_path, &backup);
                format!(
                    "invalid {}; backed up to {}: {error}",
                    config_path.display(),
                    backup.display()
                )
            })?
    };
    if document.get("mcp_servers").is_none() {
        document["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let target = document["mcp_servers"]
        .as_table_mut()
        .ok_or_else(|| "mcp_servers is not a TOML table".to_string())?;
    let source = servers
        .as_object()
        .ok_or_else(|| "standalone MCP catalog is not an object".to_string())?;
    for name in RETIRED_PUBLIC_SERVERS {
        if is_retired_managed_command(
            target
                .get(name)
                .and_then(|item| item.get("command"))
                .and_then(toml_edit::Item::as_str),
        ) {
            target.remove(name);
        }
    }
    for (name, _) in MANAGED_SERVERS {
        target.remove(name);
        let Some(config) = source.get(name) else {
            continue;
        };
        let command = config
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{name} command is missing"))?;
        let mut table = toml_edit::Table::new();
        table["command"] = toml_edit::value(command);
        table["args"] = toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::new()));
        if let Some(env_vars) = config.get("env_vars").and_then(Value::as_array) {
            let mut values = toml_edit::Array::new();
            for value in env_vars.iter().filter_map(Value::as_str) {
                values.push(value);
            }
            table["env_vars"] = toml_edit::Item::Value(toml_edit::Value::Array(values));
        }
        table["enabled"] = toml_edit::value(true);
        if let Some(timeout) = config.get("tool_timeout_sec").and_then(Value::as_i64) {
            table["tool_timeout_sec"] = toml_edit::value(timeout);
        }
        if let Some(timeout) = config.get("startup_timeout_sec").and_then(Value::as_i64) {
            table["startup_timeout_sec"] = toml_edit::value(timeout);
        }
        if let Some(env) = config.get("env").and_then(Value::as_object) {
            let mut env_table = toml_edit::InlineTable::new();
            for (key, value) in env {
                if let Some(value) = value.as_str() {
                    env_table.insert(key, toml_edit::Value::from(value));
                }
            }
            table["env"] = toml_edit::Item::Value(toml_edit::Value::InlineTable(env_table));
        }
        target.insert(name, toml_edit::Item::Table(table));
    }
    write_private(&config_path, document.to_string().as_bytes()).map_err(|error| error.to_string())
}

fn merge_claude_config(config_path: &Path, servers: &Value) -> Result<(), String> {
    let existing = read_config(config_path)?;
    let mut config = if existing.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(&existing).map_err(|error| {
            let backup = config_path.with_extension("json.flamel-invalid");
            let _ = std::fs::copy(&config_path, &backup);
            format!(
                "invalid {}; backed up to {}: {error}",
                config_path.display(),
                backup.display()
            )
        })?
    };
    let root = config
        .as_object_mut()
        .ok_or_else(|| "Claude user config is not a JSON object".to_string())?;
    let target = root
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "Claude mcpServers is not a JSON object".to_string())?;
    let source = servers
        .as_object()
        .ok_or_else(|| "standalone MCP catalog is not an object".to_string())?;
    for name in RETIRED_PUBLIC_SERVERS {
        if is_retired_managed_command(
            target
                .get(name)
                .and_then(|value| value.get("command"))
                .and_then(Value::as_str),
        ) {
            target.remove(name);
        }
    }
    for (name, _) in MANAGED_SERVERS {
        if let Some(server) = source.get(name) {
            let mut server = server.clone();
            if let Some(object) = server.as_object_mut() {
                object.remove("tool_timeout_sec");
                object.remove("startup_timeout_sec");
                object.remove("env_vars");
            }
            target.insert(name.to_string(), server);
        }
    }
    let body = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
    write_private(&config_path, &body).map_err(|error| error.to_string())
}

pub fn write_local_cli_runtime(api_base: &str, mcp_token: &str) -> Result<(), String> {
    write_local_cli_runtime_for(api_base, mcp_token, crate::FLAMEL_LOCAL_USER_ID)
}

pub fn write_local_cli_runtime_for(
    api_base: &str,
    mcp_token: &str,
    user_id: &str,
) -> Result<(), String> {
    let runtime = LocalCliRuntime {
        pid: std::process::id(),
        api_base: api_base.to_string(),
        mcp_token: mcp_token.to_string(),
        user_id: user_id.to_string(),
    };
    let body = serde_json::to_vec(&runtime).map_err(|error| error.to_string())?;
    write_private(&runtime_path(), &body).map_err(|error| error.to_string())
}

pub fn discover_local_cli_runtime() -> Option<LocalCliRuntime> {
    let runtime: LocalCliRuntime =
        serde_json::from_slice(&std::fs::read(runtime_path()).ok()?).ok()?;
    process_is_alive(runtime.pid).then_some(runtime)
}

pub fn clear_local_cli_runtime() {
    let path = runtime_path();
    let belongs_to_current_process = std::fs::read(&path)
        .ok()
        .and_then(|body| serde_json::from_slice::<LocalCliRuntime>(&body).ok())
        .is_some_and(|runtime| runtime.pid == std::process::id());
    if belongs_to_current_process {
        let _ = std::fs::remove_file(path);
    }
}

fn runtime_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("@flamel")
        .join("cli-runtime.json")
}

fn forge_credential_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("@flamel")
        .join("forge-credential")
}

/// Persist a personal Flamel Git credential for globally registered CLI MCPs.
/// The credential is encrypted with the same machine-local vault as Desktop.
pub fn store_standalone_forge_token(token: &str) -> Result<(), String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Flamel Git credential cannot be empty".into());
    }
    let vault = crate::credential_vault::CredentialVault::load()?;
    let encrypted = vault.seal(token)?;
    write_private(&forge_credential_path(), encrypted.as_bytes()).map_err(|error| error.to_string())
}

/// Resolve the personal credential used when no Flamel Server/Desktop runtime is
/// active. An explicit environment value is useful for ephemeral CI; personal
/// machines use the encrypted machine-local file.
pub fn discover_standalone_forge_token() -> Option<String> {
    if let Some(token) = std::env::var("FLAMEL_GIT_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return Some(token);
    }
    let encrypted = std::fs::read_to_string(forge_credential_path()).ok()?;
    crate::credential_vault::CredentialVault::load()
        .ok()?
        .open(encrypted.trim())
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn process_is_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        true
    }
}

fn write_private(path: &Path, body: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        use std::io::Write;
        let mut file = options.open(&temp)?;
        file.write_all(body)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_forge_credential_rejects_empty_values() {
        assert!(store_standalone_forge_token("  ").is_err());
    }

    #[test]
    fn project_path_prefers_explicit_value_then_cli_working_directory() {
        assert_eq!(
            project_path_from(Some("/explicit".into()), Some(PathBuf::from("/cwd"))),
            "/explicit"
        );
        assert_eq!(
            project_path_from(Some("".into()), Some(PathBuf::from("/cwd"))),
            "/cwd"
        );
    }

    #[test]
    fn standalone_flamel_server_enables_local_machine_management() {
        let servers = standalone_mcp_servers();
        let env = &servers["flamel"]["env"];
        assert_eq!(env["FLAMEL_SHELL_UNRESTRICTED"], "1");
        assert!(env["FLAMEL_USER_HOME"].is_string());
    }

    #[test]
    fn cli_config_merge_preserves_native_settings_and_custom_servers() {
        let home =
            std::env::temp_dir().join(format!("flamel_cli_mcp_config_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".codex")).expect("temp config dir");
        std::fs::write(
            home.join(".codex/config.toml"),
            "model = \"gpt-5.6-sol\"\n[mcp_servers.custom]\ncommand = \"custom\"\n[mcp_servers.git]\ncommand = \"third-party-git\"\n[mcp_servers.docs]\ncommand = \"/old/docs_mcp\"\n",
        )
        .expect("seed Codex config");
        std::fs::write(
            home.join(".claude.json"),
            r#"{"theme":"dark","mcpServers":{"custom":{"command":"custom"},"git":{"command":"third-party-git"},"docs":{"command":"/old/docs_mcp"}}}"#,
        )
        .expect("seed Claude config");

        let servers = standalone_mcp_servers();
        merge_codex_config(&home.join(".codex/config.toml"), &servers).expect("merge Codex config");
        merge_claude_config(&home.join(".claude.json"), &servers).expect("merge Claude config");

        let codex = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(codex.contains("model = \"gpt-5.6-sol\""));
        assert!(codex.contains("[mcp_servers.custom]"));
        assert!(codex.contains("[mcp_servers.flamel]"));
        assert!(codex.contains("third-party-git"));
        assert!(!codex.contains("docs_mcp"));
        let claude: Value =
            serde_json::from_slice(&std::fs::read(home.join(".claude.json")).unwrap()).unwrap();
        assert_eq!(claude["theme"], "dark");
        assert_eq!(claude["mcpServers"]["custom"]["command"], "custom");
        assert!(claude["mcpServers"]["flamel"]["command"].is_string());
        assert_eq!(claude["mcpServers"]["git"]["command"], "third-party-git");
        assert!(claude["mcpServers"].get("docs").is_none());

        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn personal_assistant_install_uses_explicit_homes_and_native_profile() {
        let root = std::env::temp_dir().join(format!(
            "flamel_personal_assistant_mcp_config_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let codex_home = root.join("codex-home");
        let claude_home = root.join("claude-home");
        let workspace = root.join("workspace");
        let user_home = root.join("user-home");

        install_cli_mcp_integrations_at(
            &codex_home,
            &claude_home,
            &workspace,
            &user_home,
            Some(PERSONAL_ASSISTANT_PROFILE),
        )
        .expect("install personal assistant MCP configs");

        let codex = std::fs::read_to_string(codex_home.join("config.toml")).unwrap();
        let claude = std::fs::read_to_string(claude_home.join(".claude.json")).unwrap();
        for body in [&codex, &claude] {
            assert!(body.contains(PERSONAL_ASSISTANT_PROFILE));
            assert!(!body.contains("OPENAI_API_KEY"));
            assert!(!body.contains("ANTHROPIC_API_KEY"));
        }
        // Compare decoded values: Windows backslashes are escaped in both formats.
        let codex_config = codex.parse::<toml_edit::DocumentMut>().unwrap();
        let claude_config: Value = serde_json::from_str(&claude).unwrap();
        for (name, _) in MANAGED_SERVERS {
            assert_eq!(
                codex_config["mcp_servers"][name]["env"]["FLAMEL_PROJECT_PATH"].as_str(),
                Some(workspace.to_string_lossy().as_ref())
            );
            assert_eq!(
                claude_config["mcpServers"][name]["env"]["FLAMEL_PROJECT_PATH"].as_str(),
                Some(workspace.to_string_lossy().as_ref())
            );
        }
        assert!(codex.contains(MCP_PROFILE_ENV));
        assert!(
            codex.contains("env_vars = [\"FLAMEL_TOKEN\", \"FLAMEL_URL\", \"FLAMEL_USER_ID\"]")
        );
        assert!(claude.contains(MCP_PROFILE_ENV));
        assert!(!claude.contains("env_vars"));
        assert!(!claude.contains("FLAMEL_TOKEN"));

        let _ = std::fs::remove_dir_all(root);
    }
}
