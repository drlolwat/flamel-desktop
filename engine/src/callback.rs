//! Authenticated framed IPC for engine-owned MCP callbacks; never HTTP over loopback.
use crate::AppState;
use axum::body::Body;
use flamel_protocol::{CHUNK_BYTES, MAX_BODY_BYTES};
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    sync::Arc,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tower::ServiceExt;
const MAX_ENVELOPE: usize = 1024 * 1024;
#[derive(Serialize, Deserialize)]
struct Call {
    version: u16,
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Option<serde_json::Value>,
}
#[derive(Serialize, Deserialize)]
struct Head {
    status: u16,
}

pub fn endpoint(state: &AppState) -> String {
    state
        .callback_endpoint
        .get()
        .cloned()
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", state.port))
}
pub async fn start(state: Arc<AppState>, user_id: String) -> Result<(), String> {
    if state.callback_endpoint.get().is_some() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("flamel-ipc-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| e.to_string())?;
        let path = dir.join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&path).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        let address = format!("ipc:{}", path.display());
        state
            .callback_endpoint
            .set(address.clone())
            .map_err(|_| "Broker already started")?;
        crate::cli_mcp::write_local_cli_runtime_for(&address, &state.mcp_token, &user_id)?;
        tokio::spawn(async move {
            let mut shutdown = state.shutdown_receiver();
            let permits = Arc::new(tokio::sync::Semaphore::new(64));
            loop {
                tokio::select! {
                    _=shutdown.changed()=>break,
                    accepted=listener.accept()=>{
                        let Ok((socket,_))=accepted else{break;};
                        // The private directory and peer UID both constrain local callers.
                        if socket.peer_cred().map_or(true,|cred|cred.uid()!=unsafe{libc::geteuid()}){continue;}
                        let Ok(permit)=permits.clone().try_acquire_owned() else{continue;};
                        let state=state.clone();let user=user_id.clone();
                        tokio::spawn(async move {let _permit=permit;let _=serve(socket,state,user).await;});
                    }
                }
            }
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_dir(dir);
        });
    }
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = format!(r"\\.\pipe\flamel-{}", uuid::Uuid::new_v4());
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .map_err(|e| e.to_string())?;
        let address = format!("ipc:{name}");
        state
            .callback_endpoint
            .set(address.clone())
            .map_err(|_| "Broker already started")?;
        crate::cli_mcp::write_local_cli_runtime_for(&address, &state.mcp_token, &user_id)?;
        tokio::spawn(async move {
            let mut shutdown = state.shutdown_receiver();
            let permits = Arc::new(tokio::sync::Semaphore::new(64));
            loop {
                tokio::select! {_=shutdown.changed()=>break,result=server.connect()=>if result.is_err(){break;}}
                let Ok(next) = ServerOptions::new()
                    .reject_remote_clients(true)
                    .create(&name)
                else {
                    break;
                };
                let connected = std::mem::replace(&mut server, next);
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    continue;
                };
                let state = state.clone();
                let user = user_id.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ = serve(connected, state, user).await;
                });
            }
        });
    }
    Ok(())
}
async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    mut socket: S,
    state: Arc<AppState>,
    user: String,
) -> Result<(), String> {
    let call = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let size = socket.read_u32().await.map_err(|e| e.to_string())? as usize;
        if size > MAX_ENVELOPE {
            return Err("Callback envelope too large".into());
        }
        let mut data = vec![0; size];
        socket
            .read_exact(&mut data)
            .await
            .map_err(|e| e.to_string())?;
        serde_json::from_slice::<Call>(&data).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "Callback handshake timed out")??;
    if call.version != flamel_protocol::VERSION
        || !call.path.starts_with("/api/")
        || call.path.contains("..")
    {
        return Err("Invalid callback operation".into());
    }
    let token = call
        .headers
        .get("x-flamel-token")
        .ok_or("Callback credential required")?;
    let session = if token == &state.mcp_token {
        let session = call.headers.get("x-flamel-session-id").cloned();
        if let Some(id) = &session {
            if crate::db::sessions::get_session(&state.db, id)
                .and_then(|s| s.user_id)
                .as_deref()
                != Some(&user)
            {
                return Err("Callback session does not belong to the selected profile".into());
            }
        }
        session
    } else {
        let claims = crate::mcp_auth::validate(&state, token).map_err(str::to_string)?;
        if claims.sub != user {
            return Err("Callback credential belongs to another profile".into());
        }
        crate::mcp_auth::context_session(
            &state,
            &claims,
            call.headers.get("x-flamel-session-id").map(String::as_str),
        )
        .map_err(str::to_string)?
    };
    let mut request = axum::http::Request::builder()
        .method(call.method.as_str())
        .uri(&call.path)
        .header("content-type", "application/json")
        .header("x-flamel-user-id", &user);
    if let Some(id) = &session {
        request = request.header("x-flamel-session-id", id);
    }
    let mut request = request
        .body(Body::from(
            call.body.map(|v| v.to_string()).unwrap_or_default(),
        ))
        .map_err(|e| e.to_string())?;
    request
        .extensions_mut()
        .insert(crate::mcp_auth::AuthenticatedMcpContext {
            user_id: user,
            session_id: session,
        });
    let app = crate::routes::build_router(state.clone());
    let response = app.oneshot(request).await.map_err(|e| e.to_string())?;
    let head = serde_json::to_vec(&Head {
        status: response.status().as_u16(),
    })
    .map_err(|e| e.to_string())?;
    socket
        .write_u32(head.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    socket.write_all(&head).await.map_err(|e| e.to_string())?;
    let mut body = response.into_body();
    let mut shutdown = state.shutdown_receiver();
    loop {
        let frame = tokio::select! {_=shutdown.changed()=>break,frame=body.frame()=>frame};
        let Some(frame) = frame else {
            break;
        };
        if let Ok(data) = frame.map_err(|e| e.to_string())?.into_data() {
            for chunk in data.chunks(CHUNK_BYTES) {
                tokio::time::timeout(std::time::Duration::from_secs(60), async {
                    socket.write_u32(chunk.len() as u32).await?;
                    socket.write_all(chunk).await
                })
                .await
                .map_err(|_| "Callback consumer stalled")?
                .map_err(|e| e.to_string())?;
            }
        }
    }
    socket.write_u32(0).await.map_err(|e| e.to_string())
}

