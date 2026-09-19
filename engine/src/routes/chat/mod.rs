use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::Arc,
};
use tokio::sync::broadcast;

use crate::AppState;
use crate::claude::provider::{ProviderCredentials as ProvCreds, ProviderKind, get_provider};
use crate::claude::session_pool::{SessionPool, build_recovery_context_for_provider};
use crate::compaction::{
    CompactionPhase, bounded_context_tokens, codex_compaction_attempts,
    compaction_checkpoint_payload, compaction_complete_line, compaction_complete_payload,
    compaction_failed_line, compaction_phase_line, compaction_summary_delta_line,
    summarize_codex_compaction_context,
};
use crate::db::capabilities::effective_mcp_servers;
use crate::db::mcp::list_mcp_configs;
use crate::db::memory_store::{self, Memory, MemoryDb};
use crate::db::sessions::{
    UpsertSession, get_session as get_session_meta, get_session_input_tokens,
    has_visible_session_messages, mark_session_checkpoint, persist_assistant_snapshot,
    persist_compaction_checkpoint, persist_session_message, save_compaction_summary,
    set_session_routing, update_session_continuity, upsert_session,
};
use crate::model_routing::resolve_model_route;
use crate::routes::claude_auth::try_refresh_credentials;
use crate::workflow_events::workflow_event_line;

mod compaction;
mod content;
pub(crate) mod continuation;
mod http;
mod inputs;
mod lifecycle;
mod prompt;
mod reasoning;
mod session;
mod tool_results;

use compaction::*;
use content::*;
use prompt::*;
use session::*;
use tool_results::{tool_result_is_terminal, tool_result_message};

pub use http::{
    BackgroundChatTask, handle_chat, handle_update_queued_message, launch_background_chat_task,
    read_claude_mcp_servers, submit_durable_chat_run,
};
pub use inputs::{handle_inputs, handle_steer};
pub(crate) use lifecycle::start_dispatcher;
pub use prompt::build_system_prompt;
pub(crate) use reasoning::queue_if_needed as queue_auto_reasoning_handoff;
pub use session::system_prompt_with_codex_startup_memory_reminder;
pub(crate) use session::{codex_approval_prompt, codex_approval_request_key};

#[cfg(test)]
use http::chat_access_root;
#[cfg(test)]
mod tests;
