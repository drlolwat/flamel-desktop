//! Website acceptance is backed by persisted browser tool receipts.
use super::Db;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS plan_browser_reviews (
        plan_id TEXT PRIMARY KEY REFERENCES plans(id) ON DELETE CASCADE,
        created_at TEXT NOT NULL, evidence_json TEXT
    );",
    )
}

pub fn website_scope(title: &str, description: Option<&str>) -> bool {
    let text = format!("{title} {}", description.unwrap_or("")).to_lowercase();
    ["website", "storefront", "saas", "web app", "landing page"]
        .iter()
        .any(|term| text.contains(term))
}

pub fn required(conn: &Connection, id: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM plan_browser_reviews WHERE plan_id=?1)",
        [id],
        |r| r.get(0),
    )
    .unwrap_or(false)
}
pub fn satisfied(conn: &Connection, id: &str) -> bool {
    conn.query_row("SELECT NOT EXISTS(SELECT 1 FROM plan_browser_reviews WHERE plan_id=?1 AND evidence_json IS NULL)",[id],|r|r.get(0)).unwrap_or(false)
}
pub fn require(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("INSERT OR IGNORE INTO plan_browser_reviews(plan_id,created_at) SELECT id,?2 FROM plans WHERE id=?1",params![id,chrono::Utc::now().to_rfc3339()])?;
    ensure_item(conn, id)?;
    conn.execute("UPDATE plans SET finished=0,execution_status=CASE WHEN execution_status='completed' THEN 'running' ELSE execution_status END WHERE id=?1",[id])?;
    Ok(())
}
pub fn ensure_item(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    if !required(conn, id) {
        return Ok(());
    }
    conn.execute("INSERT OR IGNORE INTO plan_items(id,plan_id,text,status,notes,section,reasoning_floor,position,created_at,updated_at)
        VALUES(?1,?2,'Review the working website in desktop and mobile browsers',?3,?4,'Acceptance','high',
        COALESCE((SELECT MAX(position)+1 FROM plan_items WHERE plan_id=?2),0),?5,?5)", params![
        format!("browser-review-{id}"),id,if satisfied(conn,id){"done"}else{"todo"},
        "Mandatory browser review. Record desktop and mobile screenshot tool IDs, a successful functional journey tool ID, tested revision, URLs/states and visual findings using plan(action=browser_review). For SaaS verify sign-in, authenticated actions and persisted results after reload; signed-out access must lead into the real account flow. Build success alone cannot complete this item.",chrono::Utc::now().to_rfc3339()])?;
    Ok(())
}
pub fn invalidate(conn: &Connection, id: &str) {
    let _ = conn.execute(
        "UPDATE plan_browser_reviews SET evidence_json=NULL,created_at=?2 WHERE plan_id=?1",
        params![id, chrono::Utc::now().to_rfc3339()],
    );
    let _ = conn.execute(
        "UPDATE plan_items SET status='todo' WHERE id=?1",
        [format!("browser-review-{id}")],
    );
}
pub fn status(db: &Db, id: &str) -> Value {
    let evidence = db
        .conn()
        .query_row(
            "SELECT evidence_json FROM plan_browser_reviews WHERE plan_id=?1",
            [id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .ok()
        .flatten();
    match evidence {
        Some(e) => {
            json!({"required":true,"evidence":e.and_then(|v|serde_json::from_str::<Value>(&v).ok()),"availableReceipts":available_receipts(db,id)})
        }
        None => json!({"required":false}),
    }
}

/// Providers do not necessarily see transport tool-call IDs. Return real successful
/// receipts with enough context to select captures, without exposing image payloads.
fn available_receipts(db: &Db, id: &str) -> Vec<Value> {
    let conn = db.conn();
    let Ok(mut stmt) = conn.prepare("SELECT m.message FROM session_messages m JOIN sessions s ON s.id=m.session_id JOIN plans p ON p.user_id=s.user_id AND (p.session_id=s.id OR s.active_plan_id=p.id) JOIN plan_browser_reviews q ON q.plan_id=p.id WHERE p.id=?1 AND m.created_at>=q.created_at ORDER BY m.id DESC LIMIT 2048") else { return vec![]; };
    let Ok(rows) = stmt.query_map([id], |r| r.get::<_, String>(0)) else {
        return vec![];
    };
    let mut evidence = Vec::new();
    for raw in rows.flatten() {
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            blocks(&value, &mut evidence);
        }
    }
    let mut seen = std::collections::HashSet::new();
    evidence.iter().filter_map(|call| {
        let name = call["name"].as_str().unwrap_or("");
        let tool_id = call["id"].as_str()?;
        if call["type"] != "tool_use" || !(name.ends_with("browser_inspect") || name.ends_with("browser_run") || name.ends_with("shell")) || !seen.insert(tool_id) { return None; }
        let receipt = evidence.iter().find(|r| r["type"]=="tool_result" && r["tool_use_id"]==tool_id && r["is_error"]!=true && !r["content"].is_null())?;
        Some(json!({"toolId":tool_id,"tool":name,"input":call["input"].to_string().chars().take(1200).collect::<String>(),"resultSummary":receipt["content"].to_string().chars().take(1800).collect::<String>()}))
    }).take(40).collect()
}

fn blocks(value: &Value, out: &mut Vec<Value>) {
    if let Some(a) = value.as_array() {
        for v in a {
            blocks(v, out);
        }
    } else if matches!(value["type"].as_str(), Some("tool_use" | "tool_result")) {
        out.push(value.clone());
    } else {
        if let Some(v) = value.get("content") {
            blocks(v, out);
        }
        if let Some(v) = value.get("message") {
            blocks(v, out);
        }
    }
}

/// Accept existing receipts only; supplied prose cannot manufacture tool success.
pub fn record(db: &Db, id: &str, review: &Value) -> Result<(), String> {
    for field in ["revision", "journeys", "visualFindings"] {
        if review[field].as_str().is_none_or(|v| v.trim().len() < 8) {
            return Err(format!("browser review requires substantive {field}"));
        }
    }
    let desktop = review["desktopToolId"]
        .as_str()
        .ok_or("desktopToolId required")?;
    let mobile = review["mobileToolId"]
        .as_str()
        .ok_or("mobileToolId required")?;
    let functional = review["functionalToolId"]
        .as_str()
        .ok_or("functionalToolId required")?;
    if desktop == mobile {
        return Err("desktop and mobile need separate screenshot receipts".into());
    }
    let conn = db.conn();
    let review_session = review["sessionId"].as_str().unwrap_or("");
    let mut stmt=conn.prepare("SELECT m.message FROM session_messages m JOIN sessions s ON s.id=m.session_id
        JOIN plans p ON p.user_id=s.user_id AND (p.session_id=s.id OR (s.id=?2 AND s.active_plan_id=p.id))
        JOIN plan_browser_reviews q ON q.plan_id=p.id WHERE p.id=?1 AND m.created_at>=q.created_at
        ORDER BY m.id DESC LIMIT 2048").map_err(|e|e.to_string())?;
    let mut evidence = Vec::new();
    for raw in stmt
        .query_map(params![id, review_session], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
    {
        if let Ok(value) = serde_json::from_str::<Value>(&raw.map_err(|e| e.to_string())?) {
            blocks(&value, &mut evidence);
        }
    }
    for (tool_id, screenshot) in [(desktop, true), (mobile, true), (functional, false)] {
        let call = evidence
            .iter()
            .find(|v| v["type"] == "tool_use" && v["id"] == tool_id)
            .ok_or_else(|| format!("No recorded tool call {tool_id} for this plan"))?;
        let name = call["name"].as_str().unwrap_or("");
        let input = call["input"].to_string();
        if screenshot && !(name.ends_with("browser_inspect") || name.ends_with("browser_run")) {
            return Err("Screenshot evidence must come from a browser tool".into());
        }
        if screenshot && !input.contains("screenshot") {
            return Err("Browser receipt must request an actual screenshot".into());
        }
        if !screenshot && !(name.ends_with("browser_run") || name.ends_with("shell")) {
            return Err(
                "Functional evidence must come from browser actions or a test command".into(),
            );
        }
        let receipt = evidence
            .iter()
            .find(|v| v["type"] == "tool_result" && v["tool_use_id"] == tool_id)
            .ok_or_else(|| {
                format!("No persisted result for {tool_id}; wait for the tool to finish")
            })?;
        if receipt["is_error"].as_bool() == Some(true) || receipt["content"].is_null() {
            return Err(format!("Tool {tool_id} did not return successful evidence"));
        }
    }
    conn.execute(
        "UPDATE plan_browser_reviews SET evidence_json=?2 WHERE plan_id=?1",
        params![id, review.to_string()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::plans;
    #[test]
    fn website_review_cannot_be_deleted_replaced_or_asserted_without_receipts() {
        let db = Db::open(":memory:");
        db.conn().execute("INSERT INTO sessions(id,project_path,created_at,updated_at,user_id) VALUES('s','/tmp','now','now','u')",[]).unwrap();
        let plan = plans::create_plan(
            &db,
            "Build the website",
            None,
            &[json!({"text":"Implement"})],
            Some("s"),
            None,
            "u",
        )
        .unwrap();
        let id = &plan.meta.id;
        let review_id = format!("browser-review-{id}");
        assert!(required(&db.conn(), id));
        plans::update_plan_execution(&db, id, "start").unwrap();
        assert_eq!(
            plans::upsert_plan_item(&db, id, &review_id, Some("done"), None, None, None, None)
                .unwrap_err(),
            plans::PlanItemUpdateError::BrowserReviewRequired
        );
        plans::delete_plan_item(&db, id, &review_id);
        assert!(
            plans::get_plan(&db, id)
                .unwrap()
                .items
                .iter()
                .any(|i| i.id == review_id)
        );
        plans::replace_plan_items(&db, id, &[json!({"text":"Claim complete","status":"done"})])
            .unwrap();
        assert!(!plans::get_plan(&db, id).unwrap().meta.finished);
        let review = json!({"desktopToolId":"desktop","mobileToolId":"mobile","functionalToolId":"journey","revision":"fixture-v1","journeys":"Signed in and persisted after reload","visualFindings":"Desktop and mobile reviewed"});
        assert!(record(&db, id, &review).is_err());
        let conn = db.conn();
        for (tool, name, input) in [
            (
                "desktop",
                "mcp__flamel__browser_inspect",
                json!({"include":["screenshot"]}),
            ),
            (
                "mobile",
                "browser_inspect",
                json!({"include":["screenshot"]}),
            ),
            (
                "journey",
                "browser_run",
                json!({"code":"test customer journey"}),
            ),
        ] {
            for block in [
                json!({"type":"tool_use","id":tool,"name":name,"input":input}),
                json!({"type":"tool_result","tool_use_id":tool,"is_error":false,"content":"Actual fixture receipt"}),
            ] {
                conn.execute("INSERT INTO session_messages(session_id,role,message,created_at) VALUES('s','assistant',?1,?2)",params![json!({"content":[block]}).to_string(),chrono::Utc::now().to_rfc3339()]).unwrap();
            }
        }
        drop(conn);
        let receipts = status(&db, id);
        assert_eq!(receipts["availableReceipts"].as_array().unwrap().len(), 3);
        assert!(
            receipts["availableReceipts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["toolId"] == "desktop")
        );
        record(&db, id, &review).unwrap();
        plans::upsert_plan_item(&db, id, &review_id, Some("done"), None, None, None, None).unwrap();
        assert!(plans::get_plan(&db, id).unwrap().meta.finished);
        plans::add_plan_items(&db, id, &[json!({"text":"Change customer flow"})]);
        assert!(!satisfied(&db.conn(), id));
        assert!(!plans::get_plan(&db, id).unwrap().meta.finished);
    }
}
