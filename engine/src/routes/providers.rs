//! GET /api/providers — which AI providers are installed and authenticated.

use axum::{Json, extract::State, http::HeaderMap, response::IntoResponse};
use serde_json::json;
use std::sync::Arc;

use crate::AppState;
use crate::claude::provider::{ProviderKind, get_provider};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderAvailability {
    pub claude_available: bool,
    pub claude_authenticated: bool,
    pub codex_available: bool,
    pub codex_authenticated: bool,
}

pub fn provider_availability(state: &AppState, user_id: Option<&str>) -> ProviderAvailability {
    let claude_available = get_provider(ProviderKind::Claude).find_binary().is_ok();
    let claude_flamel_authenticated = user_id.is_some_and(|uid| {
        crate::db::claude_auth::get_credentials(&state.db, &state.credential_vault, uid).is_some()
    });
    let claude_local_cli_authenticated =
        state.local_mode && crate::routes::claude_auth::local_claude_credentials_present();
    let claude_env_authenticated = !state.local_mode
        && user_id.is_none()
        && std::env::var("ANTHROPIC_API_KEY").is_ok_and(|key| !key.is_empty());

    let codex_available = get_provider(ProviderKind::Codex).find_binary().is_ok();
    let codex_flamel_authenticated = user_id
        .is_some_and(|uid| crate::db::provider_auth::has_credentials(&state.db, uid, "codex"));
    let codex_local_cli_authenticated =
        state.local_mode && crate::routes::codex_auth::local_codex_credentials_present();

    ProviderAvailability {
        claude_available,
        claude_authenticated: claude_flamel_authenticated
            || claude_local_cli_authenticated
            || claude_env_authenticated,
        codex_available,
        codex_authenticated: codex_flamel_authenticated || codex_local_cli_authenticated,
    }
}

pub async fn handle_get_providers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user_id = crate::routes::admin::get_current_user_id(&state, &headers);

    // Claude: check binary + credentials (Claude uses its own claude_credentials table)
    let availability = provider_availability(&state, user_id.as_deref());
    let claude_available = availability.claude_available;
    let claude_flamel_authenticated = user_id.as_ref().is_some_and(|uid| {
        crate::db::claude_auth::get_credentials(&state.db, &state.credential_vault, uid).is_some()
    });
    let claude_local_cli_authenticated =
        state.local_mode && crate::routes::claude_auth::local_claude_credentials_present();
    let claude_env_authenticated = !state.local_mode
        && user_id.is_none()
        && std::env::var("ANTHROPIC_API_KEY").is_ok_and(|k| !k.is_empty());
    let claude_authenticated = availability.claude_authenticated;
    let claude_source = if claude_flamel_authenticated {
        Some("flamel")
    } else if claude_local_cli_authenticated {
        Some("local_cli")
    } else if claude_env_authenticated {
        Some("env")
    } else {
        None
    };

    // Codex: check binary + credentials
    let codex_available = availability.codex_available;
    let codex_flamel_authenticated = user_id
        .as_ref()
        .is_some_and(|uid| crate::db::provider_auth::has_credentials(&state.db, uid, "codex"));
    let codex_local_cli_authenticated =
        state.local_mode && crate::routes::codex_auth::local_codex_credentials_present();
    let codex_authenticated = availability.codex_authenticated;
    let codex_source = if codex_flamel_authenticated {
        Some("flamel")
    } else if codex_local_cli_authenticated {
        Some("local_cli")
    } else {
        None
    };

    Json(json!({
        "providers": [
            {
                "kind": "claude",
                "available": claude_available,
                "authenticated": claude_authenticated,
                "source": claude_source,
            },
            {
                "kind": "codex",
                "available": codex_available,
                "authenticated": codex_authenticated,
                "source": codex_source,
            },
        ]
    }))
}
