use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;
use crate::db::plans::{self, PlanItemUpdateError};

use crate::routes::admin::get_current_user_id;

fn is_explicit_plan_execution_approval(message: &str) -> bool {
    let normalized = message
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let rejection_markers = [
        "do not start",
        "don t start",
        "not yet",
        "wait",
        "hold off",
        "stop",
        "cancel",
    ];
    if rejection_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return false;
    }

    let short_approvals = [
        "approved",
        "approve it",
        "go",
        "go ahead",
        "proceed",
        "start",
        "start it",
        "start the plan",
        "run it",
        "do it",
        "begin",
        "execute",
        "implement it",
        "ship it",
    ];
    if short_approvals.contains(&normalized.as_str()) {
        return true;
    }

    [
        "start the plan",
        "run the plan",
        "execute the plan",
        "begin the plan",
        "proceed with the plan",
        "go ahead with the plan",
        "implement the plan",
        "approve the plan",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

#[derive(Deserialize)]
pub struct ListQuery {
    q: Option<String>,
    #[serde(rename = "projectPath")]
    project_path: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    archived: Option<String>,
}

pub async fn handle_list_plans(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Json<Value> {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            tracing::warn!("list_plans: no user_id — returning empty");
            return Json(json!([]));
        }
    };
    let archived = query.archived.as_deref() == Some("1");
    let result = plans::list_plans(
        &state.db,
        query.q.as_deref(),
        query.project_path.as_deref(),
        query.session_id.as_deref(),
        Some(&user_id),
        archived,
    );
    Json(serde_json::to_value(result).unwrap())
}

pub async fn handle_get_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let current_user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    // Check ownership via DB before returning plan data
    let owner = plans::get_plan_user_id(&state.db, &id);
    if owner.as_deref() != Some(&current_user_id) {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
    }
    match plans::get_plan(&state.db, &id) {
        Some(plan) => {
            let mut value = serde_json::to_value(plan).unwrap();
            value["browserReview"] = crate::db::plan_quality::status(&state.db, &id);
            (StatusCode::OK, Json(value))
        }
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    }
}

pub async fn handle_create_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    };
    let title = match body.get("title").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "title required"})),
            );
        }
    };
    let description = body.get("description").and_then(|v| v.as_str());
    let items: Vec<Value> = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let session_id = body.get("sessionId").and_then(|v| v.as_str());
    let project_path = body.get("projectPath").and_then(|v| v.as_str());

    match plans::create_plan(
        &state.db,
        title,
        description,
        &items,
        session_id,
        project_path,
        &user_id,
    ) {
        Some(plan) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(plan).unwrap_or_else(|_| json!({}))),
        ),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "failed to create plan"})),
        ),
    }
}

pub async fn handle_update_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    if let Some(owner) = plans::get_plan_user_id(&state.db, &id) {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }
    if body["browserReviewRequired"].as_bool() == Some(true) {
        let result = crate::db::plan_quality::require(&state.db.conn(), &id);
        if let Err(error) = result {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":error.to_string()})),
            );
        }
    }
    if let Some(review) = body.get("browserReview") {
        if let Err(error) = crate::db::plan_quality::record(&state.db, &id, review) {
            return (
                StatusCode::CONFLICT,
                Json(json!({"error":"browser_review_required","message":error})),
            );
        }
    }
    if body["finished"].as_bool() == Some(true) {
        if let Some(plan) = plans::get_plan(&state.db, &id) {
            if plan.items.iter().any(|item| item.status != "done")
                || !crate::db::plan_quality::satisfied(&state.db.conn(), &id)
            {
                return (
                    StatusCode::CONFLICT,
                    Json(
                        json!({"error":"unfinished_work","message":"Complete every plan item and required browser review before finishing."}),
                    ),
                );
            }
        }
    }
    let title = body.get("title").and_then(|v| v.as_str());
    let description = body.get("description").and_then(|v| v.as_str());
    let finished = body.get("finished").and_then(|v| v.as_bool());
    let archived = body.get("archived").and_then(|v| v.as_bool());

    match plans::update_plan(&state.db, &id, title, description, finished, archived) {
        Some(meta) => (StatusCode::OK, Json(serde_json::to_value(meta).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    }
}

pub async fn handle_update_plan_execution(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    if let Some(owner) = plans::get_plan_user_id(&state.db, &id) {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }
    let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("");
    if !matches!(action, "start" | "resume" | "pause" | "block") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid execution action"})),
        );
    }

    let plan = match plans::get_plan(&state.db, &id) {
        Some(plan) => plan,
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    };
    if matches!(action, "start" | "resume") && plan.meta.approval_status == "pending" {
        let approval_session_id = body
            .get("approvalSessionId")
            .and_then(|value| value.as_str())
            .filter(|session_id| {
                crate::db::sessions::get_session(&state.db, session_id)
                    .and_then(|session| session.user_id)
                    .as_deref()
                    == Some(user_id.as_str())
            })
            .or(plan.meta.session_id.as_deref());
        let approved_from_message = approval_session_id
            .zip(plan.meta.approval_requested_at.as_deref())
            .and_then(|(session_id, requested_at)| {
                crate::db::sessions::latest_visible_user_message_after(
                    &state.db,
                    session_id,
                    requested_at,
                )
            })
            .is_some_and(|message| is_explicit_plan_execution_approval(&message));

        if !approved_from_message {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "explicit_approval_required",
                    "message": "The user must explicitly start or approve this plan after its latest revision."
                })),
            );
        }
    }

    match plans::update_plan_execution(&state.db, &id, action) {
        Some(plan) => (StatusCode::OK, Json(serde_json::to_value(plan).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Not found"}))),
    }
}

pub async fn handle_delete_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    if let Some(owner) = plans::get_plan_user_id(&state.db, &id) {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }
    plans::delete_plan(&state.db, &id);
    (StatusCode::OK, Json(json!({"ok": true})))
}

