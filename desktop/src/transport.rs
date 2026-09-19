use axum::{
    body::{Body, Bytes},
    extract::ws::Message,
};
use flamel_protocol::{CHUNK_BYTES, MAX_BODY_BYTES, Request, ResponseHead};
use futures::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use std::{collections::HashMap, sync::Arc};
use tauri::State;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub struct Transfer {
    cancel: CancellationToken,
    upload: Mutex<Option<mpsc::Sender<Result<Bytes, std::io::Error>>>>,
    uploaded: Mutex<usize>,
    head: Mutex<Option<oneshot::Receiver<Result<ResponseHead, String>>>>,
    output: Mutex<mpsc::Receiver<Result<Vec<u8>, String>>>,
    socket: Mutex<Option<futures::stream::SplitSink<flamel::native_socket::Socket, Message>>>,
}
#[derive(Default, Clone)]
pub struct Transport {
    entries: Arc<Mutex<HashMap<String, Arc<Transfer>>>>,
}
impl Transport {
    async fn get(&self, id: &str) -> Result<Arc<Transfer>, String> {
        self.entries
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| "Unknown or cancelled transfer".into())
    }
    async fn insert(&self, id: String, value: Arc<Transfer>) -> Result<(), String> {
        let mut entries = self.entries.lock().await;
        if entries.len() >= 64 {
            return Err("Too many concurrent transfers".into());
        }
        entries.insert(id, value);
        Ok(())
    }
    pub async fn cancel_all(&self) {
        for (_, entry) in self.entries.lock().await.drain() {
            entry.cancel.cancel();
        }
    }
}

