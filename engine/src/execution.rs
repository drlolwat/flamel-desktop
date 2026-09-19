//! Process construction supplied by the embedding host, never by renderer input.
use crate::{claude::provider::ProviderKind, db::Db};
use std::collections::BTreeMap;
use tokio::process::Command;
pub trait ExecutionBackend: Send + Sync {
    fn inherit_ambient_environment(&self) -> bool {
        false
    }
    fn command(
        &self,
        program: &str,
        args: &[String],
        home: &str,
        cwd: &str,
        environment: Option<&BTreeMap<String, String>>,
    ) -> Result<Command, String>;
    fn environment(
        &self,
        values: Vec<(String, String)>,
        mcp: Option<&serde_json::Value>,
        home: &str,
        provider: ProviderKind,
        db: Option<&Db>,
        has_credentials: bool,
    ) -> Vec<(String, String)>;
}
pub struct LocalExecution;
impl ExecutionBackend for LocalExecution {
    fn inherit_ambient_environment(&self) -> bool {
        true
    }
    fn command(
        &self,
        program: &str,
        args: &[String],
        _home: &str,
        cwd: &str,
        _environment: Option<&BTreeMap<String, String>>,
    ) -> Result<Command, String> {
        let mut command = Command::new(program);
        command.args(args).current_dir(cwd);
        Ok(command)
    }
    fn environment(
        &self,
        values: Vec<(String, String)>,
        _mcp: Option<&serde_json::Value>,
        _home: &str,
        _provider: ProviderKind,
        _db: Option<&Db>,
        _has_credentials: bool,
    ) -> Vec<(String, String)> {
        values
    }
}