pub async fn handle_upsert_plan_item(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((plan_id, item_id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    if let Some(owner) = plans::get_plan_user_id(&state.db, &plan_id) {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }
    let status = body.get("status").and_then(|v| v.as_str());
    let notes = body.get("notes").and_then(|v| v.as_str());
    let text = body.get("text").and_then(|v| v.as_str());
    let section = body.get("section").and_then(|v| v.as_str());
    let reasoning_floor = body
        .get("reasoningFloor")
        .or_else(|| body.get("reasoning_floor"))
        .and_then(|v| v.as_str());

    match plans::upsert_plan_item(
        &state.db,
        &plan_id,
        &item_id,
        status,
        notes,
        text,
        section,
        reasoning_floor,
    ) {
        Ok(item) => {
            let mut value = serde_json::to_value(item).unwrap_or_else(|_| json!({}));
            if let Some(plan) = plans::get_plan(&state.db, &plan_id) {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("executionStatus".into(), json!(plan.meta.execution_status));
                    obj.insert("approvalStatus".into(), json!(plan.meta.approval_status));
                    obj.insert("activeItemId".into(), json!(plan.meta.active_item_id));
                    obj.insert("finished".into(), json!(plan.meta.finished));
                }
            }
            (StatusCode::OK, Json(value))
        }
        Err(PlanItemUpdateError::ApprovalRequired) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "approval_required",
                "message": "Start or resume this plan before marking items in progress or done."
            })),
        ),
        Err(PlanItemUpdateError::BrowserReviewRequired) => (
            StatusCode::CONFLICT,
            Json(
                json!({"error":"browser_review_required","message":"Record successful desktop/mobile screenshot and functional journey receipts with plan(action=browser_review) before completing this item."}),
            ),
        ),
        Err(PlanItemUpdateError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Item not found"})),
        ),
    }
}

pub async fn handle_delete_plan_item(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((plan_id, item_id)): Path<(String, String)>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    if let Some(owner) = plans::get_plan_user_id(&state.db, &plan_id) {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }
    if item_id == format!("browser-review-{plan_id}")
        && crate::db::plan_quality::required(&state.db.conn(), &plan_id)
    {
        return (
            StatusCode::CONFLICT,
            Json(
                json!({"error":"required_acceptance","message":"The required browser review cannot be deleted. Record its evidence before completing it."}),
            ),
        );
    }
    plans::delete_plan_item(&state.db, &plan_id, &item_id);
    (StatusCode::OK, Json(json!({"ok": true})))
}

pub async fn handle_add_plan_items(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    if let Some(owner) = plans::get_plan_user_id(&state.db, &plan_id) {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }
    let items: Vec<Value> = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "items required"})),
        );
    }
    let new_items = plans::add_plan_items(&state.db, &plan_id, &items);
    let mut response = json!({"items": serde_json::to_value(new_items).unwrap()});
    if let Some(plan) = plans::get_plan(&state.db, &plan_id) {
        if let Some(obj) = response.as_object_mut() {
            obj.insert("id".into(), json!(plan.meta.id));
            obj.insert("planId".into(), json!(plan_id));
            obj.insert("title".into(), json!(plan.meta.title));
            obj.insert("description".into(), json!(plan.meta.description));
            obj.insert("artifact".into(), json!(plan.meta.artifact));
            obj.insert("versions".into(), json!(plan.versions));
            obj.insert("versionCount".into(), json!(plan.meta.version_count));
            obj.insert("finished".into(), json!(plan.meta.finished));
            obj.insert("executionStatus".into(), json!(plan.meta.execution_status));
            obj.insert("approvalStatus".into(), json!(plan.meta.approval_status));
            obj.insert("activeItemId".into(), json!(plan.meta.active_item_id));
            obj.insert(
                "approvalRequestedAt".into(),
                json!(plan.meta.approval_requested_at),
            );
            obj.insert("approvedAt".into(), json!(plan.meta.approved_at));
            obj.insert(
                "executionStartedAt".into(),
                json!(plan.meta.execution_started_at),
            );
            obj.insert(
                "executionUpdatedAt".into(),
                json!(plan.meta.execution_updated_at),
            );
        }
    }
    (StatusCode::CREATED, Json(response))
}

pub async fn handle_replace_plan_items(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let user_id = match get_current_user_id(&state, &headers) {
        Some(id) => id,
        None => return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))),
    };
    if let Some(owner) = plans::get_plan_user_id(&state.db, &plan_id) {
        if owner != user_id {
            return (StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"})));
        }
    }
    let items: Vec<Value> = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "items required"})),
        );
    }
    match plans::replace_plan_items(&state.db, &plan_id, &items) {
        Some(plan) => (StatusCode::OK, Json(serde_json::to_value(plan).unwrap())),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Plan not found"})),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::is_explicit_plan_execution_approval;

    #[test]
    fn recognizes_explicit_plan_execution_approval() {
        for message in [
            "go ahead",
            "start the plan",
            "approved",
            "Please proceed with the plan now.",
            "Make that wording change, then start the plan.",
        ] {
            assert!(
                is_explicit_plan_execution_approval(message),
                "expected approval: {message}"
            );
        }
    }

    #[test]
    fn rejects_edits_and_non_approval_messages() {
        for message in [
            "make it 500 chars not 200",
            "change the second item",
            "what does item three mean?",
            "yes",
            "okay",
            "looks good",
            "do not start the plan",
            "not yet",
            "wait, start the plan after I review it",
        ] {
            assert!(
                !is_explicit_plan_execution_approval(message),
                "expected rejection: {message}"
            );
        }
    }
}
