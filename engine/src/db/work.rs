use crate::db::Db;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkItem {
    pub id: String,
    pub user_id: String,
    pub project_path: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub priority: String,
    pub assignee_kind: Option<String>,
    pub assignee_id: Option<String>,
    pub milestone: Option<String>,
    pub release_status: Option<String>,
    pub plan_id: Option<String>,
    pub orchestration_run_id: Option<String>,
    pub labels: Vec<String>,
    pub position: i64,
    pub created_at: String,
    pub updated_at: String,
}

fn parse_json(raw: String) -> Value {
    serde_json::from_str(&raw).unwrap_or_else(|_| json!([]))
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItem> {
    Ok(WorkItem {
        id: row.get(0)?,
        user_id: row.get(1)?,
        project_path: row.get(2)?,
        parent_id: row.get(3)?,
        kind: row.get(4)?,
        title: row.get(5)?,
        description: row.get(6)?,
        status: row.get(7)?,
        priority: row.get(8)?,
        assignee_kind: row.get(9)?,
        assignee_id: row.get(10)?,
        milestone: row.get(11)?,
        release_status: row.get(12)?,
        plan_id: row.get(13)?,
        orchestration_run_id: row.get(14)?,
        labels: serde_json::from_str(&row.get::<_, String>(15)?).unwrap_or_default(),
        position: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
    })
}

const COLUMNS: &str = "id,user_id,project_path,parent_id,kind,title,description,status,priority,
assignee_kind,assignee_id,milestone,release_status,plan_id,orchestration_run_id,labels_json,
position,created_at,updated_at";

fn validate_choice(value: &str, allowed: &[&str], field: &str) -> Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!("Invalid {field}"))
    }
}

fn validate_item(
    kind: &str,
    status: &str,
    priority: &str,
    title: &str,
    description: &str,
) -> Result<(), String> {
    validate_choice(
        kind,
        &["task", "issue", "story", "epic", "review"],
        "work item kind",
    )?;
    validate_choice(
        status,
        &["backlog", "todo", "in_progress", "review", "done"],
        "work item status",
    )?;
    validate_choice(
        priority,
        &["low", "normal", "high", "urgent"],
        "work item priority",
    )?;
    if title.is_empty() || title.chars().count() > 300 {
        return Err("Title must contain 1-300 characters".into());
    }
    if description.chars().count() > 50_000 {
        return Err("Description cannot exceed 50,000 characters".into());
    }
    Ok(())
}

pub fn list(db: &Db, user_id: &str, project_path: &str, query: Option<&str>) -> Vec<WorkItem> {
    let conn = db.conn();
    let mut sql = format!("SELECT {COLUMNS} FROM work_items WHERE user_id=?1 AND project_path=?2");
    if query.is_some() {
        sql.push_str(" AND (title LIKE ?3 OR description LIKE ?3 OR labels_json LIKE ?3)");
    }
    sql.push_str(" ORDER BY CASE status WHEN 'in_progress' THEN 0 WHEN 'review' THEN 1 WHEN 'todo' THEN 2 WHEN 'backlog' THEN 3 ELSE 4 END, position, updated_at DESC");
    let mut stmt = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(_) => return vec![],
    };
    let rows = if let Some(query) = query {
        stmt.query_map(
            rusqlite::params![user_id, project_path, format!("%{query}%")],
            row_to_item,
        )
    } else {
        stmt.query_map(rusqlite::params![user_id, project_path], row_to_item)
    };
    rows.map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

pub fn get(db: &Db, user_id: &str, id: &str) -> Option<WorkItem> {
    db.conn()
        .query_row(
            &format!("SELECT {COLUMNS} FROM work_items WHERE id=?1 AND user_id=?2"),
            rusqlite::params![id, user_id],
            row_to_item,
        )
        .ok()
}

