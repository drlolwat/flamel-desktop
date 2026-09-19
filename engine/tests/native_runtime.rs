//! Runs with isolated profile/vault paths; never touches the developer's live database.
use axum::body::Body;
use flamel::{AppState, native::Runtime};
use flamel_protocol::{Request, VERSION};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
fn operation(method: &str, path: &str) -> Request {
    Request {
        version: VERSION,
        method: method.into(),
        path: path.into(),
        headers: Default::default(),
    }
}
async fn json_response(runtime: &Runtime, method: &str, path: &str, body: Value) -> (u16, Value) {
    let mut request = operation(method, path);
    request
        .headers
        .insert("content-type".into(), "application/json".into());
    let response = runtime
        .request(request, Body::from(body.to_string()))
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn main() {
    let root = std::env::temp_dir().join(format!("flamel-native-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    // Set environment before the runtime creates any threads.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
        std::env::set_var("FLAMEL_HOME", &root);
        std::env::set_var("FLAMEL_LOCAL_HOME", &root);
        std::env::set_var("FLAMEL_VAULT_KEY_PATH", root.join("vault.key"));
        std::env::set_var("FLAMEL_PROVIDER_AUTO_UPDATE", "0");
        std::env::set_var("FLAMEL_CATALOG_REFRESH", "0");
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let state = AppState::new_with_mode(root.join("flamel.db").to_str().unwrap(), 0, true);
        let native = Runtime::open(state.clone(), None).await.unwrap();
        assert_eq!(native.user_id(), flamel::FLAMEL_LOCAL_USER_ID);
        assert_eq!(state.port, 0);
        let mut req = operation("GET", "/api/me");
        req.headers
            .insert("x-flamel-user-id".into(), "attacker".into());
        req.headers
            .insert("authorization".into(), "Bearer attacker".into());
        let response = native.request(req, Body::empty()).await.unwrap();
        assert!(response.status().is_success());
        let me: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(me["id"], flamel::FLAMEL_LOCAL_USER_ID);
        assert!(
            native
                .request(
                    operation("GET", "/api/internal/git-credentials"),
                    Body::empty()
                )
                .await
                .is_err()
        );
        let endpoint = flamel::callback::endpoint(&state);
        assert!(endpoint.starts_with("ipc:"));
        let token = state.mcp_token.clone();
        let callback_endpoint = endpoint.clone();
        let callback = tokio::task::spawn_blocking(move || {
            flamel::callback::call(
                "GET",
                "/api/me",
                None,
                &callback_endpoint,
                &token,
                "attacker",
            )
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            callback["id"],
            flamel::FLAMEL_LOCAL_USER_ID,
            "coordinator cannot choose another identity"
        );
        let callback_endpoint = endpoint.clone();
        assert!(
            tokio::task::spawn_blocking(move || flamel::callback::call(
                "GET",
                "/api/me",
                None,
                &callback_endpoint,
                "wrong token",
                "attacker"
            ))
            .await
            .unwrap()
            .is_err()
        );
        let mut socket = native.socket("/api/realtime").unwrap();
        socket
            .send(axum::extract::ws::Message::Text(
                json!({"type":"ping"}).to_string().into(),
            ))
            .await
            .unwrap();
        let message = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(message,axum::extract::ws::Message::Text(text) if text.contains("pong")));
        let terminal_command = if cfg!(windows) {
            "[Console]::Write('native-terminal-ok')"
        } else {
            "printf native-terminal-ok"
        };
        let (status, terminal) = json_response(
            &native,
            "POST",
            "/api/terminal/start",
            json!({"command":terminal_command,"cwd":root.to_string_lossy()}),
        )
        .await;
        assert!(status < 300, "terminal startup failed: {status} {terminal}");
        let id = terminal["session_id"].as_str().unwrap();
        let mut terminal_socket = native
            .socket(&format!("/api/terminal/connect/{id}"))
            .unwrap();
        let output = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut output = String::new();
            while let Some(Ok(message)) = terminal_socket.next().await {
                if let axum::extract::ws::Message::Text(text) = message {
                    output.push_str(&text);
                    if text.contains("\"type\":\"exit\"") {
                        break;
                    }
                }
            }
            output
        })
        .await
        .unwrap();
        assert!(output.contains("bmF0aXZlLXRlcm1pbmFsLW9r"), "{output}");
        #[cfg(target_os = "linux")]
        {
            let sockets = std::fs::read_dir("/proc/self/fd")
                .unwrap()
                .filter_map(Result::ok)
                .filter_map(|entry| std::fs::read_link(entry.path()).ok())
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
                for line in std::fs::read_to_string(file).unwrap().lines().skip(1) {
                    let fields = line.split_whitespace().collect::<Vec<_>>();
                    if fields.len() > 9 && fields[3] == "0A" {
                        assert!(
                            !sockets.contains(&format!("socket:[{}]", fields[9])),
                            "Native engine opened TCP listener: {line}"
                        );
                    }
                }
            }
        }
        state.shutdown_processes().await;
        assert!(
            native
                .request(operation("GET", "/api/me"), Body::empty())
                .await
                .is_err()
        );
        drop(terminal_socket);
        drop(socket);
    });
    runtime.shutdown_timeout(std::time::Duration::from_secs(3));
    std::fs::remove_dir_all(root).unwrap();
    println!(
        "native runtime: profile, forged identity, callback auth, realtime, terminal, no TCP listener, shutdown passed"
    );
}
