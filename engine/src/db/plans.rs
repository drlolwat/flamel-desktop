use crate::db::Db;
use serde::{Deserialize, Serialize};

const PLAN_ARTIFACT_SCHEMA_VERSION: i64 = 1;
const MAX_PLAN_VERSIONS: usize = 20;
const MAX_PLAN_CONTEXT_PLANS: usize = 3;
const MAX_PLAN_CONTEXT_NEXT_ITEMS: usize = 5;
const MAX_PLAN_CONTEXT_TEXT_CHARS: usize = 180;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanArtifact {
    pub id: String,
    pub plan_id: String,
    pub kind: String,
    pub schema_version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanVersion {
    pub id: String,
    pub items: Vec<PlanItem>,
    pub timestamp: String,
    pub schema_version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<PlanArtifact>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanMeta {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    pub item_count: i64,
    pub done_count: i64,
    pub version_count: i64,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_token: Option<String>,
    pub finished: bool,
    pub archived: bool,
    pub execution_status: String,
    pub approval_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_requested_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<PlanArtifact>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    #[serde(flatten)]
    pub meta: PlanMeta,
    pub items: Vec<PlanItem>,
    pub versions: Vec<PlanVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub id: String,
    pub text: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub reasoning_floor: String,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanItemUpdateError {
    NotFound,
    ApprovalRequired,
    BrowserReviewRequired,
}

fn normalize_plan_execution_status(status: &str) -> &'static str {
    match status {
        "running" => "running",
        "paused" => "paused",
        "blocked" => "blocked",
        "completed" => "completed",
        _ => "ready",
    }
}

fn normalize_plan_approval_status(status: &str) -> &'static str {
    match status {
        "approved" => "approved",
        "not_required" => "not_required",
        _ => "pending",
    }
}

fn compact_plan_context_text(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    if max_chars <= 3 {
        return normalized.chars().take(max_chars).collect();
    }

    let mut truncated = normalized
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

fn compact_plan_item_for_context(item: &PlanItem) -> String {
    let section = item
        .section
        .as_deref()
        .map(str::trim)
        .filter(|section| !section.is_empty())
        .map(|section| format!("{section}: "))
        .unwrap_or_default();
    format!(
        "{}[{}, Auto-only reasoning floor={}] {}",
        section,
        item.status,
        item.reasoning_floor,
        compact_plan_context_text(&item.text, MAX_PLAN_CONTEXT_TEXT_CHARS)
    )
}

fn build_plan_artifact_metadata(
    conn: &rusqlite::Connection,
    plan_id: &str,
    synced_at: &str,
) -> Option<serde_json::Value> {
    let plan = snapshot_plan_metadata(conn, plan_id)?;
    let items = get_plan_items(conn, plan_id);
    let item_count = items.len();
    let done_count = items.iter().filter(|item| item.status == "done").count();
    let blocked_count = items.iter().filter(|item| item.status == "blocked").count();
    let active_item_id = plan
        .get("activeItemId")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let active_item = active_item_id.as_deref().and_then(|id| {
        items.iter().find(|item| item.id == id).map(|item| {
            serde_json::json!({
                "id": item.id,
                "text": compact_plan_context_text(&item.text, MAX_PLAN_CONTEXT_TEXT_CHARS),
                "status": item.status,
                "section": item.section,
                "reasoningFloor": item.reasoning_floor,
                "position": item.position,
            })
        })
    });
    let next_items = items
        .iter()
        .filter(|item| item.status != "done")
        .take(MAX_PLAN_CONTEXT_NEXT_ITEMS)
        .map(|item| {
            serde_json::json!({
                "id": item.id,
                "text": compact_plan_context_text(&item.text, MAX_PLAN_CONTEXT_TEXT_CHARS),
                "status": item.status,
                "section": item.section,
                "reasoningFloor": item.reasoning_floor,
                "position": item.position,
            })
        })
        .collect::<Vec<_>>();

    Some(serde_json::json!({
        "schemaVersion": PLAN_ARTIFACT_SCHEMA_VERSION,
        "autosync": "accepted_plan_context",
        "syncedAt": synced_at,
        "plan": plan,
        "progress": {
            "itemCount": item_count,
            "doneCount": done_count,
            "blockedCount": blocked_count,
        },
        "activeItem": active_item,
        "nextItems": next_items,
    }))
}

fn format_plan_context_summary(plan: &Plan) -> String {
    let mut lines = Vec::new();
    let artifact_suffix = plan
        .meta
        .artifact
        .as_ref()
        .map(|artifact| format!(", artifact_id: {}", artifact.id))
        .unwrap_or_default();
    lines.push(format!(
        "- {} (plan_id: {}{})",
        compact_plan_context_text(&plan.meta.title, MAX_PLAN_CONTEXT_TEXT_CHARS),
        plan.meta.id,
        artifact_suffix
    ));
    lines.push(format!(
        "  State: approval={}, execution={}, progress={}/{} done, updated={}",
        plan.meta.approval_status,
        plan.meta.execution_status,
        plan.meta.done_count,
        plan.meta.item_count,
        plan.meta.updated_at
    ));
    if let Some(description) = plan
        .meta
        .description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty())
    {
        lines.push(format!(
            "  Description: {}",
            compact_plan_context_text(description, MAX_PLAN_CONTEXT_TEXT_CHARS)
        ));
    }
    if let Some(active_item) = plan.meta.active_item_id.as_deref().and_then(|id| {
        plan.items
            .iter()
            .find(|item| item.id == id)
            .map(compact_plan_item_for_context)
    }) {
        lines.push(format!("  Active item: {active_item}"));
    }
    let next_items = plan
        .items
        .iter()
        .filter(|item| item.status != "done")
        .take(MAX_PLAN_CONTEXT_NEXT_ITEMS)
        .map(compact_plan_item_for_context)
        .collect::<Vec<_>>();
    if !next_items.is_empty() {
        lines.push(format!("  Next items: {}", next_items.join("; ")));
    }
    lines.join("\n")
}

fn sync_plan_artifact(conn: &rusqlite::Connection, plan_id: &str) {
    let artifact_id = format!("plan-artifact-{plan_id}");
    conn.execute(
        "INSERT OR IGNORE INTO plan_artifacts (
            id, plan_id, kind, schema_version, session_id, project_path, user_id, metadata, created_at, updated_at
        )
        SELECT ?1, id, 'plan', ?2, session_id, project_path, user_id, '{}', created_at, updated_at
        FROM plans WHERE id = ?3",
        rusqlite::params![artifact_id, PLAN_ARTIFACT_SCHEMA_VERSION, plan_id],
    )
    .ok();

    let now = chrono::Utc::now().to_rfc3339();
    let metadata = build_plan_artifact_metadata(conn, plan_id, &now)
        .and_then(|value| serde_json::to_string(&value).ok())
        .unwrap_or_else(|| "{}".to_string());
    conn.execute(
        "UPDATE plan_artifacts
         SET session_id = (SELECT session_id FROM plans WHERE id = ?1),
             project_path = (SELECT project_path FROM plans WHERE id = ?1),
             user_id = (SELECT user_id FROM plans WHERE id = ?1),
             metadata = ?2,
             updated_at = ?3
         WHERE plan_id = ?1",
        rusqlite::params![plan_id, metadata, now],
    )
    .ok();
}

