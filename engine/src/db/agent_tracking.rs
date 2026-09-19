//! Durable child receipts. MCP processes are disposable; their results are not.
use super::Db;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS nested_agent_runs (
        session_id TEXT NOT NULL, run_id TEXT NOT NULL, user_id TEXT NOT NULL,
        parent_run_id TEXT NOT NULL, kind TEXT NOT NULL, snapshot TEXT NOT NULL,
        updated_at INTEGER NOT NULL, PRIMARY KEY(session_id, run_id));
        CREATE INDEX IF NOT EXISTS nested_agent_owner ON nested_agent_runs
        (user_id,session_id,parent_run_id,updated_at);",
    )
}

fn terminal(status: &str) -> bool {
    matches!(
        status,
        "success" | "failed" | "timeout" | "skipped" | "completed_with_errors"
    )
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Only lifecycle receipts need a separate snapshot; the realtime journal owns
/// the full event stream. A late start/progress event cannot reopen finished work.
pub fn record(
    db: &Db,
    user: &str,
    session: &str,
    run: &str,
    parent: &str,
    event: &Value,
) -> rusqlite::Result<()> {
    let kind = event["type"].as_str().unwrap_or("");
    if !matches!(
        kind,
        "batch_start"
            | "batch_progress"
            | "batch_agent_done"
            | "batch_done"
            | "nested_start"
            | "nested_progress"
            | "nested_done"
            | "nested_error"
    ) {
        return Ok(());
    }
    let mut conn = db.conn();
    let tx = conn.transaction()?;
    let existing: Option<(String, String, String)> = tx.query_row(
        "SELECT user_id,parent_run_id,snapshot FROM nested_agent_runs WHERE session_id=?1 AND run_id=?2",
        params![session,run], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    if existing
        .as_ref()
        .is_some_and(|(owner, p, _)| owner != user || p != parent)
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let now = now_ms();
    if existing.is_none() && kind.starts_with("batch_") && kind != "batch_start" {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut snapshot = existing
        .as_ref()
        .and_then(|(_, _, s)| serde_json::from_str::<Value>(s).ok())
        .unwrap_or_else(|| json!({"run_id":run,"status":"running","started_at":now}));
    let finished = terminal(snapshot["status"].as_str().unwrap_or(""));
    if finished && kind != "batch_done" {
        return Ok(());
    }
    if kind == "batch_done" && snapshot.get("finished_at").is_some() {
        return Ok(());
    }
    match kind {
        "batch_start" => {
            snapshot["batch_id"] = json!(run);
            snapshot["description"] = event["description"].clone();
            snapshot["idle_timeout_secs"] = event["idle_timeout_secs"].clone();
            // Replayed starts must not erase any child completion already saved.
            if !snapshot["agents"].is_array() {
                snapshot["agents"] = event["agents"].clone();
            }
        }
        "batch_agent_done" => {
            if let Some(agents) = snapshot["agents"].as_array_mut() {
                if let Some(agent) = agents
                    .iter_mut()
                    .find(|a| a["index"] == event["agent_index"])
                {
                    if terminal(agent["status"].as_str().unwrap_or("")) {
                        return Ok(());
                    }
                    for key in [
                        "status",
                        "duration_ms",
                        "output",
                        "output_truncated",
                        "error",
                    ] {
                        if event.get(key).is_some() {
                            agent[key] = event[key].clone();
                        }
                    }
                    // Never replace a preassigned child ID with an empty failure ID.
                    if event["run_id"].as_str().is_some_and(|s| !s.is_empty()) {
                        agent["run_id"] = event["run_id"].clone();
                    }
                }
            }
        }
        "batch_done" => {
            // The children are authoritative. A count-only batch event cannot
            // fabricate successful children whose receipts never arrived.
            snapshot["finished_at"] = json!(now);
        }
        "nested_start" => {
            snapshot["description"] = event["description"].clone();
            snapshot["subagent_type"] = event["subagent_type"].clone();
            snapshot["idle_timeout_secs"] = event["idle_timeout_secs"].clone();
        }
        "nested_done" | "nested_error" => {
            snapshot["status"] = json!(if kind == "nested_done" && event["ok"] == true {
                "success"
            } else {
                "failed"
            });
            snapshot["output"] = event["output"].clone();
            snapshot["error"] = event["error"].clone();
            snapshot["finished_at"] = json!(now);
        }
        _ => {}
    }
    if let Some(agents) = snapshot["agents"].as_array() {
        let complete = agents
            .iter()
            .all(|a| terminal(a["status"].as_str().unwrap_or("")));
        snapshot["status"] = json!(if complete && !agents.is_empty() {
            if agents.iter().all(|a| a["status"] == "success") {
                "success"
            } else {
                "completed_with_errors"
            }
        } else if snapshot.get("finished_at").is_some() {
            "unknown"
        } else {
            "running"
        });
    }
    tx.execute("INSERT INTO nested_agent_runs(session_id,run_id,user_id,parent_run_id,kind,snapshot,updated_at)
        VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(session_id,run_id) DO UPDATE SET snapshot=excluded.snapshot,updated_at=excluded.updated_at",
        params![session,run,user,parent,if kind.starts_with("batch_") {"batch"} else {"agent"},snapshot.to_string(),now])?;
    tx.commit()
}

pub fn batch_status(
    db: &Db,
    user: &str,
    session: &str,
    parent: &str,
    id: Option<&str>,
) -> rusqlite::Result<Value> {
    let conn = db.conn();
    let row: Option<(String, i64)> = conn.query_row(
        "SELECT snapshot,updated_at FROM nested_agent_runs WHERE user_id=?1 AND session_id=?2
         AND parent_run_id=?3 AND (?4 IS NULL OR run_id=?4)
         ORDER BY CASE WHEN json_extract(snapshot,'$.status')='running' THEN 0 ELSE 1 END, updated_at DESC LIMIT 1",
        params![user,session,parent,id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((raw, updated)) = row else {
        return Ok(
            json!({"status":"not_found","batch_id":id,"total":0,"completed":0,"running":0,"agents":[],
            "message":"No recorded agent batch was found in this session. This does not mean any task completed."}),
        );
    };
    let mut value: Value = serde_json::from_str(&raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if !value["agents"].is_array() {
        // Agent (one child) and AgentBatch share the same recovery tool.
        value["agents"] = json!([value.clone()]);
        value["batch_id"] = value["run_id"].clone();
    }
    let age = now_ms().saturating_sub(updated);
    let stale = age
        > (value["idle_timeout_secs"]
            .as_i64()
            .unwrap_or(300)
            .clamp(60, 86_400)
            + 60)
            * 1000;
    if value["status"] == "running" && stale {
        value["status"] = json!("unknown");
        value["message"] = json!(
            "No recent update from the agent process. Completion is unconfirmed; inspect the saved results before retrying any work."
        );
    }
    let unknown = value["status"] == "unknown";
    let agents = value["agents"]
        .as_array_mut()
        .ok_or(rusqlite::Error::InvalidQuery)?;
    if unknown {
        for agent in agents.iter_mut() {
            if !terminal(agent["status"].as_str().unwrap_or("")) {
                agent["status"] = json!("unknown");
            }
        }
    }
    let total = agents.len();
    let completed = agents
        .iter()
        .filter(|a| terminal(a["status"].as_str().unwrap_or("")))
        .count();
    let succeeded = agents.iter().filter(|a| a["status"] == "success").count();
    let failed = agents.iter().filter(|a| a["status"] == "failed").count();
    let timed_out = agents.iter().filter(|a| a["status"] == "timeout").count();
    let skipped = agents.iter().filter(|a| a["status"] == "skipped").count();
    value["total"] = json!(total);
    value["completed"] = json!(completed);
    value["succeeded"] = json!(succeeded);
    value["failed"] = json!(failed);
    value["timed_out"] = json!(timed_out);
    value["skipped"] = json!(skipped);
    value["running"] = json!(if unknown { 0 } else { total - completed });
    value["last_update_ms_ago"] = json!(age);
    value["updated_at"] = json!(updated);
    value["elapsed_ms"] = json!(
        value["finished_at"]
            .as_i64()
            .unwrap_or_else(now_ms)
            .saturating_sub(value["started_at"].as_i64().unwrap_or(updated))
    );
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipts_survive_reopen_preserve_late_results_and_scope_ownership() {
        let path =
            std::env::temp_dir().join(format!("flamel-agent-tracking-{}.db", uuid::Uuid::new_v4()));
        let db = Db::open(path.to_str().unwrap());
        let start = json!({"type":"batch_start","agents":[{"index":0,"run_id":"child","description":"Test","status":"running"}]});
        record(&db, "owner", "session", "batch", "", &start).unwrap();
        drop(db);
        let db = Db::open(path.to_str().unwrap());
        assert_eq!(
            batch_status(&db, "owner", "session", "", None).unwrap()["running"],
            1
        );
        for (user, sid, parent) in [
            ("other", "session", ""),
            ("owner", "other", ""),
            ("owner", "session", "child"),
        ] {
            assert_eq!(
                batch_status(&db, user, sid, parent, Some("batch")).unwrap()["status"],
                "not_found"
            );
        }
        assert!(record(&db, "other", "session", "batch", "", &start).is_err());
        record(
            &db,
            "owner",
            "session",
            "batch",
            "",
            &json!({"type":"batch_done"}),
        )
        .unwrap();
        assert_eq!(
            batch_status(&db, "owner", "session", "", None).unwrap()["status"],
            "unknown"
        );
        record(&db,"owner","session","batch","",&json!({"type":"batch_agent_done","agent_index":0,"run_id":"","status":"failed","error":"provider stopped"})).unwrap();
        record(&db, "owner", "session", "batch", "", &start).unwrap();
        let status = batch_status(&db, "owner", "session", "", None).unwrap();
        assert_eq!(status["status"], "completed_with_errors");
        assert_eq!(status["agents"][0]["run_id"], "child");
        assert_eq!(status["agents"][0]["error"], "provider stopped");
        assert_eq!(status["completed"], 1);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn stale_and_empty_batches_never_claim_success() {
        let db = Db::open(":memory:");
        assert_eq!(
            batch_status(&db, "u", "s", "", None).unwrap()["status"],
            "not_found"
        );
        record(&db,"u","s","b","",&json!({"type":"batch_start","idle_timeout_secs":i64::MAX,"agents":[{"index":0,"status":"running","run_id":"child"}]})).unwrap();
        db.conn()
            .execute("UPDATE nested_agent_runs SET updated_at=0", [])
            .unwrap();
        let status = batch_status(&db, "u", "s", "", None).unwrap();
        assert_eq!(status["status"], "unknown");
        assert_eq!(status["completed"], 0);
        assert_eq!(status["agents"][0]["status"], "unknown");
    }

    #[test]
    fn single_agent_is_recoverable_and_duplicate_terminal_events_are_immutable() {
        let db = Db::open(":memory:");
        record(
            &db,
            "u",
            "s",
            "single",
            "",
            &json!({"type":"nested_start","description":"Inspect logs"}),
        )
        .unwrap();
        record(
            &db,
            "u",
            "s",
            "single",
            "",
            &json!({"type":"nested_done","ok":true,"output":"Found the actual cause"}),
        )
        .unwrap();
        record(
            &db,
            "u",
            "s",
            "single",
            "",
            &json!({"type":"nested_error","error":"stale replay"}),
        )
        .unwrap();
        let status = batch_status(&db, "u", "s", "", None).unwrap();
        assert_eq!(status["status"], "success");
        assert_eq!(status["total"], 1);
        assert_eq!(status["agents"][0]["output"], "Found the actual cause");
    }
}