pub fn create(db: &Db, user_id: &str, body: &Value) -> Result<WorkItem, String> {
    let project_path = body
        .get("projectPath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let title = body
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if project_path.is_empty() || title.is_empty() {
        return Err("Project and title are required".into());
    }
    let kind = body.get("kind").and_then(Value::as_str).unwrap_or("task");
    let status = body
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("backlog");
    let priority = body
        .get("priority")
        .and_then(Value::as_str)
        .unwrap_or("normal");
    let description = body
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    validate_item(kind, status, priority, title, description)?;
    if let Some(parent_id) = body.get("parentId").and_then(Value::as_str)
        && get(db, user_id, parent_id).is_none()
    {
        return Err("Parent work item not found".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let labels = body.get("labels").cloned().unwrap_or_else(|| json!([]));
    db.conn().execute(
        "INSERT INTO work_items (id,user_id,project_path,parent_id,kind,title,description,status,
         priority,assignee_kind,assignee_id,milestone,release_status,plan_id,orchestration_run_id,
         labels_json,position,created_at,updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?18)",
        rusqlite::params![id,user_id,project_path,body.get("parentId").and_then(Value::as_str),
            kind,title,description,status,priority,
            body.get("assigneeKind").and_then(Value::as_str),body.get("assigneeId").and_then(Value::as_str),
            body.get("milestone").and_then(Value::as_str),body.get("releaseStatus").and_then(Value::as_str),
            body.get("planId").and_then(Value::as_str),body.get("orchestrationRunId").and_then(Value::as_str),
            labels.to_string(),body.get("position").and_then(Value::as_i64).unwrap_or(0),now],
    ).map_err(|e| e.to_string())?;
    get(db, user_id, &id).ok_or_else(|| "Could not create work item".into())
}

pub fn update(db: &Db, user_id: &str, id: &str, body: &Value) -> Result<WorkItem, String> {
    let current = get(db, user_id, id).ok_or("Work item not found")?;
    let text = |key: &str, current: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .unwrap_or(current)
            .to_string()
    };
    let optional = |key: &str, current: Option<String>| {
        body.get(key)
            .map(|v| v.as_str().map(str::to_string))
            .unwrap_or(current)
    };
    let labels = body
        .get("labels")
        .cloned()
        .unwrap_or_else(|| json!(current.labels));
    let kind = text("kind", &current.kind);
    let title = text("title", &current.title);
    let description = text("description", &current.description);
    let status = text("status", &current.status);
    let priority = text("priority", &current.priority);
    validate_item(&kind, &status, &priority, &title, &description)?;
    if let Some(parent_id) = body.get("parentId").and_then(Value::as_str)
        && (parent_id == id || get(db, user_id, parent_id).is_none())
    {
        return Err("Parent work item is invalid".into());
    }
    let now = chrono::Utc::now().to_rfc3339();
    db.conn().execute(
        "UPDATE work_items SET parent_id=?1,kind=?2,title=?3,description=?4,status=?5,priority=?6,
         assignee_kind=?7,assignee_id=?8,milestone=?9,release_status=?10,plan_id=?11,
         orchestration_run_id=?12,labels_json=?13,position=?14,updated_at=?15 WHERE id=?16 AND user_id=?17",
        rusqlite::params![optional("parentId",current.parent_id),kind,title,
            description,status,priority,
            optional("assigneeKind",current.assignee_kind),optional("assigneeId",current.assignee_id),
            optional("milestone",current.milestone),optional("releaseStatus",current.release_status),
            optional("planId",current.plan_id),optional("orchestrationRunId",current.orchestration_run_id),
            labels.to_string(),body.get("position").and_then(Value::as_i64).unwrap_or(current.position),now,id,user_id],
    ).map_err(|e| e.to_string())?;
    get(db, user_id, id).ok_or_else(|| "Work item not found".into())
}

pub fn delete(db: &Db, user_id: &str, id: &str) -> bool {
    db.conn()
        .execute(
            "DELETE FROM work_items WHERE id=?1 AND user_id=?2",
            rusqlite::params![id, user_id],
        )
        .map(|rows| rows > 0)
        .unwrap_or(false)
}

pub fn add_comment(db: &Db, user_id: &str, id: &str, body: &str) -> Result<Value, String> {
    if get(db, user_id, id).is_none() {
        return Err("Work item not found".into());
    }
    let body = body.trim();
    if body.is_empty() || body.chars().count() > 20_000 {
        return Err("Comment must contain 1-20,000 characters".into());
    }
    let comment_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    db.conn().execute("INSERT INTO work_comments (id,work_item_id,user_id,body,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?5)",
        rusqlite::params![comment_id,id,user_id,body,now]).map_err(|e|e.to_string())?;
    Ok(
        json!({"id":comment_id,"workItemId":id,"userId":user_id,"body":body,"createdAt":now,"updatedAt":now}),
    )
}

pub fn detail(db: &Db, user_id: &str, id: &str) -> Option<Value> {
    let item = get(db, user_id, id)?;
    let conn = db.conn();
    let comments = conn.prepare("SELECT id,user_id,body,created_at,updated_at FROM work_comments WHERE work_item_id=?1 ORDER BY created_at")
        .ok().and_then(|mut stmt| stmt.query_map([id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"userId":r.get::<_,String>(1)?,"body":r.get::<_,String>(2)?,"createdAt":r.get::<_,String>(3)?,"updatedAt":r.get::<_,String>(4)?}))).ok()
            .map(|rows|rows.filter_map(Result::ok).collect::<Vec<_>>())).unwrap_or_default();
    let dependencies = conn
        .prepare("SELECT depends_on_id FROM work_dependencies WHERE work_item_id=?1")
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([id], |r| r.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>())
        })
        .unwrap_or_default();
    let artifacts = conn.prepare("SELECT id,kind,title,path,url,metadata_json,created_at FROM work_artifacts WHERE work_item_id=?1 ORDER BY created_at").ok()
        .and_then(|mut stmt|stmt.query_map([id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"path":r.get::<_,Option<String>>(3)?,"url":r.get::<_,Option<String>>(4)?,"metadata":parse_json(r.get(5)?),"createdAt":r.get::<_,String>(6)?}))).ok()
            .map(|rows|rows.filter_map(Result::ok).collect::<Vec<_>>())).unwrap_or_default();
    Some(json!({"item":item,"comments":comments,"dependencies":dependencies,"artifacts":artifacts}))
}

