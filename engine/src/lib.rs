#[cfg(feature = "engine")]
pub mod callback;
#[cfg(feature = "engine")]
pub mod identity;
#[cfg(feature = "engine")]
pub mod lifecycle;
#[cfg(feature = "engine")]
pub mod native;
#[cfg(feature = "engine")]
pub mod native_socket;

pub mod mcp_args;
#[cfg(feature = "engine")]
#[path = "mcp_modules/symbols.rs"]
pub mod mcp_symbols;
pub mod mcp_tool_catalog;

#[cfg(feature = "engine")]
pub mod capability_client;
#[cfg(feature = "engine")]
pub mod catalog_trust;
#[cfg(feature = "engine")]
pub mod claude;
#[cfg(feature = "engine")]
pub mod cli_mcp;
#[cfg(feature = "engine")]
pub mod compaction;
#[cfg(feature = "engine")]
pub mod console;
#[cfg(feature = "engine")]
pub mod credential_vault;
#[cfg(feature = "engine")]
pub mod db;
#[cfg(feature = "engine")]
pub mod developer_packs;
#[cfg(feature = "engine")]
pub mod execution;
#[cfg(feature = "engine")]
pub mod git_operations;
#[cfg(feature = "engine")]
pub mod mcp_auth;
#[cfg(feature = "engine")]
pub mod mcp_gateway;
#[cfg(feature = "engine")]
pub mod mcp_lifecycle;
pub mod mcp_parent_guard;
#[cfg(feature = "engine")]
pub mod model_routing;
#[cfg(feature = "engine")]
pub mod openclaw_import;
#[cfg(feature = "engine")]
pub mod ponytail;
#[cfg(feature = "engine")]
pub mod prompt_security;
#[cfg(feature = "engine")]
pub mod provider_cli;
#[cfg(feature = "engine")]
pub mod public_payload;
#[cfg(feature = "engine")]
pub mod realtime;
#[cfg(feature = "engine")]
pub mod routes;
#[cfg(feature = "engine")]
pub mod runtime_logs;
#[cfg(feature = "engine")]
pub mod service_access;
#[cfg(feature = "engine")]
pub mod shell;
#[cfg(feature = "engine")]
pub mod workflow_events;

pub const BUILD_VERSION: &str = match option_env!("FLAMEL_BUILD_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

#[cfg(feature = "engine")]
use {
    claude::agent_pool::AgentPool,
    claude::session_pool::SessionPool,
    console::CommandEventStore,
    db::Db,
    routes::claude_auth::OAuthPkceState,
    shell::TerminalSessions,
    std::collections::{HashMap, HashSet},
    std::path::PathBuf,
    std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(feature = "engine")]
pub const FLAMEL_LOCAL_USER_ID: &str = "flamel-local";
#[cfg(feature = "engine")]
pub const FLAMEL_LOCAL_EMAIL: &str = "local@flamel.local";

#[cfg(feature = "engine")]
pub fn flamel_local_enabled() -> bool {
    std::env::var("FLAMEL_LOCAL")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "1" | "true" | "yes" | "local")
        })
        .unwrap_or(false)
}

