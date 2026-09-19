//! Keep this in its own test executable: the first remote MCP request must work
//! before any other library has installed a process-wide TLS provider.
use axum::{Json, Router, http::{HeaderMap, StatusCode}, response::{IntoResponse, Response}, routing::post};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

async fn fixture(headers: HeaderMap, Json(request): Json<Value>) -> Response {
    if headers.get("authorization").and_then(|value| value.to_str().ok()) != Some("Bearer fixture-only") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(id) = request.get("id") else { return StatusCode::ACCEPTED.into_response(); };
    let result = match request["method"].as_str().unwrap_or("") {
        "initialize" => json!({"protocolVersion":request["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}),
        "tools/list" => json!({"tools":[{"name":"echo","description":"Fixture","inputSchema":{"type":"object"}}]}),
        "resources/list" => json!({"resources":[]}),
        "prompts/list" => json!({"prompts":[]}),
        "tools/call" => json!({"content":[{"type":"text","text":request["params"]["arguments"]["text"]}]}),
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response()
}

#[tokio::test]
async fn cold_start_remote_mcp_discovers_and_calls_with_authentication() {
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, Router::new().route("/mcp", post(fixture))).await.unwrap(); });
    let cancel = CancellationToken::new();
    let args = json!({"text":"remote-tool-proof"});
    let result = flamel::capability_client::run_remote(&url, Some(("echo", &args)), Some("fixture-only"), &cancel).await.unwrap();
    assert_eq!(result.tools.len(), 1);
    assert_eq!(result.output.unwrap()["content"][0]["text"], "remote-tool-proof");
    assert!(flamel::capability_client::run_remote(&url, None, Some("wrong-fixture-token"), &cancel).await.is_err());
    server.abort();
}
