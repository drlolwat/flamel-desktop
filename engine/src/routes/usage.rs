use axum::{Json, extract::State, http::HeaderMap};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;
use crate::routes::admin::get_current_user_id;

pub async fn handle_get_usage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Value> {
    let user_id = get_current_user_id(&state, &headers);

    // If unauthenticated, return empty data rather than leaking all users' stats.
    let uid = match &user_id {
        Some(id) => id.clone(),
        None => {
            return Json(json!({
                "today": {"inputTokens": 0, "outputTokens": 0},
                "week": {"inputTokens": 0, "outputTokens": 0},
                "month": {"inputTokens": 0, "outputTokens": 0},
                "byProject": [],
            }));
        }
    };

    let conn = state.db.conn();

    let now = chrono::Utc::now();
    let today_start = now.format("%Y-%m-%dT00:00:00+00:00").to_string();
    let week_start = (now - chrono::Duration::days(7))
        .format("%Y-%m-%dT00:00:00+00:00")
        .to_string();
    let month_start = now
        .with_day(1)
        .unwrap_or(now)
        .format("%Y-%m-%dT00:00:00+00:00")
        .to_string();

    let query_period = |start: &str| -> (i64, i64) {
        conn.query_row(
            "SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0) FROM sessions WHERE updated_at >= ?1 AND user_id = ?2",
            rusqlite::params![start, uid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, 0))
    };

    let (today_in, today_out) = query_period(&today_start);
    let (week_in, week_out) = query_period(&week_start);
    let (month_in, month_out) = query_period(&month_start);

    let mut stmt = conn
        .prepare("SELECT project_path, SUM(input_tokens), SUM(output_tokens) FROM sessions WHERE user_id = ?1 GROUP BY project_path ORDER BY SUM(input_tokens) DESC LIMIT 20")
        .unwrap();
    let by_project: Vec<Value> = stmt
        .query_map(rusqlite::params![uid], |row| {
            Ok(json!({
                "projectPath": row.get::<_, String>(0)?,
                "inputTokens": row.get::<_, i64>(1)?,
                "outputTokens": row.get::<_, i64>(2)?,
            }))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    Json(json!({
        "today": {"inputTokens": today_in, "outputTokens": today_out},
        "week": {"inputTokens": week_in, "outputTokens": week_out},
        "month": {"inputTokens": month_in, "outputTokens": month_out},
        "byProject": by_project,
    }))
}

use chrono::Datelike;
