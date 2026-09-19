//! Claude Code CLI provider implementation.
//!
//! Wraps the existing Claude CLI spawn logic behind the Provider trait.
//! This is a pure extraction — no behavior changes from the existing code.

use super::provider::*;
use super::spawn;
use serde_json::Value;

pub struct ClaudeProvider;

impl Provider for ClaudeProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Claude
    }

    fn find_binary(&self) -> Result<String, String> {
        crate::provider_cli::resolve_executable(&spawn::find_claude_cli())
            .ok_or_else(|| "Claude CLI is not installed; Flamel is installing the latest version. Retry shortly.".to_string())
    }

    fn build_args(&self, opts: &SpawnOpts) -> Vec<String> {
        let mut args = spawn::build_args(
            opts.model,
            opts.permission_mode,
            opts.system_prompt_append,
            opts.resume_session_id,
            opts.mcp_servers,
        );
        if let Some(effort) = opts.reasoning_effort {
            // Claude's native CLI has no `ultra` value; its maximum is `max`.
            let native_effort = if effort == "ultra" { "max" } else { effort };
            if matches!(native_effort, "low" | "medium" | "high" | "xhigh" | "max") {
                args.extend(["--effort".into(), native_effort.into()]);
            }
        }
        args
    }

    fn build_env(
        &self,
        creds: Option<&ProviderCredentials>,
        db: Option<&crate::db::Db>,
        user_home: &str,
        _mcp_servers: Option<&Value>,
        _reasoning_effort: Option<&str>,
    ) -> Vec<(String, String)> {
        // Convert ProviderCredentials to ClaudeCredentials for the existing function
        let claude_creds = creds.map(|c| crate::db::claude_auth::ClaudeCredentials {
            user_id: c.user_id.clone(),
            access_token: c.access_token.clone(),
            refresh_token: c.refresh_token.clone(),
            expires_at: c.expires_at,
            scopes: c.scopes.clone(),
            updated_at: String::new(),
        });
        spawn::build_env(claude_creds.as_ref(), db, user_home)
    }

    fn format_input(&self, message: &str, images: &[ImageBlock]) -> Vec<u8> {
        let content = if images.is_empty() {
            Value::String(message.to_string())
        } else {
            let mut blocks: Vec<Value> = vec![serde_json::json!({
                "type": "text",
                "text": message,
            })];
            for img in images {
                blocks.push(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": img.media_type,
                        "data": img.data,
                    }
                }));
            }
            Value::Array(blocks)
        };

        let msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": content,
            }
        });
        let mut bytes = serde_json::to_vec(&msg).unwrap();
        bytes.push(b'\n');
        bytes
    }

    fn parse_event(&self, line: &str) -> Option<NormalizedEvent> {
        // Claude CLI events are already in our normalized format conceptually.
        // We parse the JSON and wrap it as a Raw event — the existing chat.rs
        // handler processes Claude events directly. Over time we can migrate
        // to full normalization, but for now Raw passthrough preserves behavior.
        let v: Value = serde_json::from_str(line).ok()?;
        Some(NormalizedEvent::Raw { data: v })
    }

    fn serialize_for_frontend(&self, event: &NormalizedEvent) -> Option<Value> {
        // Claude events are already in the frontend's expected format
        match event {
            NormalizedEvent::Raw { data } => Some(data.clone()),
            _ => None, // Other normalized events shouldn't occur for Claude (yet)
        }
    }

    fn disallowed_tools_csv(&self) -> String {
        spawn::disallowed_tools_csv()
    }

    fn default_model(&self) -> &str {
        "claude-sonnet-4-6"
    }
}