trait ReadWrite: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWrite for T {}
pub struct Response {
    pub status: u16,
    stream: Box<dyn ReadWrite>,
    remaining: usize,
    ended: bool,
}
impl Read for Response {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() || self.ended {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut length = [0; 4];
            self.stream.read_exact(&mut length)?;
            self.remaining = u32::from_be_bytes(length) as usize;
            if self.remaining == 0 {
                self.ended = true;
                return Ok(0);
            }
            if self.remaining > CHUNK_BYTES {
                return Err(std::io::Error::other("Invalid callback frame"));
            }
        }
        let n = output.len().min(self.remaining);
        let read = self.stream.read(&mut output[..n])?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Truncated callback frame",
            ));
        }
        self.remaining -= read;
        Ok(read)
    }
}
pub fn request(
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
    base: &str,
    token: &str,
    user: &str,
) -> Result<Response, String> {
    let path_socket = base
        .strip_prefix("ipc:")
        .ok_or("Not a native callback endpoint")?;
    #[cfg(unix)]
    let mut stream: Box<dyn ReadWrite> = {
        let socket =
            std::os::unix::net::UnixStream::connect(path_socket).map_err(|e| e.to_string())?;
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(900)))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(std::time::Duration::from_secs(10)))
            .map_err(|e| e.to_string())?;
        Box::new(socket)
    };
    #[cfg(windows)]
    let mut stream: Box<dyn ReadWrite> = Box::new(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path_socket)
            .map_err(|e| e.to_string())?,
    );
    let mut headers = BTreeMap::from([
        ("x-flamel-token".into(), token.into()),
        ("x-flamel-user-id".into(), user.into()),
    ]);
    for (var, header) in [
        ("FLAMEL_SESSION_ID", "x-flamel-session-id"),
        ("FLAMEL_ASSISTANT_ID", "x-flamel-assistant-id"),
        ("FLAMEL_DELEGATION_TASK_ID", "x-flamel-delegation-id"),
    ] {
        if let Ok(value) = std::env::var(var) {
            headers.insert(header.into(), value);
        }
    }
    let data = serde_json::to_vec(&Call {
        version: flamel_protocol::VERSION,
        method: method.into(),
        path: path.into(),
        headers,
        body: body.cloned(),
    })
    .map_err(|e| e.to_string())?;
    if data.len() > MAX_ENVELOPE {
        return Err("Callback envelope too large".into());
    }
    stream
        .write_all(&(data.len() as u32).to_be_bytes())
        .and_then(|_| stream.write_all(&data))
        .map_err(|e| e.to_string())?;
    let mut length = [0; 4];
    stream.read_exact(&mut length).map_err(|e| e.to_string())?;
    let size = u32::from_be_bytes(length) as usize;
    if size > 4096 {
        return Err("Invalid callback header".into());
    }
    let mut head = vec![0; size];
    stream.read_exact(&mut head).map_err(|e| e.to_string())?;
    let head: Head = serde_json::from_slice(&head).map_err(|e| e.to_string())?;
    Ok(Response {
        status: head.status,
        stream,
        remaining: 0,
        ended: false,
    })
}
pub fn call(
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
    base: &str,
    token: &str,
    user: &str,
) -> Result<serde_json::Value, String> {
    let response = request(method, path, body, base, token, user)?;
    let status = response.status;
    let mut data = Vec::new();
    response
        .take((MAX_BODY_BYTES + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > MAX_BODY_BYTES {
        return Err("Callback response exceeds limit".into());
    }
    if !(200..300).contains(&status) {
        return Err(format!(
            "Callback failed ({status}): {}",
            String::from_utf8_lossy(&data)
        ));
    }
    if data.is_empty() {
        return Ok(serde_json::json!({"ok":true}));
    }
    serde_json::from_slice(&data).map_err(|e| e.to_string())
}
