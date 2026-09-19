/// Issue poller — periodically checks GitHub/Gitea for new issues and triggers agent runs.
/// This is an alternative to webhooks: no inbound connectivity required.
use std::sync::Arc;
use tokio::time::Duration;

use crate::AppState;
use crate::db::{agents, git_accounts};

const POLL_INTERVAL: Duration = Duration::from_secs(60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

pub fn start_issue_poller(state: Arc<AppState>) {
    tokio::spawn(async move {
        // Stagger startup so we don't hammer the API immediately on boot
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            poll_all(&state).await;
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

async fn poll_all(state: &Arc<AppState>) {
    let configs = agents::list_enabled_configs_with_tokens(&state.db);
    if configs.is_empty() {
        return;
    }

    let client = reqwest::Client::builder()
        .user_agent("flamel")
        .timeout(HTTP_TIMEOUT)
        .connect_timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_default();

    for (config, _) in configs {
        // Only poll for issue-based triggers (not PR-only configs)
        if config.trigger_event != "issues" && config.trigger_event != "issue_comment" {
            continue;
        }

        let account = match git_accounts::get_git_account_with_token(
            &state.db,
            &state.credential_vault,
            &config.git_account_id,
        ) {
            Some(a) => a,
            None => {
                tracing::warn!("issue_poller: no git account for config {}", config.id);
                continue;
            }
        };

        poll_repo(state, &client, &config, &account).await;
        agents::update_last_polled_at(&state.db, &config.id);
    }
}

async fn poll_repo(
    state: &Arc<AppState>,
    client: &reqwest::Client,
    config: &agents::AgentConfig,
    account: &git_accounts::GitAccount,
) {
    let url = issues_url(account, &config.repo_owner, &config.repo_name);

    let resp = match client.get(&url).bearer_auth(&account.token).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("issue_poller [{}]: GET {} failed: {}", config.id, url, e);
            return;
        }
    };

    if !resp.status().is_success() {
        tracing::warn!(
            "issue_poller [{}]: API returned {}",
            config.id,
            resp.status()
        );
        return;
    }

    let issues: Vec<serde_json::Value> = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("issue_poller [{}]: JSON parse error: {}", config.id, e);
            return;
        }
    };

    for issue in &issues {
        // Skip pull requests — GitHub includes them in /issues, Gitea doesn't
        if issue.get("pull_request").is_some() {
            continue;
        }

        let number = match issue.get("number").and_then(|n| n.as_i64()) {
            Some(n) => n,
            None => continue,
        };

        // Skip if we've already triggered on this issue
        if agents::is_issue_seen(&state.db, &config.id, number) {
            continue;
        }

        // Label filter
        if let Some(ref required_label) = config.trigger_label {
            let labels: Vec<&str> = issue["labels"]
                .as_array()
                .map(|ls| ls.iter().filter_map(|l| l["name"].as_str()).collect())
                .unwrap_or_default();
            if !labels.contains(&required_label.as_str()) {
                // Mark as seen anyway so we don't keep checking open issues without the label.
                // If the label gets added later, the webhook path will catch it.
                // (But we skip creating a run — not matching.)
                // Don't mark seen: if label is added later we won't catch it via polling.
                // Just skip for now and re-check next poll.
                continue;
            }
        }

        // Only trigger on open issues
        if issue.get("state").and_then(|s| s.as_str()) != Some("open") {
            agents::mark_issue_seen(&state.db, &config.id, number);
            continue;
        }

        // Build a synthetic webhook-style payload so the run has full issue context
        let payload = build_payload(config, issue);

        let status = if config.require_approval {
            "awaiting_approval"
        } else {
            "pending"
        };
        agents::create_agent_run(&state.db, &config.id, &payload.to_string(), status);
        agents::mark_issue_seen(&state.db, &config.id, number);

        tracing::info!(
            "issue_poller [{}]: queued run for {}/{}#{} (status={})",
            config.id,
            config.repo_owner,
            config.repo_name,
            number,
            status
        );
    }
}

/// Build a GitHub-compatible payload from a polled issue so agent_runner can consume it unchanged.
fn build_payload(config: &agents::AgentConfig, issue: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "action": "opened",
        "issue": issue,
        "repository": {
            "name": config.repo_name,
            "owner": {
                "login": config.repo_owner,
                "name": config.repo_owner,
            }
        }
    })
}

fn issues_url(account: &git_accounts::GitAccount, owner: &str, repo: &str) -> String {
    if account.account_type == "github" {
        format!(
            "https://api.github.com/repos/{owner}/{repo}/issues?state=open&per_page=50&sort=created&direction=desc"
        )
    } else {
        format!(
            "{}/api/v1/repos/{owner}/{repo}/issues?state=open&limit=50&type=issues",
            account.base_url.trim_end_matches('/')
        )
    }
}