#[tauri::command]
pub async fn runtime_open(
    runtime: State<'_, crate::Host>,
    transport: State<'_, Transport>,
    request: Request,
) -> Result<String, String> {
    flamel::native::validate_request(&request)?;
    let runtime = runtime.target().await?;
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(8);
    let (out, output) = mpsc::channel(8);
    let (head_tx, head) = oneshot::channel();
    let cancel = CancellationToken::new();
    transport
        .insert(
            id.clone(),
            Arc::new(Transfer {
                cancel: cancel.clone(),
                upload: Mutex::new(Some(tx)),
                uploaded: Mutex::new(0),
                head: Mutex::new(Some(head)),
                output: Mutex::new(output),
                socket: Mutex::new(None),
            }),
        )
        .await?;
    let task_id = id.clone();
    tokio::spawn(async move {
        let body = Body::from_stream(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|data| (data, rx))
        }));
        let task = async move {
            let response = match runtime.request(request, body).await {
                Ok(response) => response,
                Err(error) => {
                    let _ = head_tx.send(Err(error));
                    return;
                }
            };
            let headers = response
                .headers()
                .iter()
                .filter(|(k, _)| !matches!(k.as_str(), "set-cookie" | "authorization"))
                .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_owned())))
                .collect();
            let _ = head_tx.send(Ok(ResponseHead {
                status: response.status().as_u16(),
                headers,
                stream_id: task_id,
            }));
            let mut body = response.into_body();
            while let Some(frame) = body.frame().await {
                match frame {
                    Ok(frame) => {
                        if let Ok(data) = frame.into_data() {
                            for chunk in data.chunks(CHUNK_BYTES) {
                                if tokio::time::timeout(
                                    std::time::Duration::from_secs(60),
                                    out.send(Ok(chunk.to_vec())),
                                )
                                .await
                                .is_err()
                                {
                                    return;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let _ = out.send(Err(error.to_string())).await;
                        return;
                    }
                }
            }
        };
        tokio::select! {_=cancel.cancelled()=>{},_=task=>{}}
    });
    Ok(id)
}
#[tauri::command]
pub async fn runtime_write(
    transport: State<'_, Transport>,
    id: String,
    chunk: Vec<u8>,
    done: bool,
) -> Result<(), String> {
    if chunk.len() > CHUNK_BYTES {
        return Err("Upload chunk exceeds limit".into());
    }
    let entry = transport.get(&id).await?;
    let mut count = entry.uploaded.lock().await;
    *count += chunk.len();
    if *count > MAX_BODY_BYTES {
        entry.cancel.cancel();
        return Err("Upload exceeds 50 MiB".into());
    }
    let mut upload = entry.upload.lock().await;
    let tx = upload.as_ref().ok_or("Upload already complete")?;
    if !chunk.is_empty() {
        tokio::select! { _=entry.cancel.cancelled()=>return Err("Cancelled".into()), result=tx.send(Ok(Bytes::from(chunk)))=>result.map_err(|_|"Request closed")? }
    }
    if done {
        upload.take();
    }
    Ok(())
}
#[tauri::command]
pub async fn runtime_head(
    transport: State<'_, Transport>,
    id: String,
) -> Result<ResponseHead, String> {
    let entry = transport.get(&id).await?;
    let head = entry
        .head
        .lock()
        .await
        .take()
        .ok_or("Headers already read")?;
    tokio::select! {_=entry.cancel.cancelled()=>Err("Cancelled".into()),result=head=>result.map_err(|_|"Request closed")?}
}
#[tauri::command]
pub async fn runtime_read(
    transport: State<'_, Transport>,
    id: String,
) -> Result<Option<Vec<u8>>, String> {
    let entry = transport.get(&id).await?;
    let mut rx = entry.output.lock().await;
    tokio::select! {_=entry.cancel.cancelled()=>Err("Cancelled".into()),result=rx.recv()=>result.transpose()}
}
#[tauri::command]
pub async fn runtime_cancel(transport: State<'_, Transport>, id: String) -> Result<(), String> {
    if let Some(entry) = transport.entries.lock().await.remove(&id) {
        entry.cancel.cancel();
    }
    Ok(())
}
#[tauri::command]
pub async fn runtime_socket_open(
    runtime: State<'_, crate::Host>,
    transport: State<'_, Transport>,
    path: String,
) -> Result<String, String> {
    let runtime = runtime.target().await?;
    let socket = runtime.socket(&path).await?;
    let (sink, mut source) = socket.split();
    let (out, output) = mpsc::channel(8);
    let cancel = CancellationToken::new();
    let id = uuid::Uuid::new_v4().to_string();
    transport
        .insert(
            id.clone(),
            Arc::new(Transfer {
                cancel: cancel.clone(),
                upload: Mutex::new(None),
                uploaded: Mutex::new(0),
                head: Mutex::new(None),
                output: Mutex::new(output),
                socket: Mutex::new(Some(sink)),
            }),
        )
        .await?;
    tokio::spawn(async move {
        let task = async move {
            while let Some(Ok(message)) = source.next().await {
                if let Message::Text(text) = message {
                    let line = format!("{text}\n");
                    for chunk in line.as_bytes().chunks(CHUNK_BYTES) {
                        if out.send(Ok(chunk.to_vec())).await.is_err() {
                            return;
                        }
                    }
                }
            }
        };
        tokio::select! {_=cancel.cancelled()=>{},_=task=>{}}
    });
    Ok(id)
}
#[tauri::command]
pub async fn runtime_socket_send(
    transport: State<'_, Transport>,
    id: String,
    message: String,
) -> Result<(), String> {
    if message.len() > CHUNK_BYTES {
        return Err("Socket message exceeds limit".into());
    }
    let entry = transport.get(&id).await?;
    let mut socket = entry.socket.lock().await;
    tokio::select! {_=entry.cancel.cancelled()=>Err("Cancelled".into()),result=socket.as_mut().ok_or("Not a socket")?.send(Message::Text(message.into()))=>result.map_err(|e|e.to_string())}
}

#[tauri::command]
pub async fn runtime_reset(transport: State<'_, Transport>) -> Result<(), String> {
    transport.cancel_all().await;
    Ok(())
}
