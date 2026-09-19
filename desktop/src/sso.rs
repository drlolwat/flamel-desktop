//! Optional public OAuth client: system browser, S256 PKCE, temporary callback only.
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};
const CLOUD: &str = "https://flamel.dev";
const CLIENT: &str = "flamel-desktop";
#[derive(Deserialize, Serialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    #[serde(default)]
    expires_at: i64,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub name: String,
    pub public_url: String,
}
#[derive(Deserialize)]
pub struct ServerGrant {
    pub access_token: String,
    pub expires_in: i64,
    pub server_id: String,
    pub server_url: String,
}
pub struct Sso {
    client: reqwest::Client,
    tokens: Mutex<Tokens>,
    remember: bool,
}
fn clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
async fn store(tokens: &Tokens) -> Result<(), String> {
    let value = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        keyring::Entry::new("dev.flamel.desktop.cloud", CLIENT)
            .and_then(|entry| entry.set_password(&value))
            .map_err(|_| "System credential storage unavailable; try without Remember".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
impl Sso {
    pub async fn login(remember: bool) -> Result<Arc<Self>, String> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string())?;
        let redirect = format!(
            "http://{}/api/auth/flamel/callback",
            listener.local_addr().map_err(|e| e.to_string())?
        );
        let encode = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let verifier = encode.encode(rand::random::<[u8; 32]>());
        let challenge = encode.encode(Sha256::digest(verifier.as_bytes()));
        let state = encode.encode(rand::random::<[u8; 32]>());
        let mut authorize =
            reqwest::Url::parse(&format!("{CLOUD}/api/oauth/local/authorize")).unwrap();
        authorize.query_pairs_mut().extend_pairs([
            ("client_id", CLIENT),
            ("response_type", "code"),
            ("redirect_uri", redirect.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
        ]);
        open::that(authorize.as_str()).map_err(|e| e.to_string())?;
        let code=tokio::time::timeout(std::time::Duration::from_secs(180),async {
            loop {
                let(mut stream,_)=listener.accept().await.map_err(|e|e.to_string())?;
                let request=tokio::time::timeout(std::time::Duration::from_secs(3),async {
                    let mut bytes=Vec::new();let mut one=[0;1];
                    while bytes.len()<8192 {
                        if stream.read(&mut one).await.map_err(|e|e.to_string())?==0{break;}
                        bytes.push(one[0]);if bytes.ends_with(b"\r\n\r\n"){break;}
                    }
                    String::from_utf8(bytes).map_err(|e|e.to_string())
                }).await;
                let code=request.ok().and_then(Result::ok).and_then(|request|callback_code(&request,&redirect,&state));
                let response=if code.is_some(){"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nSign-in received. Return to Flamel Desktop."}else{"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nInvalid callback"};
                let _=stream.write_all(response.as_bytes()).await;
                if let Some(code)=code{return Ok::<_,String>(code);}
            }
        }).await.map_err(|_|"Sign-in timed out")??;
        drop(listener);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?;
        let response=client.post(format!("{CLOUD}/api/oauth/desktop/token")).json(&serde_json::json!({"client_id":CLIENT,"grant_type":"authorization_code","code":code,"code_verifier":verifier,"redirect_uri":redirect})).send().await.map_err(|e|e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Flamel sign-in failed ({})", response.status()));
        }
        let mut tokens: Tokens = response
            .json()
            .await
            .map_err(|_| "Invalid Flamel token response")?;
        tokens.expires_at = clock() + tokens.expires_in.min(600);
        if remember {
            store(&tokens).await?;
        }
        Ok(Arc::new(Self {
            client,
            tokens: Mutex::new(tokens),
            remember,
        }))
    }
    pub async fn restore() -> Result<Arc<Self>, String> {
        let value = tokio::task::spawn_blocking(|| {
            keyring::Entry::new("dev.flamel.desktop.cloud", CLIENT)
                .and_then(|entry| entry.get_password())
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "No saved Flamel session")?;
        let tokens: Tokens =
            serde_json::from_str(&value).map_err(|_| "Invalid saved Flamel session")?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?;
        let session = Arc::new(Self {
            client,
            tokens: Mutex::new(tokens),
            remember: true,
        });
        session.access().await?;
        Ok(session)
    }
    pub async fn logout(&self) -> Result<(), String> {
        let tokens = self.tokens.lock().await;
        let response=self.client.post(format!("{CLOUD}/api/oauth/desktop/token")).json(&serde_json::json!({"client_id":CLIENT,"grant_type":"revoke","refresh_token":tokens.refresh_token})).send().await.map_err(|e|e.to_string())?;
        if !response.status().is_success() {
            return Err("Flamel session revocation failed".into());
        }
        if self.remember {
            tokio::task::spawn_blocking(|| {
                keyring::Entry::new("dev.flamel.desktop.cloud", CLIENT)
                    .and_then(|entry| entry.delete_credential())
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    async fn access(&self) -> Result<String, String> {
        let mut tokens = self.tokens.lock().await;
        if tokens.expires_at <= clock() + 30 {
            let response=self.client.post(format!("{CLOUD}/api/oauth/desktop/token")).json(&serde_json::json!({"client_id":CLIENT,"grant_type":"refresh_token","refresh_token":tokens.refresh_token})).send().await.map_err(|e|e.to_string())?;
            if !response.status().is_success() {
                return Err("Flamel session expired; sign in again".into());
            }
            let mut next: Tokens = response
                .json()
                .await
                .map_err(|_| "Invalid Flamel refresh response")?;
            next.expires_at = clock() + next.expires_in.min(600);
            *tokens = next;
            if self.remember {
                store(&tokens).await?;
            }
        }
        Ok(tokens.access_token.clone())
    }
    pub async fn servers(&self) -> Result<Vec<Server>, String> {
        #[derive(Deserialize)]
        struct List {
            servers: Vec<Server>,
        }
        self.client
            .get(format!("{CLOUD}/api/desktop/servers"))
            .bearer_auth(self.access().await?)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json::<List>()
            .await
            .map(|value| value.servers)
            .map_err(|e| e.to_string())
    }
    pub async fn grant(&self, server: &str) -> Result<ServerGrant, String> {
        let grant = self
            .client
            .post(format!("{CLOUD}/api/desktop/server-token"))
            .bearer_auth(self.access().await?)
            .json(&serde_json::json!({"server_id":server}))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json::<ServerGrant>()
            .await
            .map_err(|e| e.to_string())?;
        if grant.server_id != server {
            return Err("Server grant mismatch".into());
        }
        Ok(grant)
    }
}
fn callback_code(request: &str, redirect: &str, state: &str) -> Option<String> {
    let mut parts = request.lines().next()?.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let path = parts.next()?;
    if !path.starts_with("/api/auth/flamel/callback?") {
        return None;
    }
    let url = reqwest::Url::parse(redirect).ok()?.join(path).ok()?;
    let pairs = url.query_pairs().collect::<Vec<_>>();
    if pairs.iter().filter(|(key, _)| key == "state").count() != 1
        || pairs.iter().find(|(key, _)| key == "state")?.1 != state
        || pairs.iter().filter(|(key, _)| key == "code").count() != 1
    {
        return None;
    }
    let code = pairs.iter().find(|(key, _)| key == "code")?.1.to_string();
    if code.is_empty() || code.len() > 1024 {
        return None;
    }
    Some(code)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_rejects_missing_wrong_and_duplicate_state() {
        let redirect = "http://127.0.0.1:1234/api/auth/flamel/callback";
        for path in [
            "/?state=ok&code=x",
            "/api/auth/flamel/callback?state=no&code=x",
            "/api/auth/flamel/callback?state=ok&state=no&code=x",
        ] {
            assert!(
                callback_code(&format!("GET {path} HTTP/1.1\r\n\r\n"), redirect, "ok").is_none()
            );
        }
        assert_eq!(
            callback_code(
                "GET /api/auth/flamel/callback?state=ok&code=hello HTTP/1.1\r\n\r\n",
                redirect,
                "ok"
            )
            .as_deref(),
            Some("hello")
        );
    }
}

impl flamel::service_access::ServiceAccess for Sso {
    fn forge_assertion(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + '_>>
    {
        Box::pin(async move {
            #[derive(Deserialize)]
            struct Grant {
                assertion: String,
            }
            self.client
                .post(format!("{CLOUD}/api/desktop/forge-token"))
                .bearer_auth(self.access().await?)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json::<Grant>()
                .await
                .map(|grant| grant.assertion)
                .map_err(|_| "Invalid Git grant".into())
        })
    }
}
