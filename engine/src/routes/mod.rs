pub mod abort;
pub mod admin;
pub mod agents;
pub mod artifacts;
pub mod ask;
pub mod auth;
pub mod btw;
pub mod capabilities;
pub mod chat;
pub mod claude_auth;
pub mod claude_status;
pub mod codex_auth;
pub mod console;
pub mod diagnostics;
pub mod file_content;
pub mod files;
pub mod flamel_git;
pub mod fs;
pub mod git;
pub mod git_accounts;
pub mod git_integration;
pub mod intelligence;
pub mod mcp;
pub mod memories;
pub mod nested_events;
pub mod path_guard;
pub mod permission;
pub mod plans;
pub mod projects;
pub mod prompts;
pub mod providers;
pub mod realtime;
pub mod runtime_catalog;
pub mod sessions;
pub mod settings;
pub mod terminal;
pub mod todos;
pub mod usage;
pub mod work;

use axum::{
    Router,
    http::StatusCode,
    response::Html,
    routing::{delete, get, post, put},
};
use std::sync::Arc;

use crate::AppState;

/// Simple completion page shown in the external browser after SSO.
/// The Tauri app polls /api/auth/pending to claim the session.

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Setup / health check
        .route("/api/setup", get(auth::handle_setup_required))
        .route(
            "/api/system/update-readiness",
            get(diagnostics::handle_update_readiness),
        )
        // Current user
        .route("/api/me", get(admin::handle_get_me))
        .route("/api/me/settings", get(settings::handle_get_settings))
        .route("/api/me/settings", put(settings::handle_update_settings))
        .route(
            "/api/me/settings/compaction",
            get(settings::handle_get_compaction_settings),
        )
        .route(
            "/api/me/settings/compaction",
            put(settings::handle_update_compaction_settings),
        )
        .route(
            "/api/internal/local-secret",
            post(settings::handle_local_secret),
        )
        // Claude auth (OAuth for Max subscription)
        .route("/api/claude/status", get(claude_status::handle_get_status))
        .route(
            "/api/claude/apikey",
            post(claude_status::handle_set_api_key),
        )
        .route(
            "/api/claude/apikey",
            delete(claude_status::handle_delete_api_key),
        )
        .route(
            "/api/me/claude-auth",
            get(claude_auth::handle_get_auth_status),
        )
        .route(
            "/api/me/claude-auth",
            delete(claude_auth::handle_revoke_auth),
        )
        .route(
            "/api/me/claude-auth/refresh",
            post(claude_auth::handle_refresh_auth),
        )
        .route(
            "/api/me/claude-auth/oauth/start",
            get(claude_auth::handle_oauth_start),
        )
        .route(
            "/api/me/claude-auth/oauth/callback",
            get(claude_auth::handle_oauth_callback),
        )
        .route(
            "/api/me/claude-auth/oauth/complete",
            post(claude_auth::handle_oauth_complete),
        )
        // Codex auth (device code flow for ChatGPT / OpenAI account)
        .route(
            "/api/me/codex-auth",
            get(codex_auth::handle_get_auth_status),
        )
        .route("/api/me/codex-auth", delete(codex_auth::handle_revoke_auth))
        .route(
            "/api/me/codex-auth/refresh",
            post(codex_auth::handle_refresh_auth),
        )
        .route(
            "/api/me/codex-auth/device/start",
            post(codex_auth::handle_device_start),
        )
        .route(
            "/api/me/codex-auth/device/poll",
            post(codex_auth::handle_device_poll),
        )
        // Projects
        .route("/api/projects", get(projects::handle_get_projects))
        // Sessions
        .route("/api/sessions", get(sessions::handle_list_sessions))
        .route("/api/sessions/{id}", get(sessions::handle_get_session))
        .route(
            "/api/sessions/{id}/workspace/review",
            get(sessions::handle_workspace_review),
        )
        .route(
            "/api/sessions/{id}/bootstrap",
            get(sessions::handle_session_bootstrap),
        )
        .route(
            "/api/sessions/{id}/active-plan",
            put(sessions::handle_set_session_active_plan),
        )
        .route(
            "/api/sessions/{id}/history",
            get(sessions::handle_get_session_history),
        )
        .route(
            "/api/sessions/{id}/stream",
            get(sessions::handle_session_stream),
        )
        .route(
            "/api/sessions/{id}/kill",
            post(sessions::handle_kill_session),
        )
        .route(
            "/api/sessions/{id}/compact",
            post(sessions::handle_compact_session),
        )
        .route(
            "/api/sessions/{id}/fork",
            post(sessions::handle_fork_session),
        )
        .route("/api/sessions/{id}", put(sessions::handle_update_session))
        .route(
            "/api/sessions/{id}",
            delete(sessions::handle_delete_session),
        )
        .route(
            "/api/sessions/{id}/context-stats",
            get(sessions::handle_context_stats),
        )
        .route(
            "/api/sessions/{id}/context",
            get(sessions::handle_context_search),
        )
        // Chat
        .route("/api/chat", post(chat::handle_chat))
        .route("/api/chat/steer", post(chat::handle_steer))
        .route("/api/chat/{id}/inputs", get(chat::handle_inputs))
        .route(
            "/api/chat/queue/{requestId}",
            put(chat::handle_update_queued_message),
        )
        .route("/api/realtime/ticket", post(realtime::handle_ticket))
        .route("/api/realtime", get(realtime::handle_socket))
        .route("/api/abort/{requestId}", post(abort::handle_abort))
        .route(
            "/api/sessions/{id}/abort",
            post(abort::handle_abort_session),
        )
        .route(
            "/api/permission/{requestId}",
            post(permission::handle_permission),
        )
        .route("/api/ask", post(ask::handle_ask))
        .route("/api/ask/{requestId}", post(ask::handle_ask_answer))
        .route(
            "/api/ask/{requestId}/consume",
            post(ask::handle_ask_consume),
        )
        .route("/api/ask/{requestId}/cancel", post(ask::handle_ask_cancel))
        // Unified command event console streams
        .route(
            "/api/console/sessions/{id}/stream",
            get(console::handle_command_session_stream),
        )
        // Interactive terminal / PTY sessions
        .route("/api/terminal", get(terminal::handle_list))
        .route("/api/terminal/connect/{id}", get(terminal::handle_ws))
        .route("/api/terminal/start", post(terminal::handle_start))
        .route("/api/terminal/{id}", delete(terminal::handle_delete))
        .route("/api/terminal/{id}/input", post(terminal::handle_input))
        .route("/api/terminal/{id}/resize", post(terminal::handle_resize))
        .route("/api/terminal/{id}/output", get(terminal::handle_output))
        .route("/api/terminal/{id}/status", get(terminal::handle_status))
        .route("/api/terminal/{id}/stream", get(terminal::handle_stream))
        .route("/api/terminal/{id}/ws", get(terminal::handle_ws))
        // Nested agent events — flamel_mcp forwards sub-agent stream-json lines here
        .route(
            "/api/internal/agent-batch-status",
            post(nested_events::handle_batch_status),
        )
        .route(
            "/api/internal/nested-event",
            post(nested_events::handle_nested_event),
        )
        .route("/api/btw", post(btw::handle_btw))
        .route(
            "/api/sessions/{id}/truncate",
            post(sessions::handle_truncate),
        )
        // Prompts
        .route("/api/prompts", get(prompts::handle_list_prompts))
        .route("/api/prompts", post(prompts::handle_create_prompt))
        .route("/api/prompts/{id}", delete(prompts::handle_delete_prompt))
        .route("/api/prompts/{id}/use", post(prompts::handle_use_prompt))
        // Usage
        .route("/api/usage", get(usage::handle_get_usage))
        // Files
        .route("/api/files", get(files::handle_list_files))
        .route("/api/fs/dir", get(fs::handle_list_dir))
        .route("/api/fs/mkdir", post(fs::handle_mkdir))
        .route(
            "/api/file-content",
            get(file_content::handle_get_file_content),
        )
        .route("/api/file-asset", get(file_content::handle_get_file_asset))
        .route("/api/artifacts", get(artifacts::handle_list_artifacts))
        .route(
            "/api/mcp-artifacts/{name}",
            get(artifacts::handle_get_mcp_artifact),
        )
        .route("/api/artifacts/{id}", get(artifacts::handle_get_artifact))
        .route(
            "/api/artifacts/{id}/content",
            get(artifacts::handle_get_artifact_content),
        )
        .route(
            "/api/artifacts/{id}/save",
            post(artifacts::handle_save_artifact),
        )
        .route(
            "/api/file-content",
            put(file_content::handle_save_file_content),
        )
        // Git
        .route("/api/git/status", get(git::handle_git_status))
        .route("/api/git/diff", get(git::handle_git_diff))
        .route(
            "/api/git/structured-diff",
            get(git::handle_git_structured_diff),
        )
        .route("/api/git/history", get(git::handle_git_history))
        .route("/api/git/branches", get(git::handle_git_branches))
        .route("/api/git/file", get(git::handle_git_file))
        .route("/api/git/operation", post(git::handle_git_operation))
        // Git accounts
        .route(
            "/api/git-accounts",
            get(git_accounts::handle_list_git_accounts),
        )
        .route(
            "/api/git-accounts",
            post(git_accounts::handle_create_git_account),
        )
        .route(
            "/api/git-accounts/{id}",
            delete(git_accounts::handle_delete_git_account),
        )
        .route(
            "/api/git-accounts/{id}/repos",
            get(git_accounts::handle_list_git_account_repos),
        )
        .route(
            "/api/internal/git-credentials",
            get(git_accounts::handle_list_git_accounts_internal),
        )
        .route(
            "/api/flamel-git/repositories",
            get(flamel_git::handle_list_repositories),
        )
        .route("/api/flamel-git/account", get(flamel_git::handle_account))
        .route(
            "/api/flamel-git/token",
            post(flamel_git::handle_repository_token),
        )
        .route(
            "/api/internal/flamel-git/token",
            post(flamel_git::handle_repository_token),
        )
        .route(
            "/api/internal/flamel-git/credentials",
            post(flamel_git::handle_remote_credentials),
        )
        .route(
            "/api/internal/flamel-git/repositories",
            get(flamel_git::handle_list_repositories).post(flamel_git::handle_repository_create),
        )
        .route(
            "/api/internal/flamel-git/operation",
            post(flamel_git::handle_repository_operation),
        )
        .route(
            "/api/internal/flamel-git/account/tokens",
            get(flamel_git::handle_account_tokens_list)
                .post(flamel_git::handle_account_token_create),
        )
        .route(
            "/api/internal/flamel-git/account/tokens/{id}",
            delete(flamel_git::handle_account_token_delete),
        )
        // Plans
        .route("/api/plans", get(plans::handle_list_plans))
        .route("/api/plans", post(plans::handle_create_plan))
        .route("/api/plans/{id}", get(plans::handle_get_plan))
        .route("/api/plans/{id}", put(plans::handle_update_plan))
        .route(
            "/api/plans/{id}/execution",
            put(plans::handle_update_plan_execution),
        )
        .route("/api/plans/{id}", delete(plans::handle_delete_plan))
        .route(
            "/api/plans/{planId}/items/{itemId}",
            put(plans::handle_upsert_plan_item),
        )
        .route(
            "/api/plans/{planId}/items/{itemId}",
            delete(plans::handle_delete_plan_item),
        )
        .route(
            "/api/plans/{planId}/items",
            post(plans::handle_add_plan_items),
        )
        .route(
            "/api/plans/{planId}/items",
            put(plans::handle_replace_plan_items),
        )
        // MCP
        .route(
            "/api/sessions/{id}/mcp-status",
            get(mcp::handle_session_mcp_status),
        )
        .route(
            "/api/sessions/{id}/mcp-reconnect",
            post(mcp::handle_session_mcp_reconnect),
        )
        .route("/api/mcp", get(mcp::handle_list_mcp))
        .route("/api/mcp", post(mcp::handle_create_mcp))
        .route("/api/mcp/{id}", put(mcp::handle_update_mcp))
        .route("/api/mcp/{id}", delete(mcp::handle_delete_mcp))
        // Capability Hub
        .route(
            "/api/capabilities/catalog",
            get(capabilities::handle_list_catalog),
        )
        .route(
            "/api/capabilities/catalog",
            post(capabilities::handle_create_catalog_entry),
        )
        .route(
            "/api/capabilities/{id}/connect",
            post(capabilities::handle_connect),
        )
        .route(
            "/api/capabilities/{id}/connect",
            delete(capabilities::handle_disconnect),
        )
        .route(
            "/api/capabilities/{id}/test",
            post(capabilities::handle_test_connection),
        )
        .route(
            "/api/capabilities/{id}/tools",
            get(capabilities::handle_list_tools),
        )
        .route(
            "/api/capabilities/{id}/surfaces",
            get(capabilities::handle_list_surfaces),
        )
        .route(
            "/api/capabilities/{id}/tools/{tool}",
            put(capabilities::handle_set_tool),
        )
        .route(
            "/api/capabilities/{id}/execute",
            post(capabilities::handle_execute),
        )
        .route(
            "/api/capabilities/metrics",
            get(capabilities::handle_metrics),
        )
        .route(
            "/api/capability-tasks/{id}/cancel",
            post(capabilities::handle_cancel_task),
        )
        .route(
            "/api/capabilities/packs",
            get(capabilities::handle_list_packs).post(capabilities::handle_install_pack),
        )
        .route(
            "/api/capabilities/packs/preview",
            post(capabilities::handle_preview_pack),
        )
        .route(
            "/api/capabilities/packs/{id}/rollback",
            post(capabilities::handle_rollback_pack),
        )
        .route(
            "/api/capabilities/packs/{id}",
            delete(capabilities::handle_uninstall_pack),
        )
        .route(
            "/api/capabilities/marketplace",
            get(capabilities::handle_marketplace),
        )
        .route(
            "/api/capabilities/marketplace/import",
            post(capabilities::handle_import_marketplace),
        )
        .route(
            "/api/openclaw/import/preview",
            post(capabilities::handle_preview_openclaw_import),
        )
        .route(
            "/api/capabilities/marketplace/{id}/install",
            post(capabilities::handle_install_marketplace),
        )
        .route(
            "/api/runtime-catalog/personas",
            get(runtime_catalog::handle_runtime_catalog_personas),
        )
        .route(
            "/api/admin/runtime-catalog",
            get(runtime_catalog::handle_runtime_catalog_status)
                .post(runtime_catalog::handle_runtime_catalog_control),
        )
        .route(
            "/api/capabilities/skills",
            get(capabilities::handle_skills).post(capabilities::handle_propose_skill),
        )
        .route(
            "/api/capabilities/skills/{id}",
            get(capabilities::handle_skill)
                .patch(capabilities::handle_set_skill_state)
                .delete(capabilities::handle_uninstall_skill),
        )
        .route(
            "/api/capabilities/skills/{id}/review",
            post(capabilities::handle_review_skill),
        )
        .route(
            "/api/capabilities/skills/{id}/versions",
            post(capabilities::handle_add_skill_version),
        )
        .route(
            "/api/capabilities/skills/{id}/install",
            post(capabilities::handle_install_skill),
        )
        .route(
            "/api/capabilities/skills/{id}/rollback",
            post(capabilities::handle_rollback_skill),
        )
        .route(
            "/api/capabilities/{id}/credential",
            post(capabilities::handle_set_credential)
                .put(capabilities::handle_set_credential)
                .delete(capabilities::handle_delete_credential),
        )
        .route("/api/capability-tasks", get(capabilities::handle_tasks))
        .route("/api/capability-audit", get(capabilities::handle_audit))
        .route("/api/capabilities/audit", get(capabilities::handle_audit))
        .route(
            "/api/capability-invocations",
            get(capabilities::handle_invocations),
        )
        .route(
            "/api/capability-policies",
            get(capabilities::handle_policies).post(capabilities::handle_create_policy),
        )
        .route(
            "/api/capability-policies/{id}",
            delete(capabilities::handle_delete_policy),
        )
        // Native Engineering Work Hub
        .route("/api/work-items", get(work::list).post(work::create))
        .route(
            "/api/work-items/{id}",
            get(work::get).put(work::update).delete(work::delete),
        )
        .route("/api/work-items/{id}/comments", post(work::comment))
        .route("/api/work-items/{id}/dependencies", post(work::dependency))
        .route("/api/work-items/{id}/artifacts", post(work::artifact))
        // Incremental project intelligence and validation graph
        .route("/api/project-intelligence/index", post(intelligence::index))
        .route(
            "/api/project-intelligence/context",
            post(intelligence::context),
        )
        .route(
            "/api/project-intelligence/verification",
            post(intelligence::verification),
        )
        .route(
            "/api/project-intelligence/verify",
            post(intelligence::run_verification),
        )
        .route("/api/memories", get(memories::list))
        .route(
            "/api/memories/{id}",
            put(memories::update).delete(memories::delete),
        )
        // Todos
        .route("/api/todos", get(todos::handle_list_todos))
        .route("/api/todos", post(todos::handle_create_todo))
        .route("/api/todos/{id}", put(todos::handle_update_todo))
        .route("/api/todos/{id}", delete(todos::handle_delete_todo))
        .route(
            "/api/todos/clear-done",
            delete(todos::handle_clear_done_todos),
        )
        // Agents
        .route("/api/agent-configs", get(agents::handle_list_agent_configs))
        .route(
            "/api/agent-configs",
            post(agents::handle_create_agent_config),
        )
        .route(
            "/api/agent-configs/{id}",
            put(agents::handle_update_agent_config),
        )
        .route(
            "/api/agent-configs/{id}",
            delete(agents::handle_delete_agent_config),
        )
        .route("/api/agent-runs", get(agents::handle_list_agent_runs))
        .route(
            "/api/agent-runs/{id}/approve",
            post(agents::handle_approve_agent_run),
        )
        .route(
            "/api/agent-runs/{id}/reject",
            post(agents::handle_reject_agent_run),
        )
        .route(
            "/api/agent-runs/{id}/kill",
            post(agents::handle_kill_agent_run),
        )
        .route("/api/agent-runs/stream", get(agents::handle_agent_stream))
        // Git integration (issues & PRs)
        .route("/api/git/issues", get(git_integration::handle_list_issues))
        .route(
            "/api/git/issues/{number}",
            get(git_integration::handle_get_issue),
        )
        .route("/api/git/pulls", get(git_integration::handle_list_pulls))
        .route(
            "/api/git/pulls/{number}",
            get(git_integration::handle_get_pull),
        )
        .route("/api/git/pulls", post(git_integration::handle_create_pull))
        .route(
            "/api/git/branch",
            get(git_integration::handle_get_current_branch),
        )
        .route("/api/git/clone", post(git_integration::handle_clone_repo))
        // Providers
        .route("/api/providers", get(providers::handle_get_providers))
        .route(
            "/api/admin/diagnostics/runtime-logs",
            get(diagnostics::handle_runtime_logs),
        )
        .with_state(state)
}
