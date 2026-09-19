//! Native request adapter. Reuses domain handlers without a TCP/HTTP listener.
use crate::AppState;
use axum::{
    body::Body,
    extract::DefaultBodyLimit,
    http::{HeaderName, HeaderValue},
    response::Response,
};
use flamel_protocol::{MAX_BODY_BYTES, Request, VERSION};
use std::sync::Arc;
use tower::ServiceExt;

pub struct Runtime {
    state: Arc<AppState>,
    user_id: String,
    router: axum::Router,
}
impl Runtime {
    pub async fn open(state: Arc<AppState>, profile: Option<&str>) -> Result<Self, String> {
        if !state.local_mode {
            return Err("Native runtime requires local mode".into());
        }
        let user_id = crate::identity::select(&state.db, profile)?;
        let router =
            crate::routes::build_router(state.clone()).layer(DefaultBodyLimit::max(MAX_BODY_BYTES));
        crate::callback::start(state.clone(), user_id.clone()).await?;
        crate::lifecycle::start(state.clone()).await;
        Ok(Self {
            state,
            user_id,
            router,
        })
    }
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
    pub fn state(&self) -> &Arc<AppState> {
        &self.state
    }
    pub fn socket(&self, path: &str) -> Result<crate::native_socket::Socket, String> {
        let (host, client) = crate::native_socket::pair();
        let state = self.state.clone();
        let user = self.user_id.clone();
        let uri: axum::http::Uri = path.parse().map_err(|_| "Invalid socket path")?;
        if uri.path() == "/api/realtime" {
            tokio::spawn(crate::routes::realtime::serve_socket(host, state, user));
        } else if let Some(id) = uri.path().strip_prefix("/api/terminal/connect/") {
            crate::routes::terminal::owns_terminal(&state, id, &user).map_err(|_| "Forbidden")?;
            let cursor = url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
                .find(|(key, _)| key == "after_sequence")
                .and_then(|(_, value)| value.parse().ok())
                .unwrap_or(0);
            tokio::spawn(crate::routes::terminal::terminal_ws(
                host,
                state,
                id.into(),
                cursor,
            ));
        } else {
            return Err("Unsupported native socket".into());
        }
        Ok(client)
    }
    pub async fn request(&self, input: Request, body: Body) -> Result<Response, String> {
        validate_request(&input)?;
        if self.state.is_shutting_down() {
            return Err("Runtime is shutting down".into());
        }
        if !crate::identity::is_bound(&self.state.db, &self.user_id) {
            return Err("Profile access revoked".into());
        }
        let mut request = axum::http::Request::builder()
            .method(input.method.as_str())
            .uri(&input.path);
        for (key, value) in input.headers {
            // Host identity and capability scope can never be supplied by the renderer.
            if matches!(
                key.to_ascii_lowercase().as_str(),
                "accept" | "content-type" | "range" | "if-none-match" | "if-modified-since"
            ) {
                request = request.header(
                    HeaderName::try_from(key).map_err(|e| e.to_string())?,
                    HeaderValue::try_from(value).map_err(|e| e.to_string())?,
                );
            }
        }
        request = request.header("x-flamel-user-id", &self.user_id);
        self.router
            .clone()
            .oneshot(request.body(body).map_err(|e| e.to_string())?)
            .await
            .map_err(|e| e.to_string())
    }
}

pub fn validate_request(input: &Request) -> Result<(), String> {
    if input.version != VERSION {
        return Err("Unsupported runtime protocol version".into());
    }
    let uri: axum::http::Uri = input.path.parse().map_err(|_| "Invalid operation path")?;
    if uri.scheme().is_some()
        || uri.authority().is_some()
        || !uri.path().starts_with("/api/")
        || input.path.contains(['\\', '%', '#'])
        || uri.path().split('/').any(|p| p == ".." || p == ".")
    {
        // Percent escapes in query values are valid; check the path separately below.
        if uri.scheme().is_some()
            || uri.authority().is_some()
            || !uri.path().starts_with("/api/")
            || uri.path().contains(['\\', '%', '#'])
            || uri.path().split('/').any(|p| p == ".." || p == ".")
        {
            return Err("Invalid operation path".into());
        }
    }
    if [
        "/api/internal",
        "/api/runner",
        "/api/flamel-registration",
        "/api/auth/flamel",
        "/api/auth/pending",
        "/api/auth/claim",
    ]
    .iter()
    .any(|prefix| uri.path() == *prefix || uri.path().starts_with(&format!("{prefix}/")))
    {
        return Err("Operation is not available to the desktop renderer".into());
    }
    if !matches!(
        input.method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
    ) {
        return Err("Unsupported method".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_untrusted_targets_and_privileged_routes() {
        for path in [
            "https://evil.test/api/me",
            "//evil.test/api/me",
            "/api/../internal",
            "/api/%69nternal/secrets",
            "/api/internal/secrets",
            "/api/runner/ws",
            "/api/auth/flamel/start",
        ] {
            assert!(
                validate_request(&Request {
                    version: VERSION,
                    method: "GET".into(),
                    path: path.into(),
                    headers: Default::default()
                })
                .is_err(),
                "{path}"
            );
        }
        assert!(
            validate_request(&Request {
                version: VERSION,
                method: "GET".into(),
                path: "/api/files?path=%2Ftmp%2Fhello".into(),
                headers: Default::default()
            })
            .is_ok()
        );
    }
}