#[cfg(feature = "engine")]
pub fn flamel_local_user_home() -> PathBuf {
    if let Ok(path) = std::env::var("FLAMEL_LOCAL_HOME") {
        let path = path.trim();
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(feature = "engine")]
pub fn bundled_binary_path(name: &str) -> PathBuf {
    #[cfg(windows)]
    let binary_name = format!("{name}.exe");
    #[cfg(not(windows))]
    let binary_name = name.to_string();

    if let Some(bin_dir) = std::env::var_os("FLAMEL_BIN_DIR") {
        let candidate = PathBuf::from(bin_dir).join(&binary_name);
        if candidate.exists() {
            return candidate;
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [
                dir.join(&binary_name),
                dir.join("bin").join(&binary_name),
                dir.join("resources").join("bin").join(&binary_name),
            ] {
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }

    PathBuf::from(binary_name)
}

/// Global app state shared across Axum handlers
#[cfg(feature = "engine")]
pub struct WorkspaceState {
    pub service_access: tokio::sync::RwLock<Option<Arc<dyn service_access::ServiceAccess>>>,
    pub db: Arc<Db>,
    pub db_arc: Arc<Db>,
    pub session_pool: Arc<SessionPool>,
    pub agent_pool: Arc<AgentPool>,
    pub port: u16,
    pub callback_endpoint: std::sync::OnceLock<String>,
    pub local_mode: bool,
    shutting_down: AtomicBool,
    pub(crate) runtime_started: AtomicBool,
    shutdown_signal: tokio::sync::watch::Sender<bool>,
    pub mcp_token: String,
    pub abort_controllers: Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
    pub request_to_session: Mutex<HashMap<String, String>>,
    pub permission_resolvers: Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    pub ask_resolvers: Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>,
    pub input_request_waiters: Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
    pub pending_asks: tokio::sync::Mutex<HashMap<String, HashMap<String, serde_json::Value>>>,
    pub oauth_states: Mutex<HashMap<String, OAuthPkceState>>,
    pub live_streams: tokio::sync::Mutex<HashMap<String, tokio::sync::broadcast::Sender<String>>>,
    pub active_sessions: tokio::sync::Mutex<HashSet<String>>,
    pub jwks_cache: tokio::sync::Mutex<Option<catalog_trust::JwksCache>>,
    pub runtime_catalog_sync: tokio::sync::Mutex<()>,

    pub home_dir: PathBuf,

    pub command_events: Arc<CommandEventStore>,

    pub realtime: Arc<realtime::RealtimeHub>,

    pub terminal_sessions: TerminalSessions,
    pub credential_vault: credential_vault::CredentialVault,
    pub capability_cancellations:
        tokio::sync::Mutex<HashMap<String, tokio_util::sync::CancellationToken>>,
}

pub struct LocalSsoPkceState {
    pub verifier: String,
    pub redirect_uri: String,
}

#[cfg(feature = "engine")]
impl WorkspaceState {
    pub fn new(db_path: &str, port: u16) -> Arc<Self> {
        Self::new_with_mode(db_path, port, true)
    }

    pub fn new_with_mode(db_path: &str, port: u16, local_mode: bool) -> Arc<Self> {
        assert!(
            local_mode,
            "The public workspace engine cannot host a Server"
        );
        let db = Arc::new(Db::open(db_path));
        let recoverable_chat_runs = db::realtime::reconcile_interrupted_runs(&db);
        if !recoverable_chat_runs.is_empty() {
            tracing::warn!(
                count = recoverable_chat_runs.len(),
                "found durable chat runs interrupted by the previous process"
            );
        }
        db::capabilities::seed_builtin_catalog(&db);
        db::capabilities::seed_marketplace(&db);
        db::capabilities::seed_skills(&db);
        let interrupted_tasks = db::capabilities::reconcile_interrupted_tasks(&db);
        if interrupted_tasks > 0 {
            tracing::warn!(
                interrupted_tasks,
                "reconciled capability tasks interrupted by the previous shutdown"
            );
        }

        let credential_vault = credential_vault::CredentialVault::load()
            .expect("Failed to initialize encrypted credential vault");
        let migrated_credentials = credential_vault
            .migrate_database_credentials(&db)
            .expect("Failed to authenticate or migrate encrypted credentials");
        if migrated_credentials > 0 {
            tracing::info!(
                migrated_credentials,
                "sealed legacy provider credentials at rest"
            );
        }
        let mcp_token = db::settings::get_or_create_mcp_token(&db, &credential_vault)
            .expect("Failed to initialize encrypted MCP credential");

        let home_dir = if local_mode {
            flamel_local_user_home()
        } else if let Ok(val) = std::env::var("FLAMEL_HOME") {
            PathBuf::from(val)
        } else {
            dirs::home_dir()
                .map(|h| h.join("flamel"))
                .unwrap_or_else(|| PathBuf::from("/flamel"))
        };

        let realtime = realtime::RealtimeHub::new(Arc::clone(&db));
        let (shutdown_signal, _) = tokio::sync::watch::channel(false);
        Arc::new(AppState {
            service_access: tokio::sync::RwLock::new(None),
            db: Arc::clone(&db),
            db_arc: db,
            session_pool: Arc::new(SessionPool::new()),
            agent_pool: Arc::new(AgentPool::new()),
            port,
            callback_endpoint: std::sync::OnceLock::new(),
            local_mode,
            shutting_down: AtomicBool::new(false),
            runtime_started: AtomicBool::new(false),
            shutdown_signal,
            mcp_token,
            abort_controllers: Mutex::new(HashMap::new()),
            request_to_session: Mutex::new(HashMap::new()),
            permission_resolvers: Mutex::new(HashMap::new()),
            ask_resolvers: Mutex::new(HashMap::new()),
            input_request_waiters: Mutex::new(HashMap::new()),
            pending_asks: tokio::sync::Mutex::new(HashMap::new()),
            oauth_states: Mutex::new(HashMap::new()),
            live_streams: tokio::sync::Mutex::new(HashMap::new()),
            active_sessions: tokio::sync::Mutex::new(HashSet::new()),
            jwks_cache: tokio::sync::Mutex::new(None),
            runtime_catalog_sync: tokio::sync::Mutex::new(()),

            home_dir,

            command_events: Arc::new(CommandEventStore::new()),

            realtime,

            terminal_sessions: shell::new_sessions(),
            credential_vault,
            capability_cancellations: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.shutdown_signal.send_replace(true);
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    pub fn shutdown_receiver(&self) -> tokio::sync::watch::Receiver<bool> {
        self.shutdown_signal.subscribe()
    }

    pub async fn shutdown_processes(&self) {
        self.begin_shutdown();
        if self.local_mode {
            cli_mcp::clear_local_cli_runtime();
        }
        let capability_tokens = {
            let mut tokens = self.capability_cancellations.lock().await;
            tokens.drain().map(|(_, token)| token).collect::<Vec<_>>()
        };
        for token in capability_tokens {
            token.cancel();
        }
        self.agent_pool.shutdown_all().await;
        self.session_pool.shutdown_all().await;

        let terminal_sessions = {
            let mut sessions = self.terminal_sessions.lock().unwrap();
            sessions
                .drain()
                .map(|(_, session)| session)
                .collect::<Vec<_>>()
        };
        for mut session in terminal_sessions {
            shell::kill_terminal(&mut session);
        }
    }
}

const RETIRED_MANAGED_AGENT_HASHES: &[(&str, u64)] = &[
    ("ai-engineer", 229450398550275202u64),
    ("analytics-reporter", 7380170791037655554u64),
    ("api-tester", 10418513150554999635u64),
    ("app-store-optimizer", 13738196633663204600u64),
    ("backend-architect", 6185529138431426652u64),
    ("brand-guardian", 1913807870029346683u64),
    ("content-creator", 6941637196210568587u64),
    ("context-fetcher", 10377917045446726360u64),
    ("date-checker", 7945974434209436032u64),
    ("devops-automator", 3365475700866103956u64),
    ("experiment-tracker", 2700724045722569830u64),
    ("feedback-synthesizer", 10578357831831095035u64),
    ("file-creator", 10275032192183040612u64),
    ("finance-tracker", 1634810819420623495u64),
    ("frontend-developer", 8349991218779475192u64),
    ("git-workflow", 1469459500860324265u64),
    ("growth-hacker", 4305822791559125923u64),
    ("infrastructure-maintainer", 16821635161884797366u64),
    ("instagram-curator", 11242034475427587887u64),
    ("joker", 10538223298318030665u64),
    ("knowledge-fetcher", 8158454449154770307u64),
    ("legal-compliance-checker", 1263304141585289252u64),
    ("mobile-app-builder", 15469007012907509940u64),
    ("performance-benchmarker", 6570969751351381364u64),
    ("project-shipper", 2469607984166331530u64),
    ("rapid-prototyper", 13712079254441548641u64),
    ("reddit-community-builder", 11529091751733782005u64),
    ("sprint-prioritizer", 2202854078845083268u64),
    ("studio-coach", 9642012598828729908u64),
    ("studio-producer", 10349115441168106634u64),
    ("support-responder", 87480698848958647u64),
    ("test-results-analyzer", 12663456558032286458u64),
    ("test-writer-fixer", 3549429658342709327u64),
    ("tiktok-strategist", 17986162636641982573u64),
    ("tool-evaluator", 6974737034628025913u64),
    ("trend-researcher", 15676991237690564322u64),
    ("twitter-engager", 13089569543382238527u64),
    ("ui-designer", 10286966240647378169u64),
    ("ux-researcher", 2904032737924624776u64),
    ("visual-storyteller", 14967627485881117597u64),
    ("whimsy-injector", 15403940595369099435u64),
    ("workflow-optimizer", 13325861182628080264u64),
];

fn agent_content_hash(content: &str) -> u64 {
    content
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

/// Remove only exact files written by an older native build. User-authored or edited
/// Claude agents are never touched; Runtime Catalog personas are materialized per run.
pub fn install_agents() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let agents_dir = home.join(".claude").join("agents");
    if !agents_dir.is_dir() {
        return;
    }
    let mut retired = 0usize;
    for (name, expected_hash) in RETIRED_MANAGED_AGENT_HASHES {
        let path = agents_dir.join(format!("{name}.md"));
        let Ok(current) = std::fs::read_to_string(&path) else {
            continue;
        };
        if agent_content_hash(&current) == *expected_hash {
            match std::fs::remove_file(&path) {
                Ok(()) => retired += 1,
                Err(error) => tracing::warn!("install_agents: cannot retire {path:?}: {error}"),
            }
        }
    }
    if retired > 0 {
        tracing::info!("install_agents: retired {retired} managed agent file(s) in {agents_dir:?}");
    }
}

/// Compatibility name for workspace operations; no service state is stored here.
#[cfg(feature = "engine")]
pub type AppState = WorkspaceState;