fn get_plan_artifact(conn: &rusqlite::Connection, plan_id: &str) -> Option<PlanArtifact> {
    conn.query_row(
        "SELECT id, plan_id, kind, schema_version, session_id, project_path, metadata, created_at, updated_at
         FROM plan_artifacts WHERE plan_id = ?1",
        [plan_id],
        |row| {
            let metadata_raw: String = row.get(6)?;
            let metadata = serde_json::from_str(&metadata_raw).unwrap_or_else(|_| serde_json::json!({}));
            Ok(PlanArtifact {
                id: row.get(0)?,
                plan_id: row.get(1)?,
                kind: row.get(2)?,
                schema_version: row.get(3)?,
                session_id: row.get(4)?,
                project_path: row.get(5)?,
                metadata,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        },
    )
    .ok()
}

fn get_version_count(conn: &rusqlite::Connection, plan_id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM plan_versions WHERE plan_id = ?1",
        [plan_id],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

fn snapshot_plan_metadata(conn: &rusqlite::Connection, plan_id: &str) -> Option<serde_json::Value> {
    conn.query_row(
        "SELECT title, description, session_id, project_path, finished, archived, execution_status, approval_status, active_item_id, approval_requested_at, approved_at, execution_started_at, execution_updated_at FROM plans WHERE id = ?1",
        [plan_id],
        |row| {
            let title: String = row.get(0)?;
            let description: Option<String> = row.get(1)?;
            let session_id: Option<String> = row.get(2)?;
            let project_path: Option<String> = row.get(3)?;
            let finished = row.get::<_, Option<i64>>(4)?.unwrap_or(0) == 1;
            let archived = row.get::<_, Option<i64>>(5)?.unwrap_or(0) == 1;
            let execution_status = row
                .get::<_, Option<String>>(6)?
                .unwrap_or_else(|| "ready".to_string());
            let approval_status = row
                .get::<_, Option<String>>(7)?
                .unwrap_or_else(|| "approved".to_string());
            let active_item_id: Option<String> = row.get(8)?;
            let approval_requested_at: Option<String> = row.get(9)?;
            let approved_at: Option<String> = row.get(10)?;
            let execution_started_at: Option<String> = row.get(11)?;
            let execution_updated_at: Option<String> = row.get(12)?;
            Ok(serde_json::json!({
                "title": title,
                "description": description,
                "sessionId": session_id,
                "projectPath": project_path,
                "finished": finished,
                "archived": archived,
                "executionStatus": execution_status,
                "approvalStatus": approval_status,
                "activeItemId": active_item_id,
                "approvalRequestedAt": approval_requested_at,
                "approvedAt": approved_at,
                "executionStartedAt": execution_started_at,
                "executionUpdatedAt": execution_updated_at,
            }))
        },
    )
    .ok()
}

fn take_version_snapshot(conn: &rusqlite::Connection, plan_id: &str) {
    sync_plan_artifact(conn, plan_id);

    let items = get_plan_items(conn, plan_id);
    let artifact = get_plan_artifact(conn, plan_id);
    let plan = snapshot_plan_metadata(conn, plan_id).unwrap_or_else(|| serde_json::json!({}));
    let version_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let snapshot = serde_json::to_string(&serde_json::json!({
        "schemaVersion": PLAN_ARTIFACT_SCHEMA_VERSION,
        "plan": plan,
        "artifact": artifact,
        "items": items,
    }))
    .unwrap_or_else(|_| "[]".to_string());

    conn.execute(
        "INSERT INTO plan_versions (id, plan_id, snapshot, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![version_id, plan_id, snapshot, now],
    )
    .ok();

    let mut older_stmt = conn
        .prepare("SELECT id FROM plan_versions WHERE plan_id = ?1 ORDER BY created_at DESC LIMIT -1 OFFSET ?2")
        .unwrap();
    let old_ids: Vec<String> = older_stmt
        .query_map(
            rusqlite::params![plan_id, MAX_PLAN_VERSIONS as i64],
            |row| row.get(0),
        )
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    for old_id in old_ids {
        conn.execute("DELETE FROM plan_versions WHERE id = ?1", [&old_id])
            .ok();
    }
}

fn plan_version_from_snapshot(
    id: String,
    snapshot: serde_json::Value,
    timestamp: String,
) -> PlanVersion {
    if snapshot.is_array() {
        return PlanVersion {
            id,
            items: serde_json::from_value(snapshot).unwrap_or_default(),
            timestamp,
            schema_version: 0,
            title: None,
            description: None,
            artifact: None,
        };
    }

    let schema_version = snapshot
        .get("schemaVersion")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    let items = snapshot
        .get("items")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let plan = snapshot.get("plan");
    let title = plan
        .and_then(|value| value.get("title"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let description = plan
        .and_then(|value| value.get("description"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let artifact = snapshot
        .get("artifact")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());

    PlanVersion {
        id,
        items,
        timestamp,
        schema_version,
        title,
        description,
        artifact,
    }
}

fn get_plan_versions(conn: &rusqlite::Connection, plan_id: &str) -> Vec<PlanVersion> {
    let mut stmt = conn
        .prepare("SELECT id, snapshot, created_at FROM plan_versions WHERE plan_id = ?1 ORDER BY created_at DESC LIMIT ?2")
        .unwrap();
    stmt.query_map(
        rusqlite::params![plan_id, MAX_PLAN_VERSIONS as i64],
        |row| {
            let id: String = row.get(0)?;
            let snapshot_raw: String = row.get(1)?;
            let timestamp: String = row.get(2)?;
            let snapshot =
                serde_json::from_str(&snapshot_raw).unwrap_or_else(|_| serde_json::json!([]));
            Ok(plan_version_from_snapshot(id, snapshot, timestamp))
        },
    )
    .unwrap()
    .filter_map(|r| r.ok())
    .collect()
}

fn row_to_plan_meta(
    conn: &rusqlite::Connection,
    row: &rusqlite::Row,
) -> rusqlite::Result<PlanMeta> {
    let id: String = row.get("id")?;
    let done_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM plan_items WHERE plan_id = ?1 AND status = 'done'",
            [&id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let item_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM plan_items WHERE plan_id = ?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let version_count = get_version_count(conn, &id);
    let artifact = get_plan_artifact(conn, &id);

    Ok(PlanMeta {
        id,
        title: row.get("title")?,
        description: row.get("description")?,
        session_id: row.get("session_id")?,
        project_path: row.get("project_path")?,
        item_count,
        done_count,
        version_count,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        share_token: row.get("share_token")?,
        finished: row.get::<_, Option<i64>>("finished")?.unwrap_or(0) == 1,
        archived: row.get::<_, Option<i64>>("archived")?.unwrap_or(0) == 1,
        execution_status: normalize_plan_execution_status(
            &row.get::<_, Option<String>>("execution_status")?
                .unwrap_or_else(|| "ready".to_string()),
        )
        .to_string(),
        approval_status: normalize_plan_approval_status(
            &row.get::<_, Option<String>>("approval_status")?
                .unwrap_or_else(|| "approved".to_string()),
        )
        .to_string(),
        active_item_id: row.get("active_item_id")?,
        approval_requested_at: row.get("approval_requested_at")?,
        approved_at: row.get("approved_at")?,
        execution_started_at: row.get("execution_started_at")?,
        execution_updated_at: row.get("execution_updated_at")?,
        artifact,
    })
}

fn get_plan_items(conn: &rusqlite::Connection, plan_id: &str) -> Vec<PlanItem> {
    let mut stmt = conn
        .prepare("SELECT id, text, status, notes, position, section, reasoning_floor FROM plan_items WHERE plan_id = ?1 ORDER BY position ASC")
        .unwrap();
    stmt.query_map([plan_id], |row| {
        Ok(PlanItem {
            id: row.get(0)?,
            text: row.get(1)?,
            status: row.get(2)?,
            notes: row.get(3)?,
            position: row.get(4)?,
            section: row.get(5)?,
            reasoning_floor: normalize_reasoning_floor(row.get::<_, Option<String>>(6)?.as_deref())
                .to_string(),
        })
    })
    .unwrap()
    .filter_map(|r| r.ok())
    .collect()
}

fn normalize_reasoning_floor(value: Option<&str>) -> &'static str {
    match value.map(str::trim) {
        Some("low") => "low",
        Some("high") => "high",
        Some("xhigh") => "xhigh",
        Some("max") => "max",
        Some("ultra") => "ultra",
        _ => "medium",
    }
}

fn reasoning_floor_rank(value: &str) -> u8 {
    match value {
        "low" => 0,
        "medium" => 1,
        "high" => 2,
        "xhigh" => 3,
        "max" => 4,
        "ultra" => 5,
        _ => 1,
    }
}

pub fn active_plan_reasoning_floor(db: &Db, plan_id: &str) -> Option<String> {
    let plan = get_plan(db, plan_id)?;
    let item = plan
        .meta
        .active_item_id
        .as_deref()
        .and_then(|id| plan.items.iter().find(|item| item.id == id))
        .or_else(|| plan.items.iter().find(|item| item.status == "in_progress"))
        .or_else(|| plan.items.iter().find(|item| item.status == "todo"))?;
    Some(item.reasoning_floor.clone())
}

#[derive(Clone, Copy)]
enum NewItemStatusMode {
    ForceTodo,
    PreserveAtMostOneInProgress,
}

fn normalize_update_status(status: &str) -> &'static str {
    match status {
        "inProgress" | "in_progress" => "in_progress",
        "done" => "done",
        "blocked" => "blocked",
        "todo" => "todo",
        _ => "todo",
    }
}

fn normalize_new_item_status(
    item: &serde_json::Value,
    mode: NewItemStatusMode,
    claimed_in_progress: &mut bool,
) -> String {
    let Some(requested) = item.get("status").and_then(|v| v.as_str()) else {
        return "todo".to_string();
    };

    match mode {
        NewItemStatusMode::ForceTodo => "todo".to_string(),
        NewItemStatusMode::PreserveAtMostOneInProgress => {
            match normalize_update_status(requested) {
                "in_progress" if !*claimed_in_progress => {
                    *claimed_in_progress = true;
                    "in_progress".to_string()
                }
                "done" => "done".to_string(),
                "blocked" => "blocked".to_string(),
                _ => "todo".to_string(),
            }
        }
    }
}

fn clear_other_in_progress_items(
    conn: &rusqlite::Connection,
    plan_id: &str,
    item_id: &str,
    now: &str,
) {
    conn.execute(
        "UPDATE plan_items SET status = 'todo', updated_at = ?1 WHERE plan_id = ?2 AND id != ?3 AND status = 'in_progress'",
        rusqlite::params![now, plan_id, item_id],
    )
    .ok();
}

fn plan_approval_status(conn: &rusqlite::Connection, plan_id: &str) -> String {
    conn.query_row(
        "SELECT approval_status FROM plans WHERE id = ?1",
        [plan_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| "approved".to_string())
}

fn refresh_plan_execution_from_items(conn: &rusqlite::Connection, plan_id: &str, now: &str) {
    let active_item_id: Option<String> = conn
        .query_row(
            "SELECT id FROM plan_items WHERE plan_id = ?1 AND status = 'in_progress' ORDER BY position ASC LIMIT 1",
            [plan_id],
            |row| row.get(0),
        )
        .ok();
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM plan_items WHERE plan_id = ?1",
            [plan_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let done: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM plan_items WHERE plan_id = ?1 AND status = 'done'",
            [plan_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let blocked: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM plan_items WHERE plan_id = ?1 AND status = 'blocked'",
            [plan_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let approval_status = plan_approval_status(conn, plan_id);
    let current_status = conn
        .query_row(
            "SELECT execution_status FROM plans WHERE id = ?1",
            [plan_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .unwrap_or_else(|| "ready".to_string());

    let execution_status = if total > 0
        && done == total
        && super::plan_quality::satisfied(conn, plan_id)
    {
        "completed"
    } else if current_status == "paused" {
        "paused"
    } else if blocked > 0 && blocked + done == total {
        "blocked"
    } else if approval_status == "approved" || approval_status == "not_required" {
        if active_item_id.is_some() || current_status == "running" || current_status == "completed"
        {
            "running"
        } else {
            "ready"
        }
    } else {
        "ready"
    };

    conn.execute(
        "UPDATE plans
         SET execution_status = ?1,
             active_item_id = ?2,
             finished = ?3,
             execution_updated_at = ?4,
             updated_at = ?4
         WHERE id = ?5",
        rusqlite::params![
            execution_status,
            active_item_id,
            if execution_status == "completed" {
                1
            } else {
                0
            },
            now,
            plan_id
        ],
    )
    .ok();
}

pub fn get_plan_user_id(db: &Db, id: &str) -> Option<String> {
    let conn = db.conn();
    conn.query_row("SELECT user_id FROM plans WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .ok()
}

pub fn list_plans(
    db: &Db,
    query: Option<&str>,
    project_path: Option<&str>,
    session_id: Option<&str>,
    user_id: Option<&str>,
    archived: bool,
) -> Vec<PlanMeta> {
    let conn = db.conn();
    let mut sql = "SELECT * FROM plans WHERE (archived IS NULL OR archived = 0)".to_string();
    if archived {
        sql = "SELECT * FROM plans WHERE archived = 1".to_string();
    }
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(q) = query {
        sql.push_str(&format!(" AND title LIKE ?{}", params.len() + 1));
        params.push(Box::new(format!("%{q}%")));
    }
    if let Some(pp) = project_path {
        sql.push_str(&format!(" AND project_path = ?{}", params.len() + 1));
        params.push(Box::new(pp.to_string()));
    }
    if let Some(sid) = session_id {
        sql.push_str(&format!(" AND session_id = ?{}", params.len() + 1));
        params.push(Box::new(sid.to_string()));
    }
    if let Some(uid) = user_id {
        sql.push_str(&format!(" AND user_id = ?{}", params.len() + 1));
        params.push(Box::new(uid.to_string()));
    }
    sql.push_str(" ORDER BY updated_at DESC LIMIT 50");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map(param_refs.as_slice(), |row| row_to_plan_meta(&conn, row))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
}

pub fn get_plan(db: &Db, id: &str) -> Option<Plan> {
    let conn = db.conn();
    let meta = conn
        .query_row("SELECT * FROM plans WHERE id = ?1", [id], |row| {
            row_to_plan_meta(&conn, row)
        })
        .ok()?;
    let items = get_plan_items(&conn, id);
    let versions = get_plan_versions(&conn, id);
    Some(Plan {
        meta,
        items,
        versions,
    })
}

pub fn session_has_unfinished_plan(db: &Db, session_id: &str) -> bool {
    list_plans(db, None, None, Some(session_id), None, false)
        .into_iter()
        .any(|plan| {
            plan.item_count > 0
                && !plan.finished
                && plan.done_count < plan.item_count
                && matches!(plan.approval_status.as_str(), "approved" | "not_required")
                && !matches!(
                    plan.execution_status.as_str(),
                    "paused" | "blocked" | "completed"
                )
        })
}

pub fn load_project_plan_context_for_session(db: &Db, session_id: &str) -> Option<String> {
    let conn = db.conn();
    let (session_user_id, project_path, active_plan_id): (Option<String>, String, Option<String>) =
        conn.query_row(
            "SELECT user_id, project_path, active_plan_id FROM sessions WHERE id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok()?;

    let user_id = session_user_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let project_path = project_path.trim().to_string();
    let mut sql = "SELECT id FROM plans
        WHERE (archived IS NULL OR archived = 0)
          AND approval_status IN ('approved', 'not_required')
          AND (session_id = ?1"
        .to_string();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    params.push(Box::new(session_id.to_string()));

    if let Some(user_id) = user_id.as_deref() {
        if !project_path.is_empty() {
            sql.push_str(&format!(
                " OR (user_id = ?{} AND project_path = ?{})",
                params.len() + 1,
                params.len() + 2
            ));
            params.push(Box::new(user_id.to_string()));
            params.push(Box::new(project_path.clone()));
        }
    }

    sql.push(')');
    if let Some(active_plan_id) = active_plan_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        params.push(Box::new(active_plan_id));
        sql.push_str(&format!(
            " ORDER BY CASE WHEN id = ?{} THEN 0 ELSE 1 END,",
            params.len()
        ));
    } else {
        sql.push_str(" ORDER BY");
    }
    sql.push_str(
        " CASE execution_status
            WHEN 'running' THEN 0
            WHEN 'blocked' THEN 1
            WHEN 'paused' THEN 2
            WHEN 'ready' THEN 3
            WHEN 'completed' THEN 4
            ELSE 5
        END, updated_at DESC
        LIMIT 3",
    );

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(|param| param.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).ok()?;
    let ids = stmt
        .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
        .ok()?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    drop(stmt);
    drop(conn);

    let summaries = ids
        .into_iter()
        .take(MAX_PLAN_CONTEXT_PLANS)
        .filter_map(|id| get_plan(db, &id))
        .map(|plan| format_plan_context_summary(&plan))
        .collect::<Vec<_>>();
    if summaries.is_empty() {
        None
    } else {
        Some(format!(
            "ACCEPTED PLAN CONTEXT - DURABLE PROJECT WORKFLOW STATE:\nThese accepted plan artifacts are attached to this session or project. Use them to discover and resume the current workflow, but update plan item state only when actively executing the plan with evidence.\n{}",
            summaries.join("\n")
        ))
    }
}

pub fn create_plan(
    db: &Db,
    title: &str,
    description: Option<&str>,
    items: &[serde_json::Value],
    session_id: Option<&str>,
    project_path: Option<&str>,
    user_id: &str,
) -> Option<Plan> {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();

    let id = uuid::Uuid::new_v4().to_string();
    if let Err(e) = conn.execute(
        "INSERT INTO plans (id, title, items, description, session_id, project_path, user_id, execution_status, approval_status, approval_requested_at, execution_updated_at, created_at, updated_at) VALUES (?1, ?2, '[]', ?3, ?4, ?5, ?6, 'ready', 'pending', ?7, ?7, ?7, ?7)",
        rusqlite::params![id, title, description, session_id, project_path, user_id, now],
    ) {
        tracing::error!("create_plan: INSERT failed: {e}");
        return None;
    }

    insert_plan_items(&conn, &id, items, &now, NewItemStatusMode::ForceTodo);
    if super::plan_quality::website_scope(title, description) {
        super::plan_quality::require(&conn, &id).ok()?;
    }
    if let Some(session_id) = session_id {
        conn.execute(
            "UPDATE sessions SET active_plan_id = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![id, now, session_id],
        )
        .ok();
    }
    take_version_snapshot(&conn, &id);
    drop(conn);
    let plan = get_plan(db, &id);
    if plan.is_none() {
        tracing::error!("create_plan: plan {} not found after INSERT", id);
    }
    plan
}

fn insert_plan_items(
    conn: &rusqlite::Connection,
    plan_id: &str,
    items: &[serde_json::Value],
    now: &str,
    status_mode: NewItemStatusMode,
) {
    let mut claimed_in_progress = false;
    for (i, item) in items.iter().enumerate() {
        let item_id = uuid::Uuid::new_v4().to_string();
        // Support both plain strings (["Do this", "Do that"]) and objects ({"text": "Do this"})
        let text = item
            .as_str()
            .or_else(|| item.get("text").and_then(|v| v.as_str()))
            .or_else(|| item.get("title").and_then(|v| v.as_str()))
            .unwrap_or("");
        let status = normalize_new_item_status(item, status_mode, &mut claimed_in_progress);
        let notes = item
            .get("notes")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("description").and_then(|v| v.as_str()));
        let section = item.get("section").and_then(|v| v.as_str());
        let reasoning_floor = normalize_reasoning_floor(
            item.get("reasoningFloor")
                .or_else(|| item.get("reasoning_floor"))
                .and_then(|v| v.as_str()),
        );
        conn.execute(
            "INSERT INTO plan_items (id, plan_id, text, status, notes, section, reasoning_floor, position, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            rusqlite::params![item_id, plan_id, text, status, notes, section, reasoning_floor, i as i64, now],
        )
        .ok();
    }
}

pub fn update_plan(
    db: &Db,
    id: &str,
    title: Option<&str>,
    description: Option<&str>,
    finished: Option<bool>,
    archived: Option<bool>,
) -> Option<PlanMeta> {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();

    let exists: bool = conn
        .query_row("SELECT COUNT(*) FROM plans WHERE id = ?1", [id], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_or(0)
        > 0;
    if !exists {
        return None;
    }

    if let Some(t) = title {
        conn.execute(
            "UPDATE plans SET title = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![t, now, id],
        )
        .ok();
    }
    if let Some(d) = description {
        conn.execute(
            "UPDATE plans SET description = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![d, now, id],
        )
        .ok();
    }
    if let Some(f) = finished {
        if f {
            let unfinished: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM plan_items WHERE plan_id=?1 AND status<>'done')",
                    [id],
                    |r| r.get(0),
                )
                .unwrap_or(true);
            if unfinished || !super::plan_quality::satisfied(&conn, id) {
                return None;
            }
            conn.execute(
                "UPDATE plans SET finished = 1, execution_status = 'completed', active_item_id = NULL, execution_updated_at = ?1, updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )
            .ok();
        } else {
            conn.execute(
                "UPDATE plans SET finished = 0, execution_status = 'ready', execution_updated_at = ?1, updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )
            .ok();
        }
    }
    if let Some(a) = archived {
        conn.execute(
            "UPDATE plans SET archived = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![if a { 1 } else { 0 }, now, id],
        )
        .ok();
        if a {
            conn.execute(
                "UPDATE sessions SET active_plan_id = NULL, updated_at = ?1 WHERE active_plan_id = ?2",
                rusqlite::params![now, id],
            )
            .ok();
        }
    }

    if title.is_some() || description.is_some() || archived.is_some() {
        take_version_snapshot(&conn, id);
    } else {
        sync_plan_artifact(&conn, id);
    }

    conn.query_row("SELECT * FROM plans WHERE id = ?1", [id], |row| {
        row_to_plan_meta(&conn, row)
    })
    .ok()
}

pub fn update_plan_execution(db: &Db, id: &str, action: &str) -> Option<Plan> {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();
    let rows = match action {
        "start" | "resume" => conn
            .execute(
                "UPDATE plans
                 SET approval_status = 'approved',
                     execution_status = 'running',
                     approved_at = COALESCE(approved_at, ?1),
                     execution_started_at = COALESCE(execution_started_at, ?1),
                     execution_updated_at = ?1,
                     finished = 0,
                     updated_at = ?1
                 WHERE id = ?2",
                rusqlite::params![now, id],
            )
            .ok()?,
        "pause" => conn
            .execute(
                "UPDATE plans
                 SET approval_status = 'approved',
                     execution_status = 'paused',
                     approved_at = COALESCE(approved_at, ?1),
                     execution_updated_at = ?1,
                     updated_at = ?1
                 WHERE id = ?2",
                rusqlite::params![now, id],
            )
            .ok()?,
        "block" => conn
            .execute(
                "UPDATE plans
                 SET approval_status = 'approved',
                     execution_status = 'blocked',
                     approved_at = COALESCE(approved_at, ?1),
                     execution_updated_at = ?1,
                     updated_at = ?1
                 WHERE id = ?2",
                rusqlite::params![now, id],
            )
            .ok()?,
        _ => return None,
    };
    if rows == 0 {
        return None;
    }
    if matches!(action, "start" | "resume") {
        conn.execute(
            "UPDATE sessions
             SET active_plan_id = ?1, updated_at = ?2
             WHERE id = (SELECT session_id FROM plans WHERE id = ?1)",
            rusqlite::params![id, now],
        )
        .ok();
    }
    sync_plan_artifact(&conn, id);
    drop(conn);
    get_plan(db, id)
}

pub fn delete_plan(db: &Db, id: &str) {
    let conn = db.conn();
    // Soft-delete: archive instead of hard delete so history is preserved
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE plans SET archived = 1, updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now, id],
    )
    .ok();
    conn.execute(
        "UPDATE sessions SET active_plan_id = NULL, updated_at = ?1 WHERE active_plan_id = ?2",
        rusqlite::params![now, id],
    )
    .ok();
    take_version_snapshot(&conn, id);
}

pub fn upsert_plan_item(
    db: &Db,
    plan_id: &str,
    item_id: &str,
    status: Option<&str>,
    notes: Option<&str>,
    text: Option<&str>,
    section: Option<&str>,
    reasoning_floor: Option<&str>,
) -> Result<PlanItem, PlanItemUpdateError> {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();

    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM plan_items WHERE id = ?1 AND plan_id = ?2",
            rusqlite::params![item_id, plan_id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !exists {
        return Err(PlanItemUpdateError::NotFound);
    }

    let reasoning_floor = reasoning_floor.map(|requested| {
        let requested = normalize_reasoning_floor(Some(requested));
        let current = conn
            .query_row(
                "SELECT reasoning_floor FROM plan_items WHERE id = ?1 AND plan_id = ?2",
                rusqlite::params![item_id, plan_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .unwrap_or_else(|| "medium".to_string());
        if reasoning_floor_rank(requested)
            < reasoning_floor_rank(normalize_reasoning_floor(Some(&current)))
        {
            normalize_reasoning_floor(Some(&current))
        } else {
            requested
        }
    });

    if let Some(s) = status {
        let normalized_status = normalize_update_status(s);
        if matches!(normalized_status, "in_progress" | "done")
            && plan_approval_status(&conn, plan_id) == "pending"
        {
            return Err(PlanItemUpdateError::ApprovalRequired);
        }
        if normalized_status == "done"
            && item_id == format!("browser-review-{plan_id}")
            && !super::plan_quality::satisfied(&conn, plan_id)
        {
            return Err(PlanItemUpdateError::BrowserReviewRequired);
        }
        if normalized_status == "in_progress" {
            clear_other_in_progress_items(&conn, plan_id, item_id, &now);
        }
        conn.execute(
            "UPDATE plan_items SET status = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![normalized_status, now, item_id],
        )
        .ok();
    }
    if let Some(n) = notes {
        conn.execute(
            "UPDATE plan_items SET notes = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![n, now, item_id],
        )
        .ok();
    }
    if let Some(s) = section {
        conn.execute(
            "UPDATE plan_items SET section = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![s, now, item_id],
        )
        .ok();
    }
    if let Some(floor) = reasoning_floor {
        conn.execute(
            "UPDATE plan_items SET reasoning_floor = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![floor, now, item_id],
        )
        .ok();
    }
    let document_changed =
        text.is_some() || notes.is_some() || section.is_some() || reasoning_floor.is_some();
    if let Some(t) = text {
        conn.execute(
            "UPDATE plan_items SET text = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![t, now, item_id],
        )
        .ok();
    }
    conn.execute(
        "UPDATE plans SET updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now, plan_id],
    )
    .ok();
    if document_changed {
        if status.is_some() {
            refresh_plan_execution_from_items(&conn, plan_id, &now);
        }
        take_version_snapshot(&conn, plan_id);
    } else {
        refresh_plan_execution_from_items(&conn, plan_id, &now);
        sync_plan_artifact(&conn, plan_id);
    }

    conn.query_row(
        "SELECT id, text, status, notes, position, section, reasoning_floor FROM plan_items WHERE id = ?1",
        [item_id],
        |row| {
            Ok(PlanItem {
                id: row.get(0)?,
                text: row.get(1)?,
                status: row.get(2)?,
                notes: row.get(3)?,
                position: row.get(4)?,
                section: row.get(5)?,
                reasoning_floor: normalize_reasoning_floor(
                    row.get::<_, Option<String>>(6)?.as_deref(),
                )
                .to_string(),
            })
        },
    )
    .map_err(|_| PlanItemUpdateError::NotFound)
}

pub fn delete_plan_item(db: &Db, plan_id: &str, item_id: &str) {
    if item_id == format!("browser-review-{plan_id}")
        && super::plan_quality::required(&db.conn(), plan_id)
    {
        return;
    }
    let conn = db.conn();
    conn.execute(
        "DELETE FROM plan_items WHERE id = ?1 AND plan_id = ?2",
        rusqlite::params![item_id, plan_id],
    )
    .ok();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE plans SET updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now, plan_id],
    )
    .ok();
    refresh_plan_execution_from_items(&conn, plan_id, &now);
    take_version_snapshot(&conn, plan_id);
}

pub fn add_plan_items(db: &Db, plan_id: &str, items: &[serde_json::Value]) -> Vec<PlanItem> {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();

    let max_pos: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), -1) FROM plan_items WHERE plan_id = ?1",
            [plan_id],
            |r| r.get(0),
        )
        .unwrap_or(-1);

    let mut new_items = Vec::new();
    let mut claimed_in_progress = false;
    for (i, item) in items.iter().enumerate() {
        let item_id = uuid::Uuid::new_v4().to_string();
        let text = item
            .as_str()
            .or_else(|| item.get("text").and_then(|v| v.as_str()))
            .or_else(|| item.get("title").and_then(|v| v.as_str()))
            .unwrap_or("");
        let status =
            normalize_new_item_status(item, NewItemStatusMode::ForceTodo, &mut claimed_in_progress);
        let notes = item
            .get("notes")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("description").and_then(|v| v.as_str()));
        let section = item.get("section").and_then(|v| v.as_str());
        let reasoning_floor = normalize_reasoning_floor(
            item.get("reasoningFloor")
                .or_else(|| item.get("reasoning_floor"))
                .and_then(|v| v.as_str()),
        );
        let pos = max_pos + 1 + i as i64;
        conn.execute(
            "INSERT INTO plan_items (id, plan_id, text, status, notes, section, reasoning_floor, position, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            rusqlite::params![item_id, plan_id, text, status, notes, section, reasoning_floor, pos, now],
        )
        .ok();
        new_items.push(PlanItem {
            id: item_id,
            text: text.to_string(),
            status: status.to_string(),
            notes: notes.map(|s| s.to_string()),
            section: section.map(|s| s.to_string()),
            reasoning_floor: reasoning_floor.to_string(),
            position: pos,
        });
    }

    conn.execute(
        "UPDATE plans SET updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now, plan_id],
    )
    .ok();
    super::plan_quality::invalidate(&conn, plan_id);
    refresh_plan_execution_from_items(&conn, plan_id, &now);
    take_version_snapshot(&conn, plan_id);
    new_items
}

pub fn replace_plan_items(db: &Db, plan_id: &str, items: &[serde_json::Value]) -> Option<Plan> {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();

    let exists: bool = conn
        .query_row("SELECT COUNT(*) FROM plans WHERE id = ?1", [plan_id], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_or(0)
        > 0;
    if !exists {
        return None;
    }

    conn.execute("DELETE FROM plan_items WHERE plan_id = ?1", [plan_id])
        .ok();
    let status_mode = if plan_approval_status(&conn, plan_id) == "pending" {
        NewItemStatusMode::ForceTodo
    } else {
        NewItemStatusMode::PreserveAtMostOneInProgress
    };
    insert_plan_items(&conn, plan_id, items, &now, status_mode);
    super::plan_quality::invalidate(&conn, plan_id);
    super::plan_quality::ensure_item(&conn, plan_id).ok()?;
    conn.execute(
        "UPDATE plans SET updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now, plan_id],
    )
    .ok();
    refresh_plan_execution_from_items(&conn, plan_id, &now);
    take_version_snapshot(&conn, plan_id);

    drop(conn);
    get_plan(db, plan_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sessions::{UpsertSession, upsert_session};
    use serde_json::json;

    fn seed_session(db: &Db, session_id: &str, project_path: &str, user_id: &str) {
        upsert_session(
            db,
            &UpsertSession {
                id: session_id.to_string(),
                claude_session_id: None,
                project_path: project_path.to_string(),
                title: Some("Test Session".to_string()),
                message_count: Some(0),
                pinned: Some(false),
                model: Some("claude-sonnet-4-6".to_string()),
                input_tokens: Some(0),
                output_tokens: Some(0),
                provider: Some("claude".to_string()),
                user_id: Some(user_id.to_string()),
                runner_id: None,
            },
        );
    }

    #[test]
    fn accepted_project_plan_context_attaches_to_future_sessions() {
        let db = Db::open(":memory:");
        seed_session(&db, "source-session", "/tmp/project", "user-1");
        seed_session(&db, "future-session", "/tmp/project", "user-1");

        let plan = create_plan(
            &db,
            "Ship durable plan context",
            Some("Keep the accepted plan discoverable without dumping every note."),
            &[
                json!({"text": "Wire plan artifacts into project context", "section": "Planning"}),
                json!({"text": "Add a second bounded item"}),
                json!({"text": "Add a third bounded item"}),
                json!({"text": "Add a fourth bounded item"}),
                json!({"text": "Add a fifth bounded item"}),
                json!({"text": "This sixth item should not appear in compact context"}),
            ],
            Some("source-session"),
            Some("/tmp/project"),
            "user-1",
        )
        .expect("plan created");
        let pending = create_plan(
            &db,
            "Pending plan should stay hidden",
            None,
            &[json!({"text": "not approved"})],
            Some("source-session"),
            Some("/tmp/project"),
            "user-1",
        )
        .expect("pending plan created");

        update_plan_execution(&db, &plan.meta.id, "start").expect("plan approved");
        upsert_plan_item(
            &db,
            &plan.meta.id,
            &plan.items[0].id,
            Some("in_progress"),
            None,
            None,
            None,
            None,
        )
        .expect("item updated");

        let context = load_project_plan_context_for_session(&db, "future-session")
            .expect("future session plan context");
        assert!(context.contains("ACCEPTED PLAN CONTEXT"));
        assert!(context.contains("Ship durable plan context"));
        assert!(context.contains(&plan.meta.id));
        assert!(context.contains(
            "Active item: Planning: [in_progress, Auto-only reasoning floor=medium] Wire plan artifacts"
        ));
        assert!(!context.contains(&pending.meta.id));
        assert!(!context.contains("This sixth item should not appear"));
    }

    #[test]
    fn plan_context_prioritizes_the_sessions_explicit_active_plan() {
        let db = Db::open(":memory:");
        seed_session(&db, "session-1", "/tmp/project", "user-1");
        let selected = create_plan(
            &db,
            "Selected active plan",
            None,
            &[json!({"text": "Selected active item"})],
            Some("session-1"),
            Some("/tmp/project"),
            "user-1",
        )
        .expect("selected plan created");
        update_plan_execution(&db, &selected.meta.id, "start").expect("selected plan started");
        let newer = create_plan(
            &db,
            "Newer project plan",
            None,
            &[json!({"text": "Newer active item"})],
            Some("session-1"),
            Some("/tmp/project"),
            "user-1",
        )
        .expect("newer plan created");
        update_plan_execution(&db, &newer.meta.id, "start").expect("newer plan started");
        db.conn()
            .execute(
                "UPDATE sessions SET active_plan_id = ?1 WHERE id = ?2",
                rusqlite::params![selected.meta.id, "session-1"],
            )
            .expect("active plan selected");

        let context =
            load_project_plan_context_for_session(&db, "session-1").expect("active plan context");
        let selected_position = context.find("Selected active plan").unwrap();
        let newer_position = context.find("Newer project plan").unwrap();

        assert!(selected_position < newer_position);
    }

    #[test]
    fn plan_artifact_metadata_autosyncs_bounded_context_fields() {
        let db = Db::open(":memory:");
        seed_session(&db, "session-1", "/tmp/project", "user-1");
        let plan = create_plan(
            &db,
            "Metadata plan",
            None,
            &[
                json!({"text": "First item", "notes": "Long notes should not flood artifact context"}),
                json!({"text": "Second item"}),
            ],
            Some("session-1"),
            Some("/tmp/project"),
            "user-1",
        )
        .expect("plan created");

        update_plan_execution(&db, &plan.meta.id, "start").expect("plan started");
        let loaded = get_plan(&db, &plan.meta.id).expect("plan loaded");
        let metadata = loaded.meta.artifact.expect("artifact synced").metadata;
        assert_eq!(metadata["autosync"], "accepted_plan_context");
        assert_eq!(metadata["progress"]["itemCount"], 2);
        assert_eq!(metadata["progress"]["doneCount"], 0);
        assert_eq!(metadata["nextItems"].as_array().unwrap().len(), 2);
        assert!(metadata["nextItems"].to_string().contains("First item"));
        assert!(!metadata["nextItems"].to_string().contains("Long notes"));
    }

    #[test]
    fn plan_lifecycle_owns_the_linked_session_selection() {
        let db = Db::open(":memory:");
        seed_session(&db, "session-plan", "/tmp/project", "user-1");

        let first = create_plan(
            &db,
            "First plan",
            None,
            &[json!({"text": "A"})],
            Some("session-plan"),
            Some("/tmp/project"),
            "user-1",
        )
        .expect("first plan created");
        assert_eq!(
            crate::db::sessions::get_session(&db, "session-plan")
                .unwrap()
                .active_plan_id
                .as_deref(),
            Some(first.meta.id.as_str())
        );

        let second = create_plan(
            &db,
            "Second plan",
            None,
            &[json!({"text": "B"})],
            Some("session-plan"),
            Some("/tmp/project"),
            "user-1",
        )
        .expect("second plan created");
        assert_eq!(
            crate::db::sessions::get_session(&db, "session-plan")
                .unwrap()
                .active_plan_id
                .as_deref(),
            Some(second.meta.id.as_str())
        );

        delete_plan(&db, &second.meta.id);
        assert_eq!(
            crate::db::sessions::get_session(&db, "session-plan")
                .unwrap()
                .active_plan_id,
            None
        );
    }

    #[test]
    fn create_plan_forces_initial_items_to_todo() {
        let db = Db::open(":memory:");
        let plan = create_plan(
            &db,
            "Plan",
            None,
            &[
                json!({"text": "A", "status": "in_progress"}),
                json!({"text": "B", "status": "done"}),
            ],
            Some("session-1"),
            None,
            "user-1",
        )
        .expect("plan created");

        assert_eq!(
            plan.items
                .iter()
                .map(|item| item.status.as_str())
                .collect::<Vec<_>>(),
            vec!["todo", "todo"]
        );
        assert_eq!(plan.meta.execution_status, "ready");
        assert_eq!(plan.meta.approval_status, "pending");
        assert_eq!(plan.meta.active_item_id, None);
    }

    #[test]
    fn pending_plan_requires_approval_before_execution_updates() {
        let db = Db::open(":memory:");
        let plan = create_plan(
            &db,
            "Plan",
            None,
            &[json!({"text": "A"})],
            Some("session-1"),
            None,
            "user-1",
        )
        .expect("plan created");

        let result = upsert_plan_item(
            &db,
            &plan.meta.id,
            &plan.items[0].id,
            Some("in_progress"),
            None,
            None,
            None,
            None,
        );
        assert!(matches!(result, Err(PlanItemUpdateError::ApprovalRequired)));

        let started = update_plan_execution(&db, &plan.meta.id, "start").expect("plan started");
        assert_eq!(started.meta.execution_status, "running");
        assert_eq!(started.meta.approval_status, "approved");

        upsert_plan_item(
            &db,
            &plan.meta.id,
            &plan.items[0].id,
            Some("in_progress"),
            None,
            None,
            None,
            None,
        )
        .expect("item updated after start");
    }

    #[test]
    fn setting_item_in_progress_clears_previous_in_progress_item() {
        let db = Db::open(":memory:");
        let plan = create_plan(
            &db,
            "Plan",
            None,
            &[json!({"text": "A"}), json!({"text": "B"})],
            Some("session-1"),
            None,
            "user-1",
        )
        .expect("plan created");
        let first_id = plan.items[0].id.clone();
        let second_id = plan.items[1].id.clone();
        update_plan_execution(&db, &plan.meta.id, "start").expect("plan started");

        upsert_plan_item(
            &db,
            &plan.meta.id,
            &first_id,
            Some("in_progress"),
            None,
            None,
            None,
            None,
        )
        .expect("first item updated");
        upsert_plan_item(
            &db,
            &plan.meta.id,
            &second_id,
            Some("in_progress"),
            None,
            None,
            None,
            None,
        )
        .expect("second item updated");

        let updated = get_plan(&db, &plan.meta.id).expect("plan loaded");
        assert_eq!(updated.items[0].status, "todo");
        assert_eq!(updated.items[1].status, "in_progress");
        assert_eq!(updated.meta.execution_status, "running");
        assert_eq!(
            updated.meta.active_item_id.as_deref(),
            Some(second_id.as_str())
        );
    }

    #[test]
    fn replace_plan_items_allows_only_one_in_progress_item() {
        let db = Db::open(":memory:");
        let plan = create_plan(
            &db,
            "Plan",
            None,
            &[json!({"text": "A"})],
            Some("session-1"),
            None,
            "user-1",
        )
        .expect("plan created");
        update_plan_execution(&db, &plan.meta.id, "start").expect("plan started");

        let replaced = replace_plan_items(
            &db,
            &plan.meta.id,
            &[
                json!({"text": "A", "status": "in_progress"}),
                json!({"text": "B", "status": "in_progress"}),
                json!({"text": "C", "status": "done"}),
            ],
        )
        .expect("plan replaced");

        assert_eq!(
            replaced
                .items
                .iter()
                .map(|item| item.status.as_str())
                .collect::<Vec<_>>(),
            vec!["in_progress", "todo", "done"]
        );
    }

    #[test]
    fn create_plan_returns_artifact_and_structured_version() {
        let db = Db::open(":memory:");
        let plan = create_plan(
            &db,
            "Plan",
            Some("Description"),
            &[json!({"text": "A", "notes": "Detail", "section": "Phase"})],
            Some("session-1"),
            Some("/workspace"),
            "user-1",
        )
        .expect("plan created");

        let artifact = plan.meta.artifact.as_ref().expect("artifact returned");
        assert_eq!(artifact.plan_id, plan.meta.id);
        assert_eq!(artifact.kind, "plan");
        assert_eq!(artifact.schema_version, PLAN_ARTIFACT_SCHEMA_VERSION);
        assert_eq!(artifact.session_id.as_deref(), Some("session-1"));
        assert_eq!(artifact.project_path.as_deref(), Some("/workspace"));
        assert_eq!(plan.meta.version_count, 1);
        assert_eq!(plan.versions.len(), 1);
        assert_eq!(
            plan.versions[0].schema_version,
            PLAN_ARTIFACT_SCHEMA_VERSION
        );
        assert_eq!(plan.versions[0].title.as_deref(), Some("Plan"));
        assert_eq!(plan.versions[0].description.as_deref(), Some("Description"));
        assert_eq!(plan.versions[0].items.len(), 1);
        assert_eq!(
            plan.versions[0].artifact.as_ref().unwrap().plan_id,
            plan.meta.id
        );
    }

    #[test]
    fn status_only_item_updates_do_not_create_versions() {
        let db = Db::open(":memory:");
        let plan = create_plan(
            &db,
            "Plan",
            None,
            &[json!({"text": "A"})],
            Some("session-1"),
            None,
            "user-1",
        )
        .expect("plan created");
        let initial_version_count = plan.meta.version_count;
        update_plan_execution(&db, &plan.meta.id, "start").expect("plan started");

        upsert_plan_item(
            &db,
            &plan.meta.id,
            &plan.items[0].id,
            Some("done"),
            None,
            None,
            None,
            None,
        )
        .expect("item updated");

        let updated = get_plan(&db, &plan.meta.id).expect("plan loaded");
        assert_eq!(updated.items[0].status, "done");
        assert_eq!(updated.meta.execution_status, "completed");
        assert!(updated.meta.finished);
        assert_eq!(updated.meta.version_count, initial_version_count);
        assert_eq!(updated.versions.len(), initial_version_count as usize);
    }

    #[test]
    fn planner_reasoning_floor_is_persisted_and_only_escalates() {
        let db = Db::open(":memory:");
        let plan = create_plan(
            &db,
            "Plan",
            None,
            &[json!({"text": "Sensitive change", "reasoningFloor": "high"})],
            Some("session-1"),
            None,
            "user-1",
        )
        .expect("plan created");
        let item_id = plan.items[0].id.clone();
        assert_eq!(plan.items[0].reasoning_floor, "high");
        assert_eq!(
            active_plan_reasoning_floor(&db, &plan.meta.id).as_deref(),
            Some("high")
        );

        update_plan_execution(&db, &plan.meta.id, "start").expect("plan started");
        let unchanged = upsert_plan_item(
            &db,
            &plan.meta.id,
            &item_id,
            None,
            None,
            None,
            None,
            Some("low"),
        )
        .expect("lower request retained existing floor");
        assert_eq!(unchanged.reasoning_floor, "high");

        let escalated = upsert_plan_item(
            &db,
            &plan.meta.id,
            &item_id,
            None,
            None,
            None,
            None,
            Some("xhigh"),
        )
        .expect("floor escalated");
        assert_eq!(escalated.reasoning_floor, "xhigh");
    }
}
