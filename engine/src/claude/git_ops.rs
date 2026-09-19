/// GitHub/Gitea API operations used by the agent runner — comments, status, close.
use std::time::Duration;

use crate::db::git_accounts::GitAccount;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("flamel")
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_default()
}

fn issues_base(account: &GitAccount, owner: &str, repo: &str, number: i64) -> String {
    if account.account_type == "github" {
        format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}")
    } else {
        format!(
            "{}/api/v1/repos/{owner}/{repo}/issues/{number}",
            account.base_url.trim_end_matches('/')
        )
    }
}

/// Post a comment on an issue.
pub async fn post_issue_comment(
    account: &GitAccount,
    owner: &str,
    repo: &str,
    number: i64,
    body: &str,
) {
    let url = format!("{}/comments", issues_base(account, owner, repo, number));
    let res = client()
        .post(&url)
        .bearer_auth(&account.token)
        .json(&serde_json::json!({ "body": body }))
        .send()
        .await;
    if let Ok(r) = res {
        if !r.status().is_success() {
            tracing::warn!(
                "git_ops: comment on {owner}/{repo}#{number} failed: {}",
                r.status()
            );
        }
    }
}

/// Close an issue.
pub async fn close_issue(account: &GitAccount, owner: &str, repo: &str, number: i64) {
    let url = issues_base(account, owner, repo, number);
    let res = client()
        .patch(&url)
        .bearer_auth(&account.token)
        .json(&serde_json::json!({ "state": "closed" }))
        .send()
        .await;
    if let Ok(r) = res {
        if !r.status().is_success() {
            tracing::warn!(
                "git_ops: close {owner}/{repo}#{number} failed: {}",
                r.status()
            );
        }
    }
}

/// Add a label to an issue (GitHub only supports adding, Gitea supports replace).
pub async fn add_label(account: &GitAccount, owner: &str, repo: &str, number: i64, label: &str) {
    let url = format!("{}/labels", issues_base(account, owner, repo, number));
    let res = client()
        .post(&url)
        .bearer_auth(&account.token)
        .json(&serde_json::json!({ "labels": [label] }))
        .send()
        .await;
    if let Ok(r) = res {
        if !r.status().is_success() {
            tracing::debug!(
                "git_ops: add label '{label}' on {owner}/{repo}#{number} failed: {} (label may not exist)",
                r.status()
            );
        }
    }
}
