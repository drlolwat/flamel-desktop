//! Account-bound MCP credentials. The coordinator secret is never given to a
//! hosted provider or runner. This is an API credential, not a login JWT.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::AppState;

const PREFIX: &str = "flamel-mcp.v1.";
const AUDIENCE: &str = "flamel-mcp";
pub const TOKEN_LIFETIME_SECONDS: i64 = 48 * 60 * 60;
pub const REFRESH_AFTER_SECONDS: u64 = 24 * 60 * 60;

/// Middleware-only attestation for MCP runtimes whose IDs are not chat rows.
/// HTTP headers cannot supply this extension; the credential must validate first.
#[derive(Clone, Debug)]
pub struct AuthenticatedMcpContext {
    pub user_id: String,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpClaims {
    pub sub: String,
    pub session_id: Option<String>,
    aud: String,
    iat: i64,
    exp: i64,
}

pub fn is_scoped(token: &str) -> bool {
    token.starts_with(PREFIX)
}

/// Call only after resolving the owner and runtime/session on the coordinator.
/// Desktop keeps its existing local-only standalone CLI connection contract.
pub fn token_for_process(state: &AppState, user_id: &str, session_id: Option<&str>) -> String {
    if state.local_mode {
        return state.mcp_token.clone();
    }
    token_for_account(state, user_id, session_id)
}

/// Remote execution must be account-scoped even when dispatched by Desktop.
pub fn token_for_account(state: &AppState, user_id: &str, session_id: Option<&str>) -> String {
    issue(
        &state.mcp_token,
        user_id,
        session_id,
        chrono::Utc::now().timestamp(),
    )
}

fn issue(secret: &str, user_id: &str, session_id: Option<&str>, now: i64) -> String {
    sign(
        secret,
        &McpClaims {
            sub: user_id.into(),
            session_id: session_id.map(str::to_owned),
            aud: AUDIENCE.into(),
            iat: now,
            exp: now + TOKEN_LIFETIME_SECONDS,
        },
    )
}

fn sign(secret: &str, claims: &McpClaims) -> String {
    let payload =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("MCP claim serialization"));
    let mut signer =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    signer.update(PREFIX.as_bytes());
    signer.update(payload.as_bytes());
    format!(
        "{PREFIX}{payload}.{}",
        URL_SAFE_NO_PAD.encode(signer.finalize().into_bytes())
    )
}

pub fn validate(state: &AppState, token: &str) -> Result<McpClaims, &'static str> {
    validate_at(&state.mcp_token, token, chrono::Utc::now().timestamp())
}

fn validate_at(secret: &str, token: &str, now: i64) -> Result<McpClaims, &'static str> {
    if secret.is_empty() || token.len() > 8192 {
        return Err("Invalid MCP credential");
    }
    let (payload, signature) = token
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.split_once('.'))
        .ok_or("Invalid MCP credential")?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| "Invalid MCP signature")?;
    let mut verifier =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| "Invalid MCP verifier")?;
    verifier.update(PREFIX.as_bytes());
    verifier.update(payload.as_bytes());
    verifier
        .verify_slice(&signature)
        .map_err(|_| "Invalid MCP signature")?;
    let claims: McpClaims = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| "Invalid MCP payload")?,
    )
    .map_err(|_| "Invalid MCP payload")?;
    if claims.aud != AUDIENCE
        || claims.sub.trim().is_empty()
        || claims
            .session_id
            .as_deref()
            .is_some_and(|id| id.trim().is_empty())
    {
        return Err("Invalid MCP identity or audience");
    }
    if claims.exp <= now
        || claims.iat > now + 60
        || claims.exp <= claims.iat
        || claims.exp.saturating_sub(claims.iat) > TOKEN_LIFETIME_SECONDS
    {
        return Err(
            "MCP credential expired or has invalid lifetime; restart this provider session",
        );
    }
    Ok(claims)
}

/// Ignore caller-supplied account headers. A signed context is the issuer's
/// ownership attestation for agent/assistant runtimes without a chat row; any
/// persisted chat owner must still match (including ownership changes).
pub fn context_session(
    state: &AppState,
    claims: &McpClaims,
    supplied: Option<&str>,
) -> Result<Option<String>, &'static str> {
    if let (Some(bound), Some(supplied)) = (claims.session_id.as_deref(), supplied) {
        if bound != supplied {
            return Err("MCP session does not match credential");
        }
    }
    let session_id = claims.session_id.as_deref().or(supplied);
    if let Some(session_id) = session_id {
        match crate::db::sessions::get_session(&state.db, session_id) {
            Some(session) if session.user_id.as_deref() != Some(claims.sub.as_str()) => {
                return Err("MCP session belongs to another account");
            }
            None if claims.session_id.is_none() => {
                return Err("MCP session ownership could not be verified");
            }
            _ => {}
        }
    }
    Ok(session_id.map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_tampering_expiry_wrong_audience_and_wrong_signing_key() {
        let token = issue("coordinator-secret", "alice", Some("session-1"), 1000);
        assert_eq!(
            validate_at("coordinator-secret", &token, 1001).unwrap().sub,
            "alice"
        );
        assert!(validate_at("other-server", &token, 1001).is_err());
        assert!(validate_at("coordinator-secret", &token, 1000 + TOKEN_LIFETIME_SECONDS).is_err());
        let claims = McpClaims {
            sub: "alice".into(),
            session_id: None,
            aud: "login".into(),
            iat: 1000,
            exp: 2000,
        };
        assert!(
            validate_at(
                "coordinator-secret",
                &sign("coordinator-secret", &claims),
                1001
            )
            .is_err()
        );
        let changed = token.replacen("flamel-mcp.v1.", "flamel-mcp.v1.e", 1);
        assert!(validate_at("coordinator-secret", &changed, 1001).is_err());
        assert!(validate_at("", &token, 1001).is_err());
    }

    #[tokio::test]
    async fn coordinator_upgrade_invalidates_old_shared_tokens_and_stays_stable() {
        let state = AppState::new(":memory:", 0);
        let legacy = crate::db::Db::open(":memory:");
        legacy
            .conn()
            .execute(
                "INSERT INTO app_settings (key,value) VALUES ('mcp_token','old-shared-secret')",
                [],
            )
            .unwrap();
        let current =
            crate::db::settings::get_or_create_mcp_token(&legacy, &state.credential_vault).unwrap();
        assert_ne!(current, "old-shared-secret");
        assert_eq!(
            current,
            crate::db::settings::get_or_create_mcp_token(&legacy, &state.credential_vault).unwrap()
        );
        assert!(
            validate_at(
                &current,
                &issue("old-shared-secret", "alice", None, 1000),
                1001
            )
            .is_err()
        );
        let stored: String = legacy
            .conn()
            .query_row(
                "SELECT value FROM app_settings WHERE key='mcp_coordinator_token_v2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(stored, current);
    }
}