pub fn add_dependency(db: &Db, user_id: &str, id: &str, depends_on: &str) -> Result<(), String> {
    if id == depends_on {
        return Err("A work item cannot depend on itself".into());
    }
    if get(db, user_id, id).is_none() || get(db, user_id, depends_on).is_none() {
        return Err("Work item not found".into());
    }
    let creates_cycle = db
        .conn()
        .query_row(
            "WITH RECURSIVE chain(id) AS (
                SELECT depends_on_id FROM work_dependencies WHERE work_item_id=?1
                UNION
                SELECT d.depends_on_id FROM work_dependencies d JOIN chain c ON d.work_item_id=c.id
             ) SELECT EXISTS(SELECT 1 FROM chain WHERE id=?2)",
            rusqlite::params![depends_on, id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        == 1;
    if creates_cycle {
        return Err("Dependency would create a cycle".into());
    }
    db.conn().execute("INSERT OR IGNORE INTO work_dependencies (work_item_id,depends_on_id,created_at) VALUES (?1,?2,?3)",
        rusqlite::params![id,depends_on,chrono::Utc::now().to_rfc3339()]).map_err(|e|e.to_string())?;
    Ok(())
}

pub fn add_artifact(db: &Db, user_id: &str, id: &str, body: &Value) -> Result<Value, String> {
    if get(db, user_id, id).is_none() {
        return Err("Work item not found".into());
    }
    let artifact_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let title = body
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Artifact");
    db.conn().execute("INSERT INTO work_artifacts (id,work_item_id,kind,title,path,url,metadata_json,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        rusqlite::params![artifact_id,id,body.get("kind").and_then(Value::as_str).unwrap_or("file"),title,
            body.get("path").and_then(Value::as_str),body.get("url").and_then(Value::as_str),
            body.get("metadata").cloned().unwrap_or_else(||json!({})).to_string(),now]).map_err(|e|e.to_string())?;
    Ok(json!({"id":artifact_id,"workItemId":id,"title":title,"createdAt":now}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_item_lifecycle_preserves_links_and_evidence() {
        let db = Db::open(":memory:");
        let dependency = create(
            &db,
            "user-1",
            &json!({"projectPath":"/repo","title":"Foundation"}),
        )
        .unwrap();
        let item = create(
            &db,
            "user-1",
            &json!({
                "projectPath":"/repo","title":"Ship capability hub","priority":"high",
                "assigneeKind":"agent","assigneeId":"release-engineer","planId":"plan-1"
            }),
        )
        .unwrap();
        let updated = update(
            &db,
            "user-1",
            &item.id,
            &json!({
                "status":"review","releaseStatus":"candidate","orchestrationRunId":"run-1"
            }),
        )
        .unwrap();
        assert_eq!(updated.status, "review");
        assert_eq!(updated.plan_id.as_deref(), Some("plan-1"));
        add_comment(
            &db,
            "user-1",
            &item.id,
            "Validated on local and server modes",
        )
        .unwrap();
        add_dependency(&db, "user-1", &item.id, &dependency.id).unwrap();
        add_artifact(
            &db,
            "user-1",
            &item.id,
            &json!({"kind":"test","title":"Verification report","path":"report.json"}),
        )
        .unwrap();
        let detail = detail(&db, "user-1", &item.id).unwrap();
        assert_eq!(detail["comments"].as_array().unwrap().len(), 1);
        assert_eq!(detail["dependencies"][0], dependency.id);
        assert_eq!(detail["artifacts"][0]["title"], "Verification report");
        assert!(get(&db, "user-2", &item.id).is_none());
    }
}
