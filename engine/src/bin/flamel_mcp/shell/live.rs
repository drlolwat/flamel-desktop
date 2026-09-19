//! Same-host MCP adapter for the existing supervised PTY runtime.
//! Remote workers retain their local subprocess path: never execute on the coordinator.
use super::super::*;

#[derive(Clone)]
pub(super) struct PtyApi {
    client: reqwest::blocking::Client,
    base: String,
    token: String,
    user_id: String,
}

pub(super) fn is_same_host(base: &str) -> bool {
    if base.starts_with("ipc:") {
        return true;
    }
    reqwest::Url::parse(base).is_ok_and(|url| {
        url.scheme() == "http"
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some_and(|host| {
                host == "localhost"
                    || host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
    })
}

impl PtyApi {
    pub(super) fn for_host(base: &str, token: &str) -> Result<Option<Self>, String> {
        if !is_same_host(base) || token.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self {
            client: reqwest::blocking::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(3))
                .build()
                .map_err(|error| error.to_string())?,
            base: base.trim_end_matches('/').into(),
            token: token.into(),
            user_id: flamel_user_id(),
        }))
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::blocking::RequestBuilder {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header("x-flamel-token", &self.token)
            .header("x-flamel-user-id", &self.user_id);
        if let Ok(id) = std::env::var("FLAMEL_SESSION_ID") {
            request = request.header("x-flamel-session-id", id);
        }
        request
    }

    pub(super) fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, String> {
        if self.base.starts_with("ipc:") {
            return flamel::callback::call(
                method.as_str(),
                path,
                body,
                &self.base,
                &self.token,
                &self.user_id,
            );
        }
        let mut request = self
            .request(method, path)
            .timeout(std::time::Duration::from_secs(10));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .map_err(|error| format!("Terminal API request failed: {error}"))?;
        let status = response.status();
        let text = response.text().map_err(|error| error.to_string())?;
        if !status.is_success() {
            return Err(format!("Terminal API {status}: {text}"));
        }
        if text.is_empty() {
            Ok(json!({"ok": true}))
        } else {
            serde_json::from_str(&text).map_err(|error| error.to_string())
        }
    }

    pub(super) fn start(&self, command: &str, cwd: &str) -> Result<Value, String> {
        let mut started = self.call(
            reqwest::Method::POST,
            "/api/terminal/start",
            Some(&json!({"command": command, "cwd": cwd})),
        )?;
        let id = started["session_id"]
            .as_str()
            .ok_or("Terminal response omitted session id")?
            .to_string();
        started["terminal_session_id"] = json!(id);
        started["session_id"] = json!(format!("pty:{id}"));
        Ok(started)
    }

    pub(super) fn forward(
        &self,
        command_session_id: &str,
        timeout_secs: Option<u64>,
    ) -> Result<String, String> {
        let response: Box<dyn std::io::Read> = if self.base.starts_with("ipc:") {
            let response = flamel::callback::request(
                "GET",
                &format!("/api/console/sessions/{command_session_id}/stream"),
                None,
                &self.base,
                &self.token,
                &self.user_id,
            )?;
            if !(200..300).contains(&response.status) {
                return Err(format!("Terminal callback failed ({})", response.status));
            }
            Box::new(response)
        } else {
            let mut request = self.request(
                reqwest::Method::GET,
                &format!("/api/console/sessions/{command_session_id}/stream"),
            );
            if let Some(seconds) = timeout_secs {
                request = request.timeout(std::time::Duration::from_secs(seconds.max(1)));
            }
            let response = request
                .send()
                .and_then(reqwest::blocking::Response::error_for_status)
                .map_err(|error| error.to_string())?;
            Box::new(response)
        };
        let mut output = String::new();
        let mut exit_code = None;
        for line in io::BufReader::new(response).lines() {
            let line = line.map_err(|error| error.to_string())?;
            let payload: Value = serde_json::from_str(&line).map_err(|error| error.to_string())?;
            if payload["type"] == "command_gap" {
                append_capped_shell_output(&mut output, "[Earlier terminal output was truncated]");
            }
            let Some(event) = payload.get("event") else {
                continue;
            };
            if event["kind"] == "output_chunk" {
                if let Some(text) = event.pointer("/chunk/text").and_then(Value::as_str) {
                    append_capped_shell_output(&mut output, text);
                }
            }
            if event["kind"] == "command_finished" {
                exit_code = event.pointer("/run/exit_code").and_then(Value::as_i64);
            }
            send_mcp_command_event(event.clone());
            if event["kind"] == "session_closed" {
                if let Some(code) = exit_code.filter(|code| *code != 0) {
                    output.push_str(&format!("\n[exit {code}]"));
                }
                return Ok(if output.is_empty() {
                    "(no output)".into()
                } else {
                    output
                });
            }
        }
        Err("Terminal stream disconnected before completion was confirmed".into())
    }
}

pub(super) fn terminal_id(id: &str) -> Result<Option<&str>, String> {
    let Some(id) = id.strip_prefix("pty:") else {
        return Ok(None);
    };
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid terminal session id".to_string())?;
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_loopback_mcp_uses_the_host_terminal() {
        assert!(is_same_host("http://127.0.0.1:8181"));
        assert!(is_same_host("http://[::1]:8181"));
        assert!(is_same_host("http://localhost:8181"));
        assert!(!is_same_host("https://code.lolwat.net"));
        assert!(!is_same_host("http://192.0.2.10:8181"));
        assert!(!is_same_host("http://localhost.example.com:8181"));
        assert!(terminal_id("pty:../admin").is_err());
    }
}
