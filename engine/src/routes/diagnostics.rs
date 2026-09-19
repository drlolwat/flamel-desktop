use axum::{
    Json,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc};

use crate::routes::admin::require_admin;
use crate::{AppState, runtime_logs};

pub async fn handle_update_readiness(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if !is_local_update_request(peer, &headers) {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})));
    }
    let active_sessions = state.active_sessions.lock().await.len();
    (
        StatusCode::OK,
        Json(update_readiness_payload(active_sessions)),
    )
}

fn update_readiness_payload(active_sessions: usize) -> Value {
    json!({
        "ready": active_sessions == 0,
        "activeChatSessions": active_sessions,
        "durableChatRecovery": true,
        "recoverableChatSessions": active_sessions,
        "safeForManagedRestart": true,
    })
}

fn is_local_update_request(peer: SocketAddr, headers: &HeaderMap) -> bool {
    peer.ip().is_loopback()
        && [
            "forwarded",
            "x-forwarded-for",
            "x-real-ip",
            "cf-connecting-ip",
        ]
        .into_iter()
        .all(|name| !headers.contains_key(name))
}

#[derive(Debug, Deserialize)]
pub struct RuntimeLogsQuery {
    limit: Option<usize>,
    #[serde(rename = "minLevel", alias = "min_level")]
    min_level: Option<String>,
}

pub async fn handle_runtime_logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RuntimeLogsQuery>,
) -> (StatusCode, Json<Value>) {
    if let Err(response) = require_admin(&state, &headers) {
        return response;
    }

    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let logs = runtime_logs::recent_logs(limit, query.min_level.as_deref());
    let counts_by_level = runtime_logs::counts_by_level();

    (
        StatusCode::OK,
        Json(json!({
            "logs": logs,
            "countsByLevel": counts_by_level,
            "limit": limit,
            "minLevel": query.min_level,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::{is_local_update_request, update_readiness_payload};
    use axum::http::{HeaderMap, HeaderValue};
    use std::net::SocketAddr;

    #[test]
    fn update_readiness_is_loopback_only() {
        let headers = HeaderMap::new();
        assert!(is_local_update_request(
            "127.0.0.1:8181".parse::<SocketAddr>().unwrap(),
            &headers,
        ));
        assert!(is_local_update_request(
            "[::1]:8181".parse::<SocketAddr>().unwrap(),
            &headers,
        ));
        assert!(!is_local_update_request(
            "192.0.2.10:8181".parse::<SocketAddr>().unwrap(),
            &headers,
        ));

        let mut proxied = HeaderMap::new();
        proxied.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.1"));
        assert!(!is_local_update_request(
            "127.0.0.1:8181".parse::<SocketAddr>().unwrap(),
            &proxied,
        ));
    }

    #[test]
    fn update_readiness_advertises_durable_chat_recovery() {
        let idle = update_readiness_payload(0);
        assert_eq!(idle["ready"], true);
        assert_eq!(idle["safeForManagedRestart"], true);
        assert_eq!(idle["durableChatRecovery"], true);
        assert_eq!(idle["recoverableChatSessions"], 0);

        let busy = update_readiness_payload(3);
        assert_eq!(busy["ready"], false);
        assert_eq!(busy["activeChatSessions"], 3);
        assert_eq!(busy["recoverableChatSessions"], 3);
        assert_eq!(busy["safeForManagedRestart"], true);
    }
}
