use futures::StreamExt;
use jsonwebtoken::jwk::JwkSet;
use std::time::{Duration, Instant};

const JWKS_URL: &str = "https://flamel.dev/.well-known/jwks.json";
pub const FLAMEL_DEV_ISSUER: &str = "https://flamel.dev";
const CACHE_TTL: Duration = Duration::from_secs(3600);
const KEY_REFRESH_COOLDOWN: Duration = Duration::from_secs(30);
const MAX_JWKS_BYTES: usize = 1024 * 1024;

pub struct JwksCache {
    pub key_set: JwkSet,
    pub fetched_at: Instant,
}

/// Fetch JWKS from flamel.dev, using the in-memory cache if still fresh (1hr TTL).
/// On cache miss or expiry, re-fetches and refreshes the cache.
pub async fn get_jwks(cache: &tokio::sync::Mutex<Option<JwksCache>>) -> Result<JwkSet, String> {
    get_jwks_for_kid(cache, None, JWKS_URL).await
}

async fn get_jwks_for_kid(
    cache: &tokio::sync::Mutex<Option<JwksCache>>,
    kid: Option<&str>,
    url: &str,
) -> Result<JwkSet, String> {
    // Serialize refreshes so concurrent logins cannot stampede the cloud host.
    let mut guard = cache.lock().await;
    if let Some(ref c) = *guard {
        let missing_key = kid.is_some_and(|kid| {
            !c.key_set
                .keys
                .iter()
                .any(|key| key.common.key_id.as_deref() == Some(kid))
        });
        if c.fetched_at.elapsed() < CACHE_TTL
            && (!missing_key || c.fetched_at.elapsed() < KEY_REFRESH_COOLDOWN)
        {
            return Ok(c.key_set.clone());
        }
    }

    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("JWKS client: {e}"))?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("JWKS fetch: {e}"))?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_JWKS_BYTES as u64)
    {
        return Err("JWKS endpoint returned an invalid response".into());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("JWKS read: {e}"))?;
        if body.len().saturating_add(chunk.len()) > MAX_JWKS_BYTES {
            return Err("JWKS response is too large".into());
        }
        body.extend_from_slice(&chunk);
    }

    let key_set: JwkSet = serde_json::from_slice(&body).map_err(|e| format!("JWKS parse: {e}"))?;

    *guard = Some(JwksCache {
        key_set: key_set.clone(),
        fetched_at: Instant::now(),
    });

    Ok(key_set)
}
