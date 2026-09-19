/// POST /api/btw
///
/// Forks a temporary one-shot Claude process using the session's existing context
/// (via --resume <claude_session_id>) and answers a side question without interrupting
/// the main session. The forked process has no tools and is killed after one turn.
use axum::{
    Json,
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::AppState;
use crate::claude::spawn::{build_args, build_env, find_claude_cli};
use crate::routes::claude_auth::try_refresh_credentials;

pub async fn handle_btw(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let question = match body.get("question").and_then(|v| v.as_str()) {
        Some(q) if !q.trim().is_empty() => q.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "question required"})),
            )
                .into_response();
        }
    };
    let session_id = body
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let working_directory = body
        .get("workingDirectory")
        .and_then(|v| v.as_str())
        .unwrap_or(".")
        .to_string();

    // Auth
    let user_id = crate::routes::admin::get_current_user_id(&state, &headers);
    let claude_creds = if let Some(ref uid) = user_id {
        try_refresh_credentials(&state, uid).await
    } else {
        None
    };

    // Resolve claude_session_id for context resumption (DB is authoritative)
    let claude_session_id = session_id.as_deref().and_then(|sid| {
        crate::db::sessions::get_session(&state.db, sid).and_then(|s| s.claude_session_id)
    });

    // Build args: 1 turn, no tools (empty allowed-tools), resume context
    let mut args = build_args(
        None, // use default model
        "default",
        Some(
            "You are answering a quick side question. Keep your answer focused and brief. Do not use any tools.",
        ),
        claude_session_id.as_deref(),
        None, // no MCP servers
    );
    // Add max-turns 1 to ensure single-turn behavior
    args.push("--max-turns".to_string());
    args.push("1".to_string());

    let cli = find_claude_cli();
    let env_vars = build_env(claude_creds.as_ref(), Some(&state.db), "");

    let mut cmd = Command::new(&cli);
    crate::mcp_parent_guard::configure_tokio_child(&mut cmd);
    cmd.args(&args)
        .current_dir(&working_directory)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .envs(env_vars);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("btw: failed to spawn Claude: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "failed to spawn claude"})),
            )
                .into_response();
        }
    };

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // Write question as stream-json user message
    let stdin_msg = json!({
        "type": "user",
        "message": {"role": "user", "content": question}
    });
    let mut msg_bytes = serde_json::to_vec(&stdin_msg).unwrap_or_default();
    msg_bytes.push(b'\n');
    let _ = stdin.write_all(&msg_bytes).await;
    let _ = stdin.flush().await;
    drop(stdin); // EOF signals end of input

    // Stream text content back as SSE
    let stream = async_stream::stream! {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }

            let Ok(msg) = serde_json::from_str::<Value>(trimmed) else { continue };
            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

            match msg_type {
                "stream_event" => {
                    if let Some(event) = msg.get("event") {
                        let et = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if et == "content_block_delta" {
                            let text = event
                                .get("delta")
                                .and_then(|d| d.get("text"))
                                .and_then(|t| t.as_str())
                                .unwrap_or("");
                            if !text.is_empty() {
                                yield Ok::<_, std::convert::Infallible>(
                                    format!("data: {}\n\n", json!({"type": "text", "text": text}))
                                );
                            }
                        }
                    }
                }
                "result" => {
                    yield Ok::<_, std::convert::Infallible>(
                        format!("data: {}\n\n", json!({"type": "done"}))
                    );
                    break;
                }
                _ => {}
            }
        }

        // Clean up
        let _ = child.kill().await;
    };

    Response::builder()
        .status(200)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
