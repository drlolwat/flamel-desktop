//! Remote credentials and networking stay in the native host, scoped to one origin.
use axum::{body::Body, extract::ws::Message};
use flamel_protocol::Request;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
#[derive(Deserialize, Serialize)]
struct Credentials {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    #[serde(default)]
    expires_at: i64,
}
pub struct Remote {
    pub base: String,
    client: reqwest::Client,
    credentials: Mutex<Credentials>,
    remember: bool,
    cloud: Option<(Arc<crate::sso::Sso>, String)>,
}
pub fn validate_base(value: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid server URL")?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("Use an HTTPS server origin (HTTP is supported only on loopback)".into());
    }
    Ok(url.origin().ascii_serialization())
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())
}
fn clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
async fn store(base: String, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move||keyring::Entry::new("dev.flamel.desktop.server",&base).and_then(|entry|entry.set_password(&value)).map_err(|_|"System credential storage is unavailable; uncheck Remember to connect for this session".to_string())).await.map_err(|e|e.to_string())?
}
impl Remote {
    pub async fn login(
        base: &str,
        email: &str,
        password: &str,
        remember: bool,
    ) -> Result<Arc<Self>, String> {
        let base = validate_base(base)?;
        let client = client()?;
        let response = client
            .post(format!("{base}/api/auth/local/login"))
            .json(&serde_json::json!({"email":email,"password":password}))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Sign-in failed ({})", response.status()));
        }
        let mut credentials: Credentials = response
            .json()
            .await
            .map_err(|_| "Invalid sign-in response")?;
        credentials.expires_at = clock() + credentials.expires_in.min(600);
        if remember {
            store(
                base.clone(),
                serde_json::to_string(&credentials).map_err(|e| e.to_string())?,
            )
            .await?;
        }
        Ok(Arc::new(Self {
            base,
            client,
            credentials: Mutex::new(credentials),
            remember,
            cloud: None,
        }))
    }
    pub async fn restore(base: &str) -> Result<Arc<Self>, String> {
        let base = validate_base(base)?;
        let lookup = base.clone();
        let value = tokio::task::spawn_blocking(move || {
            keyring::Entry::new("dev.flamel.desktop.server", &lookup)
                .and_then(|entry| entry.get_password())
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "No saved session for this server")?;
        let credentials: Credentials =
            serde_json::from_str(&value).map_err(|_| "Invalid saved session")?;
        let remote = Arc::new(Self {
            base,
            client: client()?,
            credentials: Mutex::new(credentials),
            remember: true,
            cloud: None,
        });
        remote.access().await?;
        Ok(remote)
    }
    pub async fn from_cloud(
        cloud: Arc<crate::sso::Sso>,
        server: String,
    ) -> Result<Arc<Self>, String> {
        let grant = cloud.grant(&server).await?;
        let base = validate_base(&grant.server_url)?;
        let credentials = Credentials {
            access_token: grant.access_token,
            refresh_token: String::new(),
            expires_in: grant.expires_in,
            expires_at: clock() + grant.expires_in.min(600),
        };
        Ok(Arc::new(Self {
            base,
            client: client()?,
            credentials: Mutex::new(credentials),
            remember: false,
            cloud: Some((cloud, server)),
        }))
    }
    async fn access(&self) -> Result<String, String> {
        let mut credentials = self.credentials.lock().await;
        if credentials.expires_at <= clock() + 30 {
            if let Some((cloud, server)) = &self.cloud {
                let grant = cloud.grant(server).await?;
                if validate_base(&grant.server_url)? != self.base {
                    return Err("Registered server origin changed; select it again".into());
                }
                credentials.access_token = grant.access_token;
                credentials.expires_at = clock() + grant.expires_in.min(600);
                return Ok(credentials.access_token.clone());
            }

            let response = self
                .client
                .post(format!("{}/api/auth/local/refresh", self.base))
                .json(&serde_json::json!({"refresh_token":credentials.refresh_token}))
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !response.status().is_success() {
                return Err("Session expired; sign in again".into());
            }
            let mut next: Credentials = response
                .json()
                .await
                .map_err(|_| "Invalid refresh response")?;
            next.expires_at = clock() + next.expires_in.min(600);
            *credentials = next;
            if self.remember {
                store(
                    self.base.clone(),
                    serde_json::to_string(&*credentials).map_err(|e| e.to_string())?,
                )
                .await?;
            }
        }
        Ok(credentials.access_token.clone())
    }
    pub async fn request(
        &self,
        input: Request,
        body: Body,
    ) -> Result<axum::response::Response, String> {
        flamel::native::validate_request(&input)?;
        if input.path.starts_with("/api/auth/") {
            return Err("Account grants are managed by the native host".into());
        }
        let access = self.access().await?;
        let mut request = self
            .client
            .request(
                input.method.parse().map_err(|_| "Invalid method")?,
                format!("{}{}", self.base, input.path),
            )
            .bearer_auth(access);
        for (key, value) in input.headers {
            if matches!(
                key.to_ascii_lowercase().as_str(),
                "accept" | "content-type" | "range" | "if-none-match" | "if-modified-since"
            ) {
                request = request.header(key, value);
            }
        }
        let response = request
            .body(reqwest::Body::wrap_stream(body.into_data_stream()))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if response.status().is_redirection() {
            return Err(
                "Server redirected the request; select its final HTTPS origin explicitly".into(),
            );
        }
        let mut builder = axum::response::Response::builder().status(response.status());
        for (key, value) in response.headers() {
            if !matches!(key.as_str(), "set-cookie" | "authorization") {
                builder = builder.header(key, value);
            }
        }
        builder
            .body(Body::from_stream(response.bytes_stream()))
            .map_err(|e| e.to_string())
    }
    pub async fn socket(&self, path: &str) -> Result<flamel::native_socket::Socket, String> {
        let parsed = reqwest::Url::parse(&format!("{}{}", self.base, path))
            .map_err(|_| "Invalid socket path")?;
        if !(parsed.path() == "/api/realtime"
            || parsed.path().starts_with("/api/terminal/connect/"))
            || parsed.origin().ascii_serialization() != self.base
        {
            return Err("Invalid socket operation".into());
        }
        let mut url = parsed;
        url.set_scheme(if self.base.starts_with("https:") {
            "wss"
        } else {
            "ws"
        })
        .map_err(|_| "Invalid socket scheme")?;
        // One-use tickets are obtained through the authenticated selected origin.
        let (websocket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .map_err(|e| e.to_string())?;
        let (mut tx, mut rx) = websocket.split();
        let (host, client) = flamel::native_socket::pair();
        let (mut sink, mut source) = host.split();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    value=rx.next()=>match value {
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))=>if text.len()>flamel_protocol::MAX_BODY_BYTES || sink.send(Message::Text(text.into())).await.is_err(){break;},
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(data)))=>{if tx.send(tokio_tungstenite::tungstenite::Message::Pong(data)).await.is_err(){break;}},
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))|Some(Err(_))|None=>break,
                        _=>{},
                    },
                    value=source.next()=>match value {
                        Some(Ok(Message::Text(text)))=>if tx.send(tokio_tungstenite::tungstenite::Message::Text(text.to_string())).await.is_err(){break;},
                        _=>break,
                    }
                }
            }
            let _ = tx.close().await;
        });
        Ok(client)
    }
    pub async fn logout(&self) -> Result<(), String> {
        if self.cloud.is_some() {
            return Ok(());
        }
        let credentials = self.credentials.lock().await;
        self.client
            .post(format!("{}/api/auth/local/logout", self.base))
            .json(&serde_json::json!({"refresh_token":credentials.refresh_token}))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        let base = self.base.clone();
        if self.remember {
            tokio::task::spawn_blocking(move || {
                keyring::Entry::new("dev.flamel.desktop.server", &base)
                    .and_then(|entry| entry.delete_credential())
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_credential_and_origin_confusion() {
        for value in [
            "https://user:pass@example.test",
            "http://example.test",
            "https://example.test/path",
            "https://example.test?token=secret",
            "https://example.test#x",
            "javascript:alert(1)",
        ] {
            assert!(validate_base(value).is_err());
        }
        assert_eq!(
            validate_base("https://example.test/").unwrap(),
            "https://example.test"
        );
    }
}
