use crate::claude::provider::ProviderKind;
use crate::db::Db;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,
    pub project_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: i64,
    pub pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub provider: String,
    pub continuity_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checkpoint_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_owner_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orchestration_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orchestration_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orchestration_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_plan_id: Option<String>,
    pub requested_model: String,
    pub requested_reasoning_effort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_reason: Option<String>,
}

pub fn list_sessions(
    db: &Db,
    user_id: Option<&str>,
    query: Option<&str>,
    project_path: Option<&str>,
) -> Vec<SessionMeta> {
    let conn = db.conn();

    // Build parameterised WHERE clause dynamically
    // The session browser is an interactive-chat surface. Internal execution
    // sessions remain addressable by id for Goal streaming and audit, but must
    // never leak into the user's chat history.
    let mut where_parts: Vec<&str> = vec![
        "COALESCE(session_owner_kind, 'interactive') = 'interactive'",
        "(EXISTS (SELECT 1 FROM session_messages visible WHERE visible.session_id=sessions.id AND visible.role IN ('user', 'assistant')) OR NULLIF(TRIM(claude_session_id), '') IS NOT NULL)",
    ];
    let mut params: Vec<String> = Vec::new();

    if let Some(q) = query {
        // FTS — MATCH clause comes first, extra filters follow as AND
        if let Some(uid) = user_id {
            where_parts.push("s.user_id = ?");
            params.push(uid.to_string());
        }
        if let Some(pp) = project_path {
            where_parts.push("s.project_path = ?");
            params.push(pp.to_string());
        }
        // Qualify the visibility predicate for the FTS join.
        where_parts[0] = "COALESCE(s.session_owner_kind, 'interactive') = 'interactive'";
        where_parts[1] = "(EXISTS (SELECT 1 FROM session_messages visible WHERE visible.session_id=s.id AND visible.role IN ('user', 'assistant')) OR NULLIF(TRIM(s.claude_session_id), '') IS NOT NULL)";
        let extra = format!("AND {}", where_parts.join(" AND "));
        let sql = format!(
            "SELECT s.* FROM sessions s
             JOIN sessions_fts ON sessions_fts.id = s.id
             WHERE sessions_fts MATCH ? {extra}
             ORDER BY s.updated_at DESC LIMIT 100"
        );
        // MATCH param goes first
        let all_params: Vec<String> = std::iter::once(q.to_string()).chain(params).collect();
        let mut stmt = conn.prepare(&sql).unwrap();
        stmt.query_map(rusqlite::params_from_iter(all_params.iter()), row_to_meta)
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    } else {
        if let Some(uid) = user_id {
            where_parts.push("user_id = ?");
            params.push(uid.to_string());
        }
        if let Some(pp) = project_path {
            where_parts.push("project_path = ?");
            params.push(pp.to_string());
        }
        let where_clause = if where_parts.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_parts.join(" AND "))
        };
        let sql = format!(
            "SELECT * FROM sessions {where_clause}
             ORDER BY pinned DESC, updated_at DESC LIMIT 200"
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        stmt.query_map(rusqlite::params_from_iter(params.iter()), row_to_meta)
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }
}

pub fn get_session(db: &Db, id: &str) -> Option<SessionMeta> {
    let conn = db.conn();
    conn.query_row("SELECT * FROM sessions WHERE id = ?1", [id], row_to_meta)
        .ok()
}

pub fn upsert_session(db: &Db, meta: &UpsertSession) -> Option<SessionMeta> {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();

    let existing = get_session_inner(&conn, &meta.id);
    if existing.is_some() {
        conn.execute(
            "UPDATE sessions SET
                claude_session_id = COALESCE(?1, claude_session_id),
                title = COALESCE(?2, title),
                updated_at = ?3,
                message_count = COALESCE(?4, message_count),
                pinned = COALESCE(?5, pinned),
                model = COALESCE(?6, model),
                input_tokens = COALESCE(?7, input_tokens),
                output_tokens = COALESCE(?8, output_tokens),
                provider = COALESCE(?9, provider)
            WHERE id = ?10",
            rusqlite::params![
                meta.claude_session_id,
                meta.title,
                now,
                meta.message_count,
                meta.pinned.map(|p| if p { 1 } else { 0 }),
                meta.model,
                meta.input_tokens,
                meta.output_tokens,
                meta.provider.as_ref().map(|p| p.as_str()),
                meta.id,
            ],
        )
        .ok();
    } else {
        conn.execute(
            "INSERT INTO sessions (id, claude_session_id, project_path, title, created_at, updated_at, message_count, pinned, model, input_tokens, output_tokens, provider, user_id, runner_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                meta.id,
                meta.claude_session_id,
                meta.project_path,
                meta.title,
                now,
                meta.message_count.unwrap_or(0),
                if meta.pinned.unwrap_or(false) { 1 } else { 0 },
                meta.model,
                meta.input_tokens.unwrap_or(0),
                meta.output_tokens.unwrap_or(0),
                meta.provider.as_deref().unwrap_or(ProviderKind::Claude.as_str()),
                meta.user_id,
                meta.runner_id,
            ],
        )
        .ok();
    }

    get_session_inner(&conn, &meta.id)
}

pub struct UpsertSession {
    pub id: String,
    pub claude_session_id: Option<String>,
    pub project_path: String,
    pub title: Option<String>,
    pub message_count: Option<i64>,
    pub pinned: Option<bool>,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub provider: Option<String>,
    pub user_id: Option<String>,
    pub runner_id: Option<String>,
}

pub fn set_session_routing(
    db: &Db,
    session_id: &str,
    requested_model: &str,
    requested_reasoning_effort: &str,
    resolved_model: &str,
    provider: &str,
    resolved_reasoning_effort: Option<&str>,
    routing_reason: &str,
) {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE sessions
         SET requested_model = ?1,
             requested_reasoning_effort = ?2,
             model = ?3,
             provider = ?4,
             resolved_reasoning_effort = ?5,
             routing_reason = ?6,
             updated_at = ?7
         WHERE id = ?8",
        rusqlite::params![
            requested_model,
            requested_reasoning_effort,
            resolved_model,
            provider,
            resolved_reasoning_effort,
            routing_reason,
            now,
            session_id,
        ],
    )
    .ok();
}

pub fn update_session_continuity(
    db: &Db,
    session_id: &str,
    continuity_status: &str,
    recovery_strategy: Option<&str>,
    active_request_id: Option<&str>,
    checkpointed: bool,
    last_error: Option<&str>,
) {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE sessions
         SET continuity_status = ?1,
             recovery_strategy = ?2,
             active_request_id = ?3,
             last_error = ?4,
             last_checkpoint_at = CASE WHEN ?5 THEN ?6 ELSE last_checkpoint_at END,
             updated_at = ?6
         WHERE id = ?7",
        rusqlite::params![
            continuity_status,
            recovery_strategy,
            active_request_id,
            last_error,
            if checkpointed { 1 } else { 0 },
            now,
            session_id,
        ],
    )
    .ok();
}

pub fn mark_session_checkpoint(db: &Db, session_id: &str) {
    let current = get_session(db, session_id)
        .map(|session| {
            (
                session.continuity_status,
                session.recovery_strategy,
                session.active_request_id,
                session.last_error,
            )
        })
        .unwrap_or_else(|| ("active".to_string(), None, None, None));
    update_session_continuity(
        db,
        session_id,
        &current.0,
        current.1.as_deref(),
        current.2.as_deref(),
        true,
        current.3.as_deref(),
    );
}

pub fn set_session_owner(
    db: &Db,
    session_id: &str,
    owner_kind: &str,
    orchestration_id: Option<&str>,
    orchestration_run_id: Option<&str>,
    orchestration_role: Option<&str>,
) {
    let conn = db.conn();
    conn.execute(
        "UPDATE sessions
         SET session_owner_kind = ?2,
             orchestration_id = ?3,
             orchestration_run_id = ?4,
             orchestration_role = ?5
         WHERE id = ?1",
        rusqlite::params![
            session_id,
            owner_kind,
            orchestration_id,
            orchestration_run_id,
            orchestration_role,
        ],
    )
    .ok();
}

pub fn update_session(
    db: &Db,
    id: &str,
    title: Option<&str>,
    pinned: Option<bool>,
) -> Option<SessionMeta> {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();

    if let Some(t) = title {
        conn.execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![t, now, id],
        )
        .ok();
    }
    if let Some(p) = pinned {
        conn.execute(
            "UPDATE sessions SET pinned = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![if p { 1 } else { 0 }, now, id],
        )
        .ok();
    }

    get_session_inner(&conn, id)
}

/// Persist the plan currently attached to an interactive session. The server owns
/// this relationship so reconnecting clients never infer it from list ordering.
pub fn set_session_active_plan(
    db: &Db,
    session_id: &str,
    active_plan_id: Option<&str>,
) -> Option<SessionMeta> {
    let conn = db.conn();
    let now = chrono::Utc::now().to_rfc3339();
    let changed = conn
        .execute(
            "UPDATE sessions SET active_plan_id = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![active_plan_id, now, session_id],
        )
        .ok()?;
    if changed == 0 {
        return None;
    }
    get_session_inner(&conn, session_id)
}

/// Repair pre-migration sessions only when the database contains exactly one
/// running plan for the session. Ambiguous history is deliberately left unset.
pub fn reconcile_session_active_plan(db: &Db, session_id: &str) -> Option<String> {
    let conn = db.conn();
    let current = conn
        .query_row(
            "SELECT active_plan_id FROM sessions WHERE id = ?1",
            [session_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten();
    if let Some(plan_id) = current {
        let valid = conn
            .query_row(
                "SELECT COUNT(*) FROM plans
                 WHERE id = ?1 AND session_id = ?2 AND COALESCE(archived, 0) = 0",
                rusqlite::params![plan_id, session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            == 1;
        if valid {
            return Some(plan_id);
        }
        conn.execute(
            "UPDATE sessions SET active_plan_id = NULL WHERE id = ?1",
            [session_id],
        )
        .ok();
    }

    let candidates = {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM plans
                 WHERE session_id = ?1
                   AND COALESCE(archived, 0) = 0
                   AND COALESCE(finished, 0) = 0
                   AND execution_status = 'running'
                 ORDER BY COALESCE(execution_updated_at, updated_at) DESC
                 LIMIT 2",
            )
            .ok()?;
        stmt.query_map([session_id], |row| row.get::<_, String>(0))
            .ok()?
            .filter_map(Result::ok)
            .collect::<Vec<_>>()
    };
    if candidates.len() != 1 {
        return None;
    }
    let plan_id = candidates[0].clone();
    conn.execute(
        "UPDATE sessions SET active_plan_id = ?1 WHERE id = ?2",
        rusqlite::params![plan_id, session_id],
    )
    .ok()?;
    Some(plan_id)
}

const CONTEXT_SUMMARY_LIMIT: i64 = 15;

#[derive(Debug, Clone)]
pub struct RecoveryContextParts {
    pub compaction_summary: Option<String>,
    pub plan_context: Option<String>,
    pub recent_turns: String,
    pub recent_turn_count: usize,
}

/// Load a compact text summary of the last N messages in a session.
pub fn load_session_context_summary(db: &Db, session_id: &str) -> String {
    load_session_context_summary_with_offset(db, session_id, 0)
}

/// Build persisted recovery context for restarting a lost session.
/// Includes all persisted messages — recovery callers now send a dedicated
/// continuation prompt instead of replaying the original user message.
pub fn load_session_recovery_context(db: &Db, session_id: &str) -> Option<String> {
    load_session_recovery_context_with_offset(db, session_id, 0)
}

pub fn load_session_recovery_context_parts(
    db: &Db,
    session_id: &str,
) -> Option<RecoveryContextParts> {
    load_session_recovery_context_parts_with_offset(db, session_id, 0)
}

pub fn load_session_recovery_context_with_offset(
    db: &Db,
    session_id: &str,
    offset: i64,
) -> Option<String> {
    load_session_recovery_context_parts_with_offset(db, session_id, offset)
        .and_then(|parts| format_recovery_context_from_parts(&parts))
}

pub fn load_session_recovery_context_parts_with_offset(
    db: &Db,
    session_id: &str,
    offset: i64,
) -> Option<RecoveryContextParts> {
    let compaction_summary = load_compaction_summary(db, session_id)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let recent_turns = load_session_context_summary_with_offset(db, session_id, offset);
    let recent_turn_count = recent_turns
        .split("\n\n")
        .filter(|section| !section.trim().is_empty())
        .count();
    let plan_context = crate::db::plans::load_project_plan_context_for_session(db, session_id)
        .map(|context| context.trim().to_string())
        .filter(|context| !context.is_empty());
    if compaction_summary.is_none() && plan_context.is_none() && recent_turns.trim().is_empty() {
        None
    } else {
        Some(RecoveryContextParts {
            compaction_summary,
            plan_context,
            recent_turns,
            recent_turn_count,
        })
    }
}

fn load_raw_session_messages(db: &Db, session_id: &str) -> Vec<(String, String)> {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT role, message FROM session_messages WHERE session_id = ?1 ORDER BY rowid ASC",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([session_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };

    rows.filter_map(Result::ok).collect()
}

pub fn load_persisted_session_messages(db: &Db, session_id: &str) -> Vec<(String, String)> {
    collapse_persisted_messages_into_visible_turns(load_raw_session_messages(db, session_id), false)
}

/// Load the visible conversation plus durable UI-only history records.
/// Compaction checkpoints are deliberately excluded from provider input but
/// retained here so reconnecting clients can reconstruct the chat timeline.
pub fn load_persisted_session_history(db: &Db, session_id: &str) -> Vec<(String, String)> {
    collapse_persisted_messages_into_visible_turns(load_raw_session_messages(db, session_id), true)
}

pub fn has_visible_session_messages(db: &Db, session_id: &str) -> bool {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT role, message FROM session_messages
         WHERE session_id = ?1 AND role IN ('user', 'assistant')
         ORDER BY rowid ASC",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return false,
    };
    let rows = match stmt.query_map([session_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) {
        Ok(rows) => rows,
        Err(_) => return false,
    };

    // This is used by hot metadata routes. Do not load and rebuild the whole
    // transcript merely to answer whether one visible turn exists.
    for row in rows.filter_map(Result::ok) {
        let (role, message) = row;
        if role == "assistant" || !is_hidden_continuation_message(&message) {
            return true;
        }
    }
    false
}

pub fn latest_visible_user_message_after(db: &Db, session_id: &str, after: &str) -> Option<String> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT message FROM session_messages
             WHERE session_id = ?1 AND role = 'user' AND created_at > ?2
             ORDER BY rowid DESC",
        )
        .ok()?;
    let rows = stmt
        .query_map(rusqlite::params![session_id, after], |row| {
            row.get::<_, String>(0)
        })
        .ok()?;

    for message in rows.filter_map(Result::ok) {
        if is_hidden_continuation_message(&message) {
            continue;
        }
        let text = extract_message_text_for_summary(&message);
        if !text.trim().is_empty() {
            return Some(text);
        }
    }
    None
}

fn is_hidden_continuation_message(message: &str) -> bool {
    let msg: serde_json::Value = match serde_json::from_str(message) {
        Ok(v) => v,
        Err(_) => return false,
    };
    msg.get("flamel")
        .and_then(|meta| meta.get("hiddenContinuation"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn load_session_context_summary_with_offset(db: &Db, session_id: &str, offset: i64) -> String {
    let mut raw_messages = load_raw_session_messages(db, session_id);
    let offset = offset.max(0) as usize;
    if offset > 0 {
        let keep = raw_messages.len().saturating_sub(offset);
        raw_messages.truncate(keep);
    }
    let mut messages = collapse_persisted_messages_into_visible_turns(raw_messages, false);
    trim_trailing_resumption_recaps(&mut messages);
    let start = messages
        .len()
        .saturating_sub(CONTEXT_SUMMARY_LIMIT as usize);

    let mut parts: Vec<String> = Vec::new();
    for (role, msg_json) in messages[start..].iter() {
        let text = extract_message_text_for_summary(msg_json);
        if !text.is_empty() {
            let truncated = if role.as_str() == "assistant" {
                truncate_summary_text_preserving_tail(&text, 600)
            } else {
                truncate_summary_text(&text, 600)
            };
            parts.push(format!("{}: {}", role.to_uppercase(), truncated));
        }
    }
    parts.join("\n\n")
}

fn format_recovery_context(
    compaction_summary: Option<&str>,
    plan_context: Option<&str>,
    recent_turns: Option<&str>,
) -> Option<String> {
    let mut sections = Vec::new();
    if let Some(turns) = recent_turns.map(str::trim).filter(|s| !s.is_empty()) {
        sections.push(format!(
            "NEWEST PERSISTED TURNS (authoritative for completed and current work):\n{turns}"
        ));
    }
    if let Some(plan_context) = plan_context.map(str::trim).filter(|s| !s.is_empty()) {
        sections.push(plan_context.to_string());
    }
    if let Some(summary) = compaction_summary.map(str::trim).filter(|s| !s.is_empty()) {
        sections.push(format!(
            "OLDER CHECKPOINT BACKGROUND (may be stale; never override newer turns):\n{summary}"
        ));
    }
    if sections.is_empty() {
        None
    } else {
        Some(format!(
            concat!(
                "SESSION RECOVERY CONTEXT: The live provider session was lost and this context is hidden recovery state, ",
                "not a new user request. The current user message outside this block is authoritative and must be handled ",
                "before any recovered work. Within this block, use this precedence: NEWEST PERSISTED TURNS for completed ",
                "and current work; database-backed ACCEPTED PLAN CONTEXT for plan identity, progress, and active items only; ",
                "then OLDER CHECKPOINT BACKGROUND for historical context only. Never repeat, undo, or report as unfinished ",
                "an action that newer turns show as completed. Do not restart from, re-answer, or re-derive an earlier user ",
                "prompt. If the previous task was already complete, respond only to the current user message. If recovery ",
                "followed an interruption, the newest persisted user turn replaces the interrupted request. Do not reopen or ",
                "change plan items solely because recovery context mentions a plan.\n\n{}"
            ),
            sections.join("\n\n")
        ))
    }
}

pub fn format_recovery_context_from_parts(parts: &RecoveryContextParts) -> Option<String> {
    format_recovery_context(
        parts.compaction_summary.as_deref(),
        parts.plan_context.as_deref(),
        Some(parts.recent_turns.as_str()),
    )
}

pub fn format_summarized_recovery_context(
    parts: &RecoveryContextParts,
    resume_digest: &str,
) -> Option<String> {
    let resume_digest = resume_digest.trim();
    if resume_digest.is_empty() {
        return None;
    }

    let mut sections = vec![format!(
        "NEWEST RECOVERY DIGEST (authoritative for completed and current work):\n{resume_digest}"
    )];
    if let Some(plan_context) = parts
        .plan_context
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        sections.push(plan_context.to_string());
    }
    if let Some(summary) = parts
        .compaction_summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        sections.push(format!(
            "OLDER CHECKPOINT BACKGROUND (may be stale; never override the recovery digest):\n{summary}"
        ));
    }

    Some(format!(
        concat!(
            "SESSION RECOVERY CONTEXT: The live provider session was lost and this context is hidden recovery state, ",
            "not a new user request. The current user message outside this block is authoritative and must be handled ",
            "before any recovered work. Within this block, use the NEWEST RECOVERY DIGEST for completed and current work; ",
            "use database-backed ACCEPTED PLAN CONTEXT for plan identity, progress, and active items only; and treat OLDER ",
            "CHECKPOINT BACKGROUND as historical context only. Never repeat, undo, or report as unfinished an action that ",
            "the digest shows as completed. Do not restart from, re-answer, or re-derive an earlier user prompt. If the ",
            "previous task was already complete, respond only to the current user message. If recovery followed an ",
            "interruption, the newest persisted user turn replaces the interrupted request. Do not reopen or change plan ",
            "items solely because recovery context mentions a plan.\n\n{}"
        ),
        sections.join("\n\n")
    ))
}

pub fn format_recovery_display_summary(parts: &RecoveryContextParts) -> Option<String> {
    let mut sections = Vec::new();
    if let Some(turns) = Some(parts.recent_turns.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        sections.push(format!("Newest persisted turns:\n{turns}"));
    }
    if let Some(plan_context) = parts
        .plan_context
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        sections.push(plan_context.to_string());
    }
    if let Some(summary) = parts
        .compaction_summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        sections.push(format!("Older checkpoint background:\n{summary}"));
    }
    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

fn trim_trailing_resumption_recaps(messages: &mut Vec<(String, String)>) {
    while let Some((role, message)) = messages.last() {
        if role != "assistant" || !looks_like_resumption_recap(message) {
            break;
        }
        messages.pop();
    }
}

fn looks_like_stall_recap_text(text: &str) -> bool {
    let normalized = text.trim().to_lowercase();
    if normalized.is_empty() || normalized.ends_with('?') {
        return false;
    }

    let stall_actions = [
        "i'm checking",
        "i am checking",
        "checking the",
        "i'm loading",
        "i am loading",
        "loading the",
        "i'm reading",
        "i am reading",
        "reading the",
        "i'm tracing",
        "i am tracing",
        "tracing the",
        "i'm looking at",
        "i am looking at",
        "re-reading the",
        "reopening the",
        "pulling the",
        "reviewing the",
        "scanning the",
    ];
    let stall_targets = [
        "active plan",
        "plan panel",
        "plan items",
        "repo state",
        "repo contents",
        "current repo",
        "existing code",
        "entrypoints",
        "codebase",
        "project state",
    ];

    stall_actions
        .iter()
        .any(|marker| normalized.contains(marker))
        && stall_targets
            .iter()
            .any(|marker| normalized.contains(marker))
}

fn looks_like_resumption_recap(message: &str) -> bool {
    let text = extract_message_text_for_summary(message);
    let normalized = text.trim().to_lowercase();
    if normalized.is_empty() || normalized.ends_with('?') {
        return false;
    }

    if looks_like_stall_recap_text(&text) {
        return true;
    }

    let progress_markers = [
        "i'm resuming",
        "i am resuming",
        "i'm switching",
        "i am switching",
        "moving on to",
        "next is",
        "next up",
        "the next real implementation target",
    ];
    let work_markers = [
        "real implementation",
        "actual implementation",
        "move the code forward",
        "real code behind it",
        "replace the fake progress",
        "without guessing",
    ];

    progress_markers
        .iter()
        .any(|marker| normalized.contains(marker))
        && work_markers
            .iter()
            .any(|marker| normalized.contains(marker))
}

fn normalize_summary_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_summary_text(text: &str, max_chars: usize) -> String {
    let normalized = normalize_summary_text(text);
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return "…".to_string();
    }

    let mut truncated: String = normalized.chars().take(max_chars - 1).collect();
    truncated.push('…');
    truncated
}

fn truncate_summary_text_preserving_tail(text: &str, max_chars: usize) -> String {
    let normalized = normalize_summary_text(text);
    if normalized.chars().count() <= max_chars {
        return normalized;
    }

    const SEPARATOR: &str = " … ";
    let separator_chars = SEPARATOR.chars().count();
    if max_chars <= separator_chars {
        return truncate_summary_text(&normalized, max_chars);
    }

    let content_chars = max_chars - separator_chars;
    let head_chars = content_chars.div_ceil(2);
    let tail_chars = content_chars - head_chars;
    let head: String = normalized.chars().take(head_chars).collect();
    let reversed_tail: String = normalized.chars().rev().take(tail_chars).collect();
    let tail: String = reversed_tail.chars().rev().collect();
    format!("{head}{SEPARATOR}{tail}")
}

fn summarize_json_value(value: &serde_json::Value, max_chars: usize) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(truncate_summary_text(text, max_chars)),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => Some(value.to_string()),
        serde_json::Value::Array(items) => {
            let mut parts = items
                .iter()
                .take(3)
                .filter_map(|item| summarize_json_value(item, max_chars.saturating_div(2).max(24)))
                .collect::<Vec<_>>();
            if items.len() > 3 {
                parts.push(format!("+{} more", items.len() - 3));
            }
            if parts.is_empty() {
                None
            } else {
                Some(truncate_summary_text(&parts.join(", "), max_chars))
            }
        }
        serde_json::Value::Object(_) => serde_json::to_string(value)
            .ok()
            .map(|text| truncate_summary_text(&text, max_chars)),
    }
}

fn summarize_tool_input(input: &serde_json::Value) -> Option<String> {
    let obj = input.as_object()?;
    let mut parts = Vec::new();
    let priority_keys = [
        "action",
        "file",
        "relative_path",
        "path",
        "pattern",
        "name_path",
        "command",
        "cwd",
        "query",
        "url",
        "repo",
        "branch",
        "target",
        "title",
        "session_id",
    ];

    for key in priority_keys {
        if let Some(value) = obj
            .get(key)
            .and_then(|value| summarize_json_value(value, 80))
        {
            parts.push(format!("{key}={value}"));
        }
    }

    if let Some(files) = obj.get("files").and_then(|value| value.as_array()) {
        let names = files
            .iter()
            .take(3)
            .filter_map(|value| summarize_json_value(value, 48))
            .collect::<Vec<_>>();
        if !names.is_empty() {
            let suffix = if files.len() > names.len() {
                format!(" (+{} more)", files.len() - names.len())
            } else {
                String::new()
            };
            parts.push(format!("files={}{}", names.join(", "), suffix));
        }
    }
    if let Some(edits) = obj.get("edits").and_then(|value| value.as_array()) {
        parts.push(format!("edits={}", edits.len()));
    }
    if let Some(items) = obj.get("items").and_then(|value| value.as_array()) {
        parts.push(format!("items={}", items.len()));
    }
    if let Some(targets) = obj.get("targets").and_then(|value| value.as_array()) {
        let names = targets
            .iter()
            .take(3)
            .filter_map(|value| summarize_json_value(value, 40))
            .collect::<Vec<_>>();
        if !names.is_empty() {
            parts.push(format!("targets={}", names.join(", ")));
        }
    }

    if parts.is_empty() {
        summarize_json_value(input, 160)
    } else {
        Some(truncate_summary_text(&parts.join(", "), 220))
    }
}

fn summarize_tool_use_block(block: &serde_json::Value) -> String {
    let name = block
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    match block.get("input").and_then(summarize_tool_input) {
        Some(input) if !input.is_empty() => format!("[Tool: {name} | {input}]"),
        _ => format!("[Tool: {name}]"),
    }
}

fn summarize_tool_result_block(block: &serde_json::Value) -> String {
    let tool_id = block
        .get("tool_use_id")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let status = if block
        .get("is_error")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        "error"
    } else {
        "result"
    };
    let detail = block
        .get("content")
        .and_then(|value| summarize_json_value(value, 180));

    match detail {
        Some(text) if !text.is_empty() => format!("[Tool {status}: {tool_id} | {text}]"),
        _ => format!("[Tool {status}: {tool_id}]"),
    }
}

fn extract_message_text_for_summary(msg_json: &str) -> String {
    let msg: serde_json::Value = match serde_json::from_str(msg_json) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    let source = msg.get("message").unwrap_or(&msg);
    let content = if source.is_array() {
        source
    } else {
        match source.get("content") {
            Some(content) => content,
            None => return String::new(),
        }
    };
    if let Some(text) = content.as_str() {
        return normalize_summary_text(text);
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .filter_map(|block| {
                let t = block.get("type").and_then(|v| v.as_str())?;
                match t {
                    "text" => block
                        .get("text")
                        .and_then(|v| v.as_str())
                        .map(|text| truncate_summary_text(text, 240)),
                    "tool_use" => Some(summarize_tool_use_block(block)),
                    "tool_result" => Some(summarize_tool_result_block(block)),
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
    }
    String::new()
}

/// Load ALL messages for a session in chronological order — used for Haiku compaction.
pub fn load_all_messages_for_compaction(db: &Db, session_id: &str) -> Vec<(String, String)> {
    load_persisted_session_messages(db, session_id)
}

pub fn session_history_revision(db: &Db, session_id: &str) -> i64 {
    db.conn()
        .query_row(
            "SELECT revision FROM session_history_revisions WHERE session_id=?1",
            [session_id],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

pub fn bump_session_history_revision_with_conn(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO session_history_revisions (session_id, revision) VALUES (?1, 1)
         ON CONFLICT(session_id) DO UPDATE SET revision=revision+1",
        [session_id],
    )?;
    Ok(())
}

pub fn persist_session_message(db: &Db, session_id: &str, role: &str, message: &str) {
    if let Err(error) = try_persist_session_message(db, session_id, role, message) {
        tracing::error!(%error, session_id, role, "failed to persist a session message");
    }
}

pub fn try_persist_session_message(
    db: &Db,
    session_id: &str,
    role: &str,
    message: &str,
) -> rusqlite::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut conn = db.conn();
    let tx = conn.transaction()?;
    tx.execute(
            "INSERT INTO session_messages (session_id, role, message, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, role, message, &now],
        )?;
    let message_id = tx.last_insert_rowid();
    crate::db::context::record_message_revision_with_conn(
        &tx, session_id, message_id, role, message, &now,
    );
    bump_session_history_revision_with_conn(&tx, session_id)?;
    tx.commit()
}

fn redact_secret_strings(value: &mut Value, secret: &str, replacement: &str) -> bool {
    match value {
        Value::String(text) if text.contains(secret) => {
            *text = text.replace(secret, replacement);
            true
        }
        Value::Array(values) => values.iter_mut().fold(false, |changed, value| {
            redact_secret_strings(value, secret, replacement) || changed
        }),
        Value::Object(values) => values.values_mut().fold(false, |changed, value| {
            redact_secret_strings(value, secret, replacement) || changed
        }),
        _ => false,
    }
}

pub fn redact_session_message_secret(
    db: &Db,
    session_id: &str,
    user_id: &str,
    secret: &str,
    replacement: &str,
) -> Result<bool, String> {
    if secret.is_empty() {
        return Ok(false);
    }

    let mut conn = db.conn();
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    let owner = tx
        .query_row(
            "SELECT user_id FROM sessions WHERE id=?1",
            [session_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    if owner != user_id {
        return Err("Session does not belong to the current user".into());
    }

    let messages = {
        let mut statement = tx
            .prepare(
                "SELECT id, role, message, created_at FROM session_messages
                 WHERE session_id=?1 ORDER BY id ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([session_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?
    };

    let mut changed = false;
    for (message_id, role, message, created_at) in messages {
        let redacted = match serde_json::from_str::<Value>(&message) {
            Ok(mut value) => {
                if !redact_secret_strings(&mut value, secret, replacement) {
                    continue;
                }
                serde_json::to_string(&value).map_err(|error| error.to_string())?
            }
            Err(_) if message.contains(secret) => message.replace(secret, replacement),
            Err(_) => continue,
        };

        tx.execute(
            "UPDATE session_messages SET message=?1 WHERE id=?2",
            rusqlite::params![&redacted, message_id],
        )
        .map_err(|error| error.to_string())?;
        crate::db::context::record_message_revision_with_conn(
            &tx,
            session_id,
            message_id,
            &role,
            &redacted,
            &created_at,
        );
        let context: String = tx
            .query_row(
                "SELECT content FROM context_events
                 WHERE session_id=?1 AND source_kind='session_message' AND source_id=?2
                 ORDER BY active DESC, sequence DESC LIMIT 1",
                rusqlite::params![session_id, message_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if context != redacted {
            return Err("Could not redact the stored secret from session context".into());
        }
        changed = true;
    }

    if changed {
        bump_session_history_revision_with_conn(&tx, session_id)
            .map_err(|error| error.to_string())?;
    }
    tx.commit().map_err(|error| error.to_string())?;
    Ok(changed)
}

pub fn persist_compaction_checkpoint(db: &Db, session_id: &str, payload: &serde_json::Value) {
    let sanitized = crate::compaction::redact_compaction_value(payload.clone());
    persist_session_message(db, session_id, "compaction", &sanitized.to_string());
}

pub fn persist_assistant_snapshot(db: &Db, session_id: &str, message: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    let mut conn = db.conn();
    let result = (|| -> rusqlite::Result<()> {
        let tx = conn.transaction()?;
        let last_row: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, role FROM session_messages WHERE session_id = ?1 ORDER BY rowid DESC LIMIT 1",
                [session_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .ok();

        if let Some((message_id, role)) = last_row
            && role == "assistant"
        {
            tx.execute(
                "UPDATE session_messages SET message = ?1, created_at = ?2 WHERE id = ?3",
                rusqlite::params![message, &now, message_id],
            )?;
            crate::db::context::record_message_revision_with_conn(
                &tx,
                session_id,
                message_id,
                "assistant",
                message,
                &now,
            );
            bump_session_history_revision_with_conn(&tx, session_id)?;
            return tx.commit();
        }

        tx.execute(
            "INSERT INTO session_messages (session_id, role, message, created_at) VALUES (?1, 'assistant', ?2, ?3)",
            rusqlite::params![session_id, message, &now],
        )?;
        crate::db::context::record_message_revision_with_conn(
            &tx,
            session_id,
            tx.last_insert_rowid(),
            "assistant",
            message,
            &now,
        );
        bump_session_history_revision_with_conn(&tx, session_id)?;
        tx.commit()
    })();
    if let Err(error) = result {
        tracing::error!(%error, session_id, "failed to persist an assistant snapshot");
    }
}

fn collapse_persisted_messages_into_visible_turns<I>(
    messages: I,
    include_compactions: bool,
) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    fn flush_pending_assistant(
        visible: &mut Vec<(String, String)>,
        latest_snapshot: &mut Option<String>,
        recovery_segments: &mut Vec<String>,
        tool_results: &mut Vec<String>,
    ) {
        if let Some(message) = latest_snapshot.take() {
            recovery_segments.push(message);
        }
        if recovery_segments.is_empty() {
            // Guidance can insert a visible user row while an older tool is
            // still running. A late receipt must survive even if no new
            // assistant snapshot arrived before the next user boundary.
            if !tool_results.is_empty() {
                visible.push((
                    "tool_result".to_string(),
                    merge_assistant_segments(std::mem::take(tool_results)),
                ));
            }
            return;
        }
        // Receipts are separate durable rows, not replaceable assistant
        // snapshots. Keep their original payload/ordering across snapshots and
        // hidden continuations so history can reconcile them by tool_use_id.
        recovery_segments.append(tool_results);
        let merged = merge_assistant_segments(std::mem::take(recovery_segments));
        visible.push(("assistant".to_string(), merged));
    }

    let mut visible: Vec<(String, String)> = Vec::new();
    let mut latest_assistant_snapshot: Option<String> = None;
    let mut recovery_segments: Vec<String> = Vec::new();
    let mut tool_results: Vec<String> = Vec::new();

    for (role, message) in messages {
        match role.as_str() {
            "user" => {
                if is_hidden_continuation_message(&message) {
                    if let Some(snapshot) = latest_assistant_snapshot.take() {
                        recovery_segments.push(snapshot);
                    }
                    continue;
                }
                flush_pending_assistant(
                    &mut visible,
                    &mut latest_assistant_snapshot,
                    &mut recovery_segments,
                    &mut tool_results,
                );
                visible.push((role, message));
            }
            "assistant" => {
                // Assistant rows are persisted as rolling snapshots for the current turn.
                // Keep only the latest snapshot for each contiguous segment, then merge
                // segments across hidden continuation boundaries at flush time.
                latest_assistant_snapshot = Some(message);
            }
            "tool_result" => {
                tool_results.push(message);
            }
            "compaction" if include_compactions => {
                flush_pending_assistant(
                    &mut visible,
                    &mut latest_assistant_snapshot,
                    &mut recovery_segments,
                    &mut tool_results,
                );
                visible.push((role, message));
            }
            _ => {}
        }
    }

    flush_pending_assistant(
        &mut visible,
        &mut latest_assistant_snapshot,
        &mut recovery_segments,
        &mut tool_results,
    );

    visible
}

fn merge_assistant_segments(messages: Vec<String>) -> String {
    let mut merged: Option<serde_json::Value> = None;

    for message in messages {
        match serde_json::from_str::<serde_json::Value>(&message) {
            Ok(curr) => {
                if let Some(prev) = merged.as_mut() {
                    let curr_content = curr
                        .get("content")
                        .and_then(|c| c.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if let Some(prev_obj) = prev.as_object_mut() {
                        match prev_obj.get_mut("content").and_then(|c| c.as_array_mut()) {
                            Some(prev_content) => prev_content.extend(curr_content),
                            None => {
                                prev_obj.insert(
                                    "content".to_string(),
                                    serde_json::Value::Array(curr_content),
                                );
                            }
                        }
                        if let Some(usage) = curr.get("usage").cloned() {
                            prev_obj.insert("usage".to_string(), usage);
                        }
                        if let Some(model) = curr.get("model").cloned() {
                            prev_obj.insert("model".to_string(), model);
                        }
                    } else {
                        *prev = curr;
                    }
                } else {
                    merged = Some(curr);
                }
            }
            Err(_) => {
                merged = Some(serde_json::Value::String(message));
            }
        }
    }

    merged
        .map(|value| match value {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        })
        .unwrap_or_default()
}

/// Save a generated compaction summary for a session.
pub fn save_compaction_summary(
    db: &Db,
    session_id: &str,
    summary: &str,
    tokens_before: i64,
    model: &str,
) {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO compact_summaries (id, session_id, summary, tokens_before, model, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, session_id, summary, tokens_before, model, now],
    )
    .ok();
}

/// Load the most recent compaction summary for a session, if any.
pub fn load_compaction_summary(db: &Db, session_id: &str) -> Option<String> {
    let conn = db.conn();
    conn.query_row(
        "SELECT summary FROM compact_summaries WHERE session_id = ?1 ORDER BY created_at DESC LIMIT 1",
        [session_id],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .map(|summary| crate::compaction::redact_compaction_text(&summary))
}

pub fn load_latest_compaction_history(
    db: &Db,
    session_id: &str,
) -> Option<(String, i64, String, String)> {
    let conn = db.conn();
    conn.query_row(
        "SELECT summary, tokens_before, model, created_at
         FROM compact_summaries
         WHERE session_id = ?1
         ORDER BY created_at DESC LIMIT 1",
        [session_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )
    .ok()
    .map(|(summary, tokens, model, created_at)| {
        (
            crate::compaction::redact_compaction_text(&summary),
            tokens,
            model,
            created_at,
        )
    })
}

pub fn get_session_input_tokens(db: &Db, session_id: &str) -> i64 {
    let conn = db.conn();
    conn.query_row(
        "SELECT COALESCE(input_tokens, 0) FROM sessions WHERE id = ?1",
        [session_id],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0)
}

pub fn clear_session_claude_id(db: &Db, session_id: &str) {
    let conn = db.conn();
    conn.execute(
        "UPDATE sessions SET claude_session_id = NULL WHERE id = ?1",
        [session_id],
    )
    .ok();
}

pub fn fork_session(
    db: &Db,
    source_session_id: &str,
    new_session_id: &str,
    title: Option<&str>,
) -> Option<SessionMeta> {
    let source = get_session(db, source_session_id)?;
    let mut conn = db.conn();
    let tx = conn.transaction().ok()?;
    let now = chrono::Utc::now().to_rfc3339();
    let fork_title = title
        .map(str::to_string)
        .or_else(|| source.title.as_ref().map(|value| format!("{value} fork")))
        .unwrap_or_else(|| "Forked session".to_string());

    tx.execute(
        "INSERT INTO sessions (
            id, claude_session_id, project_path, title, created_at, updated_at,
            message_count, pinned, model, input_tokens, output_tokens, provider,
            user_id, runner_id, continuity_status, recovery_strategy, parent_session_id
         ) VALUES (?1, NULL, ?2, ?3, ?4, ?4, ?5, 0, ?6, ?7, ?8, ?9, ?10, ?11, 'idle', 'fork_and_compact', ?12)",
        rusqlite::params![
            new_session_id,
            source.project_path,
            fork_title,
            now,
            source.message_count,
            source.model,
            source.input_tokens,
            source.output_tokens,
            source.provider,
            source.user_id,
            source.runner_id,
            source_session_id,
        ],
    )
    .ok()?;

    tx.execute(
        "INSERT INTO session_messages (session_id, role, message, created_at)
         SELECT ?1, role, message, created_at
         FROM session_messages
         WHERE session_id = ?2
         ORDER BY rowid ASC",
        rusqlite::params![new_session_id, source_session_id],
    )
    .ok()?;

    tx.execute(
        "INSERT INTO compact_summaries (id, session_id, summary, tokens_before, model, created_at)
         SELECT lower(hex(randomblob(16))), ?1, summary, tokens_before, model, created_at
         FROM compact_summaries
         WHERE session_id = ?2
         ORDER BY created_at ASC",
        rusqlite::params![new_session_id, source_session_id],
    )
    .ok()?;

    tx.commit().ok()?;
    drop(conn);
    get_session(db, new_session_id)
}

pub fn delete_session(db: &Db, id: &str) {
    let conn = db.conn();
    conn.execute("DELETE FROM session_blocks WHERE session_id = ?1", [id])
        .ok();
    conn.execute("DELETE FROM shares WHERE session_id = ?1", [id])
        .ok();
    conn.execute("DELETE FROM session_messages WHERE session_id = ?1", [id])
        .ok();
    conn.execute("DELETE FROM sessions WHERE id = ?1", [id])
        .ok();
}

fn get_session_inner(conn: &rusqlite::Connection, id: &str) -> Option<SessionMeta> {
    conn.query_row("SELECT * FROM sessions WHERE id = ?1", [id], row_to_meta)
        .ok()
}

fn row_to_meta(row: &rusqlite::Row) -> rusqlite::Result<SessionMeta> {
    Ok(SessionMeta {
        id: row.get("id")?,
        claude_session_id: row.get::<_, Option<String>>("claude_session_id")?,
        project_path: row.get("project_path")?,
        title: row.get::<_, Option<String>>("title")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        message_count: row.get::<_, Option<i64>>("message_count")?.unwrap_or(0),
        pinned: row.get::<_, Option<i64>>("pinned")?.unwrap_or(0) == 1,
        model: row.get::<_, Option<String>>("model")?,
        input_tokens: row.get::<_, Option<i64>>("input_tokens")?.unwrap_or(0),
        output_tokens: row.get::<_, Option<i64>>("output_tokens")?.unwrap_or(0),
        provider: row
            .get::<_, Option<String>>("provider")?
            .unwrap_or_else(|| ProviderKind::Codex.as_str().to_string()),
        continuity_status: row
            .get::<_, Option<String>>("continuity_status")?
            .unwrap_or_else(|| "idle".to_string()),
        recovery_strategy: row.get::<_, Option<String>>("recovery_strategy")?,
        active_request_id: row.get::<_, Option<String>>("active_request_id")?,
        last_checkpoint_at: row.get::<_, Option<String>>("last_checkpoint_at")?,
        last_error: row.get::<_, Option<String>>("last_error")?,
        user_id: row.get::<_, Option<String>>("user_id")?,
        runner_id: row.get::<_, Option<String>>("runner_id").unwrap_or(None),
        session_owner_kind: row
            .get::<_, Option<String>>("session_owner_kind")
            .unwrap_or(None),
        orchestration_id: row
            .get::<_, Option<String>>("orchestration_id")
            .unwrap_or(None),
        orchestration_run_id: row
            .get::<_, Option<String>>("orchestration_run_id")
            .unwrap_or(None),
        orchestration_role: row
            .get::<_, Option<String>>("orchestration_role")
            .unwrap_or(None),
        parent_session_id: row
            .get::<_, Option<String>>("parent_session_id")
            .unwrap_or(None),
        active_plan_id: row
            .get::<_, Option<String>>("active_plan_id")
            .unwrap_or(None),
        requested_model: row
            .get::<_, Option<String>>("requested_model")
            .unwrap_or(None)
            .unwrap_or_else(|| "auto".to_string()),
        requested_reasoning_effort: row
            .get::<_, Option<String>>("requested_reasoning_effort")
            .unwrap_or(None)
            .unwrap_or_else(|| "auto".to_string()),
        resolved_reasoning_effort: row
            .get::<_, Option<String>>("resolved_reasoning_effort")
            .unwrap_or(None),
        routing_reason: row
            .get::<_, Option<String>>("routing_reason")
            .unwrap_or(None),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn seed_session(db: &Db, session_id: &str) {
        upsert_session(
            db,
            &UpsertSession {
                id: session_id.to_string(),
                claude_session_id: None,
                project_path: "/tmp/project".to_string(),
                title: Some("Test Session".to_string()),
                message_count: Some(0),
                pinned: Some(false),
                model: Some("claude-sonnet-4-6".to_string()),
                input_tokens: Some(0),
                output_tokens: Some(0),
                provider: Some(ProviderKind::Claude.as_str().to_string()),
                user_id: Some("user-1".to_string()),
                runner_id: None,
            },
        );
    }

    fn insert_message(db: &Db, session_id: &str, role: &str, content: serde_json::Value) {
        persist_session_message(
            db,
            session_id,
            role,
            &json!({"role": role, "content": content}).to_string(),
        );
    }

    #[test]
    fn history_revision_changes_when_transcript_changes() {
        let db = Db::open(":memory:");
        let session_id = "revision-session";
        seed_session(&db, session_id);

        assert_eq!(session_history_revision(&db, session_id), 0);
        persist_session_message(
            &db,
            session_id,
            "user",
            &json!({"content": [{"type": "text", "text": "secret value"}]}).to_string(),
        );
        assert_eq!(session_history_revision(&db, session_id), 1);

        persist_assistant_snapshot(&db, session_id, "partial");
        assert_eq!(session_history_revision(&db, session_id), 2);
        persist_assistant_snapshot(&db, session_id, "complete");
        assert_eq!(session_history_revision(&db, session_id), 3);

        assert!(
            redact_session_message_secret(&db, session_id, "user-1", "secret", "[redacted]",)
                .unwrap()
        );
        assert_eq!(session_history_revision(&db, session_id), 4);
    }

    #[test]
    fn stored_secrets_are_removed_from_transcript_and_context() {
        let db = Db::open(":memory:");
        let session_id = "secret-session";
        seed_session(&db, session_id);
        persist_session_message(
            &db,
            session_id,
            "user",
            &json!({
                "content": [{"type": "text", "text": "Use sudo password hunter2 and remember it"}]
            })
            .to_string(),
        );
        persist_session_message(
            &db,
            session_id,
            "user",
            &json!({"content": [{"type": "text", "text": "Store the password I sent earlier"}]})
                .to_string(),
        );

        assert!(
            redact_session_message_secret(
                &db,
                session_id,
                "user-1",
                "hunter2",
                "[stored securely as local secret: sudo]",
            )
            .unwrap()
        );

        let conn = db.conn();
        let transcript_secrets: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_messages
                 WHERE session_id=?1 AND instr(message, 'hunter2')>0",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        let context_secrets: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM context_events
                 WHERE session_id=?1 AND instr(content, 'hunter2')>0",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        let transcript_marker: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_messages
                 WHERE session_id=?1 AND instr(message, '[stored securely as local secret: sudo]')>0",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(transcript_secrets, 0);
        assert_eq!(context_secrets, 0);
        assert_eq!(transcript_marker, 1);
    }

    #[test]
    fn assistant_streaming_persists_one_transcript_and_context_snapshot() {
        let db = Db::open(":memory:");
        let session_id = "streaming-session";
        seed_session(&db, session_id);

        persist_assistant_snapshot(&db, session_id, "partial");
        persist_assistant_snapshot(&db, session_id, "partial answer");
        persist_assistant_snapshot(&db, session_id, "complete answer");

        let conn = db.conn();
        let transcript: (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), MAX(message) FROM session_messages
                 WHERE session_id=?1 AND role='assistant'",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let context: (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), MAX(content) FROM context_events
                 WHERE session_id=?1 AND source_kind='session_message'",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(transcript, (1, "complete answer".to_string()));
        assert_eq!(context, (1, "complete answer".to_string()));
    }

    #[test]
    fn active_plan_reconciliation_never_guesses_between_running_plans() {
        let db = Db::open(":memory:");
        seed_session(&db, "plan-session");
        let first = crate::db::plans::create_plan(
            &db,
            "First",
            None,
            &[json!({"text": "A"})],
            Some("plan-session"),
            Some("/tmp/project"),
            "user-1",
        )
        .unwrap();
        let second = crate::db::plans::create_plan(
            &db,
            "Second",
            None,
            &[json!({"text": "B"})],
            Some("plan-session"),
            Some("/tmp/project"),
            "user-1",
        )
        .unwrap();
        crate::db::plans::update_plan_execution(&db, &first.meta.id, "start").unwrap();
        crate::db::plans::update_plan_execution(&db, &second.meta.id, "start").unwrap();
        set_session_active_plan(&db, "plan-session", None).unwrap();

        assert_eq!(reconcile_session_active_plan(&db, "plan-session"), None);
        assert_eq!(
            get_session(&db, "plan-session").unwrap().active_plan_id,
            None
        );
    }

    #[test]
    fn active_plan_reconciliation_repairs_one_running_plan() {
        let db = Db::open(":memory:");
        seed_session(&db, "plan-session");
        let plan = crate::db::plans::create_plan(
            &db,
            "Only running plan",
            None,
            &[json!({"text": "A"})],
            Some("plan-session"),
            Some("/tmp/project"),
            "user-1",
        )
        .unwrap();
        crate::db::plans::update_plan_execution(&db, &plan.meta.id, "start").unwrap();
        set_session_active_plan(&db, "plan-session", None).unwrap();

        assert_eq!(
            reconcile_session_active_plan(&db, "plan-session").as_deref(),
            Some(plan.meta.id.as_str())
        );
    }

    #[test]
    fn admission_upsert_preserves_existing_context_usage() {
        let db = Db::open(":memory:");
        seed_session(&db, "usage-session");
        upsert_session(
            &db,
            &UpsertSession {
                id: "usage-session".to_string(),
                claude_session_id: None,
                project_path: "/tmp/project".to_string(),
                title: None,
                message_count: None,
                pinned: None,
                model: None,
                input_tokens: Some(861_000),
                output_tokens: Some(2_000),
                provider: None,
                user_id: Some("user-1".to_string()),
                runner_id: None,
            },
        );
        upsert_session(
            &db,
            &UpsertSession {
                id: "usage-session".to_string(),
                claude_session_id: None,
                project_path: "/tmp/project".to_string(),
                title: Some("Next turn".to_string()),
                message_count: Some(2),
                pinned: None,
                model: Some("gpt-5.6-sol".to_string()),
                input_tokens: None,
                output_tokens: None,
                provider: Some(ProviderKind::Codex.as_str().to_string()),
                user_id: Some("user-1".to_string()),
                runner_id: None,
            },
        );

        let session = get_session(&db, "usage-session").expect("session");
        assert_eq!(session.input_tokens, 861_000);
        assert_eq!(session.output_tokens, 2_000);
    }

    #[test]
    fn legacy_session_without_provider_defaults_to_codex() {
        let db = Db::open(":memory:");
        seed_session(&db, "legacy-provider-session");
        db.conn()
            .execute(
                "UPDATE sessions SET provider = NULL WHERE id = ?1",
                ["legacy-provider-session"],
            )
            .expect("clear legacy provider");

        let session = get_session(&db, "legacy-provider-session").expect("session");
        assert_eq!(session.provider, ProviderKind::Codex.as_str());
    }

    #[test]
    fn interactive_session_list_keeps_legacy_provider_rows_but_hides_empty_and_internal_rows() {
        let db = Db::open(":memory:");
        seed_session(&db, "empty");
        seed_session(&db, "visible");
        seed_session(&db, "legacy-provider");
        seed_session(&db, "internal");
        insert_message(
            &db,
            "visible",
            "user",
            json!([{"type": "text", "text": "hello"}]),
        );
        db.conn()
            .execute(
                "UPDATE sessions SET claude_session_id='provider-thread-1' WHERE id='legacy-provider'",
                [],
            )
            .unwrap();
        insert_message(
            &db,
            "internal",
            "user",
            json!([{"type": "text", "text": "hidden"}]),
        );
        set_session_owner(&db, "internal", "goal_worker", Some("goal-1"), None, None);

        let sessions = list_sessions(&db, Some("user-1"), None, None);
        let ids = sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"visible"));
        assert!(ids.contains(&"legacy-provider"));
    }

    #[test]
    fn session_owner_metadata_round_trips() {
        let db = Db::open(":memory:");
        let session_id = "orchestration-session";
        seed_session(&db, session_id);

        set_session_owner(
            &db,
            session_id,
            "orchestration_coordinator",
            Some("definition-1"),
            Some("run-1"),
            Some("coordinator"),
        );

        let session = get_session(&db, session_id).unwrap();
        assert_eq!(
            session.session_owner_kind.as_deref(),
            Some("orchestration_coordinator")
        );
        assert_eq!(session.orchestration_id.as_deref(), Some("definition-1"));
        assert_eq!(session.orchestration_run_id.as_deref(), Some("run-1"));
        assert_eq!(session.orchestration_role.as_deref(), Some("coordinator"));
    }

    #[test]
    fn interactive_session_list_excludes_internal_goal_sessions() {
        let db = Db::open(":memory:");
        seed_session(&db, "interactive-session");
        insert_message(
            &db,
            "interactive-session",
            "user",
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "visible conversation"}]
            }),
        );
        seed_session(&db, "goal-session");
        set_session_owner(
            &db,
            "goal-session",
            "goal_worker",
            None,
            None,
            Some("worker"),
        );

        let sessions = list_sessions(&db, Some("user-1"), None, None);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "interactive-session");
        assert!(get_session(&db, "goal-session").is_some());
    }

    #[test]
    fn migrations_preserve_sessions_with_legacy_orchestration_metadata() {
        let db = Db::open(":memory:");
        seed_session(&db, "legacy-orchestration-session");
        insert_message(
            &db,
            "legacy-orchestration-session",
            "user",
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "durable conversation"}]
            }),
        );
        set_session_owner(
            &db,
            "legacy-orchestration-session",
            "orchestration_coordinator",
            Some("definition-1"),
            Some("run-1"),
            Some("coordinator"),
        );

        {
            let conn = db.conn();
            crate::db::schema::run_migrations(&conn);
        }

        assert!(get_session(&db, "legacy-orchestration-session").is_some());
        let message_count = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM session_messages WHERE session_id=?1",
                ["legacy-orchestration-session"],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(message_count, 1);
    }

    #[test]
    fn migrations_classify_legacy_goal_sessions_from_worker_metadata() {
        let db = Db::open(":memory:");
        seed_session(&db, "legacy-goal-session");
        insert_message(
            &db,
            "legacy-goal-session",
            "user",
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "approved objective"}],
                "flamel": {"hiddenContinuation": false, "worker": true}
            }),
        );
        {
            let conn = db.conn();
            crate::db::schema::run_migrations(&conn);
        }

        let session = get_session(&db, "legacy-goal-session").unwrap();
        assert_eq!(session.session_owner_kind.as_deref(), Some("goal_worker"));
        assert!(list_sessions(&db, Some("user-1"), None, None).is_empty());
    }

    #[test]
    fn fork_session_copies_history_summary_and_lineage() {
        let db = Db::open(":memory:");
        let source_id = "source-session";
        let fork_id = "fork-session";
        seed_session(&db, source_id);
        insert_message(
            &db,
            source_id,
            "user",
            json!([{"type": "text", "text": "Original request"}]),
        );
        insert_message(
            &db,
            source_id,
            "assistant",
            json!([{"type": "text", "text": "Original answer"}]),
        );
        save_compaction_summary(&db, source_id, "Existing summary", 2048, "gpt-5.4-mini");

        let fork = fork_session(&db, source_id, fork_id, Some("Fork title")).unwrap();

        assert_eq!(fork.parent_session_id.as_deref(), Some(source_id));
        assert_eq!(fork.title.as_deref(), Some("Fork title"));
        assert_eq!(fork.claude_session_id, None);
        assert_eq!(fork.recovery_strategy.as_deref(), Some("fork_and_compact"));

        let messages = load_persisted_session_messages(&db, fork_id);
        assert_eq!(messages.len(), 2);
        assert!(
            messages
                .iter()
                .any(|(_, message)| message.contains("Original request"))
        );
        assert!(
            messages
                .iter()
                .any(|(_, message)| message.contains("Original answer"))
        );
        assert_eq!(
            load_compaction_summary(&db, fork_id).as_deref(),
            Some("Existing summary")
        );
    }

    #[test]
    fn recovery_context_uses_summary_and_includes_all_turns() {
        let db = Db::open(":memory:");
        let session_id = "session-1";
        seed_session(&db, session_id);
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Earlier request"}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "Earlier reply"}]),
        );
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Current request"}]),
        );
        save_compaction_summary(
            &db,
            session_id,
            "Project summary",
            1200,
            "claude-haiku-4-5-20251001",
        );

        let context = load_session_recovery_context(&db, session_id).unwrap();

        assert!(context.contains("Project summary"));
        assert!(context.contains("Earlier request"));
        assert!(context.contains("Earlier reply"));
        assert!(context.contains("Current request"));
        assert!(context.contains("The current user message outside this block is authoritative"));
        assert!(
            context.find("Current request").unwrap()
                < context.rfind("OLDER CHECKPOINT BACKGROUND").unwrap()
        );
    }

    #[test]
    fn recovery_context_falls_back_to_recent_turns_only() {
        let db = Db::open(":memory:");
        let session_id = "session-2";
        seed_session(&db, session_id);
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Need a fix"}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "Working on it"}]),
        );
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Continue"}]),
        );

        let context = load_session_recovery_context(&db, session_id).unwrap();

        assert!(context.contains("Need a fix"));
        assert!(context.contains("Working on it"));
        assert!(context.contains("Continue"));
        assert!(!context.contains("Compaction summary:"));
        assert!(context.contains("this context is hidden recovery state"));
        assert!(context.contains("The current user message outside this block is authoritative"));
        assert!(context.contains("Do not restart from, re-answer, or re-derive"));
        assert!(
            context.contains("the newest persisted user turn replaces the interrupted request")
        );
        assert!(context.contains(
            "Do not reopen or change plan items solely because recovery context mentions a plan"
        ));
    }

    #[test]
    fn collapses_consecutive_assistant_snapshots_in_recovery_context() {
        let db = Db::open(":memory:");
        let session_id = "session-3";
        seed_session(&db, session_id);
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Push it"}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "Pushed"}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "Pushed to"}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "Pushed to gitea/main."}]),
        );
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "next"}]),
        );

        let context = load_session_recovery_context(&db, session_id).unwrap();

        assert!(!context.contains("ASSISTANT: Pushed\n\nASSISTANT: Pushed to"));
        assert!(context.contains("ASSISTANT: Pushed to gitea/main."));
    }

    #[test]
    fn hidden_continuation_messages_are_filtered_from_history_and_recovery() {
        let db = Db::open(":memory:");
        let session_id = "session-4";
        seed_session(&db, session_id);
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Real request"}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "Partial progress recap"}]),
        );
        persist_session_message(
            &db,
            session_id,
            "user",
            &json!({
                "role": "user",
                "content": [{"type": "text", "text": "Keep working. Do not repeat or re-summarize anything you already wrote above — the user has already read it. Pick up from the next concrete action and drive the task to completion."}],
                "flamel": {"hiddenContinuation": true},
            }).to_string(),
        );

        let visible_messages = load_persisted_session_messages(&db, session_id);
        assert_eq!(visible_messages.len(), 2);
        assert!(
            visible_messages
                .iter()
                .all(|(_, msg)| !msg.contains("hiddenContinuation"))
        );

        let context = load_session_recovery_context(&db, session_id).unwrap();
        assert!(context.contains("Real request"));
        assert!(context.contains("Partial progress recap"));
        assert!(!context.contains("Keep working. Do not repeat or re-summarize"));
    }

    #[test]
    fn compaction_checkpoints_survive_ui_history_without_entering_provider_context() {
        let db = Db::open(":memory:");
        let session_id = "session-compaction-history";
        seed_session(&db, session_id);
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Keep implementing"}]),
        );
        let payload = json!({
            "runId": "compact-run-1",
            "phase": "complete",
            "summary": "Current session: implementation is active.",
            "preTokens": 229_000,
        });
        persist_compaction_checkpoint(&db, session_id, &payload);
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "Continuing now"}]),
        );

        let history = load_persisted_session_history(&db, session_id);
        assert_eq!(history.len(), 3);
        assert_eq!(history[1].0, "compaction");
        assert!(history[1].1.contains("compact-run-1"));

        let provider_messages = load_all_messages_for_compaction(&db, session_id);
        assert_eq!(provider_messages.len(), 2);
        assert!(
            provider_messages
                .iter()
                .all(|(role, _)| role != "compaction")
        );
    }

    #[test]
    fn visible_session_messages_ignore_hidden_continuations() {
        let db = Db::open(":memory:");
        let session_id = "session-4a";
        seed_session(&db, session_id);

        assert!(!has_visible_session_messages(&db, session_id));

        persist_session_message(
            &db,
            session_id,
            "user",
            &json!({
                "role": "user",
                "content": [{"type": "text", "text": "Continue from where you left off."}],
                "flamel": {"hiddenContinuation": true},
            })
            .to_string(),
        );
        assert!(!has_visible_session_messages(&db, session_id));

        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Real request"}]),
        );
        assert!(has_visible_session_messages(&db, session_id));
    }

    #[test]
    fn latest_visible_user_message_respects_approval_time_and_hidden_turns() {
        let db = Db::open(":memory:");
        let session_id = "session-plan-approval";
        seed_session(&db, session_id);
        let conn = db.conn();
        for (message, created_at) in [
            (
                json!([{"type": "text", "text": "Create a plan"}]).to_string(),
                "2026-07-11T10:00:00Z",
            ),
            (
                json!({
                    "role": "user",
                    "content": [{"type": "text", "text": "Continue automatically"}],
                    "flamel": {"hiddenContinuation": true},
                })
                .to_string(),
                "2026-07-11T10:02:00Z",
            ),
            (
                json!([{"type": "text", "text": "Start the plan"}]).to_string(),
                "2026-07-11T10:03:00Z",
            ),
        ] {
            conn.execute(
                "INSERT INTO session_messages (session_id, role, message, created_at) VALUES (?1, 'user', ?2, ?3)",
                rusqlite::params![session_id, message, created_at],
            )
            .unwrap();
        }
        drop(conn);

        assert_eq!(
            latest_visible_user_message_after(&db, session_id, "2026-07-11T10:01:00Z").as_deref(),
            Some("Start the plan")
        );
        assert!(
            latest_visible_user_message_after(&db, session_id, "2026-07-11T10:04:00Z").is_none()
        );
    }

    #[test]
    fn hidden_continuation_segments_merge_without_replaying_every_snapshot() {
        let db = Db::open(":memory:");
        let session_id = "session-4b";
        seed_session(&db, session_id);
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Real request"}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "First chunk. "}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "First chunk, latest snapshot. "}]),
        );
        persist_session_message(
            &db,
            session_id,
            "user",
            &json!({
                "role": "user",
                "content": [{"type": "text", "text": "Continue only if there is concrete unfinished work."}],
                "flamel": {"hiddenContinuation": true},
            })
            .to_string(),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "Recovered chunk."}]),
        );

        let visible_messages = load_persisted_session_messages(&db, session_id);
        assert_eq!(visible_messages.len(), 2);
        assert_eq!(visible_messages[0].0, "user");
        assert_eq!(visible_messages[1].0, "assistant");
        assert!(
            visible_messages[1]
                .1
                .contains("First chunk, latest snapshot.")
        );
        assert!(visible_messages[1].1.contains("Recovered chunk."));
        assert!(
            !visible_messages[1]
                .1
                .contains("Continue only if there is concrete unfinished work.")
        );
    }

    #[test]
    fn tool_result_noise_does_not_expand_visible_history() {
        let db = Db::open(":memory:");
        let session_id = "session-4c";
        seed_session(&db, session_id);
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Ship it"}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "Working..."}]),
        );
        persist_session_message(
            &db,
            session_id,
            "tool_result",
            &json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "tool-1", "content": "intermediate output"}],
            })
            .to_string(),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "Finished."}]),
        );

        let visible_messages = load_persisted_session_messages(&db, session_id);
        assert_eq!(visible_messages.len(), 2);
        assert_eq!(visible_messages[0].0, "user");
        assert_eq!(visible_messages[1].0, "assistant");
        assert!(visible_messages[1].1.contains("Finished."));
        // Receipts remain available for tool reconciliation, but do not add
        // extra visible turns or replay an obsolete assistant snapshot.
        assert!(visible_messages[1].1.contains("intermediate output"));
        assert!(!visible_messages[1].1.contains("Working..."));
    }

    #[test]
    fn rolling_snapshots_preserve_terminal_tool_receipts_and_metadata() {
        let tool = json!({"type": "tool_use", "id": "verify-1", "name": "shell", "input": {"action": "verify"}});
        let receipt = json!({
            "type": "tool_result", "tool_use_id": "verify-1",
            "content": "verification passed", "is_error": false, "is_delta": false,
            "presentation": {"version": 1, "text": "verification passed"},
            "artifacts": [{"id": "verification-log"}]
        });
        let rows = vec![
            (
                "assistant".into(),
                json!({"role": "assistant", "content": [tool.clone()]}).to_string(),
            ),
            (
                "tool_result".into(),
                json!({"role": "user", "content": [receipt.clone()]}).to_string(),
            ),
            (
                "assistant".into(),
                json!({"role": "assistant", "model": "gpt-6-astra", "content": [
                    tool, {"type": "tool_use", "id": "next", "name": "shell", "input": {}},
                    {"type": "text", "text": "Next check"}
                ]})
                .to_string(),
            ),
        ];
        let visible = collapse_persisted_messages_into_visible_turns(rows, false);
        assert_eq!(visible.len(), 1);
        let message: Value = serde_json::from_str(&visible[0].1).unwrap();
        let content = message["content"].as_array().unwrap();
        assert_eq!(content.len(), 4);
        assert_eq!(content[3], receipt);
        assert_eq!(message["role"], "assistant");
        assert_eq!(message["model"], "gpt-6-astra");
        assert_eq!(
            content
                .iter()
                .filter(|item| item["type"] == "tool_use")
                .count(),
            2
        );
    }

    #[test]
    fn tool_receipts_before_envelopes_survive_hidden_continuations() {
        let failed = json!({"type": "tool_result", "tool_use_id": "early", "content": "failed", "is_error": true});
        let partial = json!({"type": "tool_result", "tool_use_id": "active", "content": "still running", "is_delta": true});
        let rows = vec![
            ("tool_result".into(), json!({"content": [failed.clone()]}).to_string()),
            ("assistant".into(), json!({"content": [{"type": "tool_use", "id": "early", "name": "shell"}]}).to_string()),
            ("user".into(), json!({"content": [{"type": "text", "text": "hidden"}], "flamel": {"hiddenContinuation": true}}).to_string()),
            ("assistant".into(), json!({"content": [{"type": "tool_use", "id": "active", "name": "shell"}]}).to_string()),
            ("tool_result".into(), json!({"content": [partial.clone()]}).to_string()),
            ("assistant".into(), json!({"content": [
                {"type": "tool_use", "id": "active", "name": "shell"},
                {"type": "text", "text": "Latest snapshot"}
            ]}).to_string()),
        ];
        let visible = collapse_persisted_messages_into_visible_turns(rows, false);
        assert_eq!(visible.len(), 1);
        let message: Value = serde_json::from_str(&visible[0].1).unwrap();
        let content = message["content"].as_array().unwrap();
        assert_eq!(content.len(), 5);
        assert_eq!(content[3], failed);
        assert_eq!(content[4], partial);
        assert!(!visible[0].1.contains("hidden"));
    }

    #[test]
    fn tool_receipts_do_not_leak_across_visible_turn_or_compaction_boundaries() {
        for boundary in ["user", "compaction"] {
            let rows = vec![
                ("assistant".into(), json!({"content": [{"type": "tool_use", "id": "before", "name": "shell"}]}).to_string()),
                ("tool_result".into(), json!({"content": [{"type": "tool_result", "tool_use_id": "before", "content": "complete"}]}).to_string()),
                (boundary.into(), json!({"content": [{"type": "text", "text": "Boundary"}]}).to_string()),
                ("assistant".into(), json!({"content": [{"type": "tool_use", "id": "after", "name": "shell"}]}).to_string()),
            ];
            let visible = collapse_persisted_messages_into_visible_turns(rows, true);
            assert_eq!(visible.len(), 3);
            assert!(visible[0].1.contains("complete"));
            assert!(!visible[2].1.contains("complete"));
            assert!(!visible[2].1.contains("tool_result"));
        }
    }

    #[test]
    fn late_tool_receipt_survives_between_guidance_messages_without_a_snapshot() {
        let rows = vec![
            ("assistant".into(), json!({"content": [{"type": "tool_use", "id": "slow", "name": "shell"}]}).to_string()),
            ("user".into(), json!({"content": [{"type": "text", "text": "First guidance"}]}).to_string()),
            ("tool_result".into(), json!({"content": [{"type": "tool_result", "tool_use_id": "slow", "content": "finished", "is_error": false}]}).to_string()),
            ("user".into(), json!({"content": [{"type": "text", "text": "Second guidance"}]}).to_string()),
        ];
        let visible = collapse_persisted_messages_into_visible_turns(rows, true);
        assert_eq!(
            visible.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
            vec!["assistant", "user", "tool_result", "user"]
        );
        let receipt: Value = serde_json::from_str(&visible[2].1).unwrap();
        assert_eq!(receipt["content"][0]["tool_use_id"], "slow");
        assert_eq!(receipt["content"][0]["content"], "finished");
    }

    #[test]
    fn recovery_context_drops_trailing_resumption_recaps() {
        let db = Db::open(":memory:");
        let session_id = "session-5";
        seed_session(&db, session_id);
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Build locomotion"}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "I’m loading the active plan, current repo state, and existing code so I can resume with real implementation and only mark items done when code is actually landed."}]),
        );
        persist_session_message(
            &db,
            session_id,
            "user",
            &json!({
                "role": "user",
                "content": [{"type": "text", "text": "Continue only if there is concrete unfinished work."}],
                "flamel": {"hiddenContinuation": true},
            }).to_string(),
        );

        let context = load_session_recovery_context(&db, session_id).unwrap();
        assert!(context.contains("Build locomotion"));
        assert!(!context.contains("I’m loading the active plan"));
    }

    #[test]
    fn resumption_recap_detection_ignores_real_work_updates() {
        let recap = json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "I’m checking the active plan and repo state now so I can move the code forward with real implementation."}]
        }).to_string();
        let stall_recap = json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "I’m reading the plan items and existing code before I continue."}]
        }).to_string();
        let real_update = json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "Implemented locomotion stamina, added acceleration, and patched shard reconciliation."}]
        }).to_string();

        assert!(looks_like_resumption_recap(&recap));
        assert!(looks_like_resumption_recap(&stall_recap));
        assert!(!looks_like_resumption_recap(&real_update));
    }

    #[test]
    fn message_summary_includes_tool_inputs_and_results() {
        let message = json!({
            "role": "assistant",
            "content": [
                {"type": "text", "text": "Tracing the recovery summary path."},
                {
                    "type": "tool_use",
                    "name": "file_read",
                    "input": {"action": "read", "file": "src-server/src/claude/session_pool.rs"}
                },
                {
                    "type": "tool_result",
                    "tool_use_id": "tool-1",
                    "content": "loaded the recovery prompt",
                    "is_error": false
                }
            ]
        })
        .to_string();

        let summary = extract_message_text_for_summary(&message);
        assert!(summary.contains("Tracing the recovery summary path."));
        assert!(summary.contains("file_read"));
        assert!(summary.contains("action=read"));
        assert!(summary.contains("file=src-server/src/claude/session_pool.rs"));
        assert!(summary.contains("tool-1"));
        assert!(summary.contains("loaded the recovery prompt"));
    }

    #[test]
    fn session_context_summary_keeps_compact_tool_context() {
        let db = Db::open(":memory:");
        let session_id = "session-7";
        seed_session(&db, session_id);
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Fix the ugly recovery digest"}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([
                {"type": "text", "text": "Tracing the resume digest path."},
                {
                    "type": "tool_use",
                    "name": "code_nav",
                    "input": {"action": "search", "pattern": "resume digest", "path": "src-server"}
                }
            ]),
        );
        persist_session_message(
            &db,
            session_id,
            "tool_result",
            &json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "tool-9",
                    "content": "found summarize_codex_recovery_context in session_pool.rs",
                    "is_error": false
                }],
            })
            .to_string(),
        );

        let summary = load_session_context_summary(&db, session_id);
        assert!(summary.contains("code_nav"));
        assert!(summary.contains("pattern=resume digest"));
        assert!(summary.contains("tool-9"));
        assert!(summary.contains("session_pool.rs"));
    }

    #[test]
    fn session_context_summary_preserves_terminal_assistant_result() {
        let db = Db::open(":memory:");
        let session_id = "session-terminal-result";
        seed_session(&db, session_id);
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Integrate and deploy Ponytail"}]),
        );

        let mut content = vec![json!({
            "type": "text",
            "text": "Starting the Ponytail integration and tracing the existing controls."
        })];
        for index in 0..24 {
            content.push(json!({
                "type": "tool_use",
                "name": "file_read",
                "input": {"file": format!("frontend/src/components/LongInspection{index}.tsx")}
            }));
            content.push(json!({
                "type": "tool_result",
                "tool_use_id": format!("tool-{index}"),
                "content": format!("inspected implementation detail {index}"),
                "is_error": false
            }));
        }
        content.push(json!({
            "type": "text",
            "text": "Implemented Ponytail, verified interrupt visibility, and all checks passed."
        }));
        insert_message(
            &db,
            session_id,
            "assistant",
            serde_json::Value::Array(content),
        );

        let summary = load_session_context_summary(&db, session_id);
        let assistant = summary
            .lines()
            .find(|line| line.starts_with("ASSISTANT: "))
            .expect("assistant summary");
        assert!(assistant.contains("Starting the Ponytail integration"));
        assert!(assistant.contains(
            "Implemented Ponytail, verified interrupt visibility, and all checks passed."
        ));
        assert!(assistant.chars().count() <= "ASSISTANT: ".chars().count() + 600);
    }

    #[test]
    fn continuity_updates_are_persisted() {
        let db = Db::open(":memory:");
        let session_id = "session-4";
        seed_session(&db, session_id);

        update_session_continuity(
            &db,
            session_id,
            "active",
            Some("provider_resume"),
            Some("req-123"),
            true,
            None,
        );

        let session = get_session(&db, session_id).unwrap();
        assert_eq!(session.provider, "claude");
        assert_eq!(session.continuity_status, "active");
        assert_eq!(
            session.recovery_strategy.as_deref(),
            Some("provider_resume")
        );
        assert_eq!(session.active_request_id.as_deref(), Some("req-123"));
        assert!(session.last_checkpoint_at.is_some());
    }

    #[test]
    fn recovery_context_includes_accepted_plan_artifact_summary() {
        let db = Db::open(":memory:");
        let session_id = "session-plan-context";
        seed_session(&db, session_id);
        let plan = crate::db::plans::create_plan(
            &db,
            "Continue accepted plan",
            Some("Project plan description"),
            &[
                json!({
                    "text": "Carry the active item into recovery context",
                    "notes": "Extremely detailed notes should not be dumped into the recovery prompt",
                    "section": "Planning"
                }),
                json!({"text": "Keep another item available"}),
            ],
            Some(session_id),
            Some("/tmp/project"),
            "user-1",
        )
        .expect("plan created");
        crate::db::plans::update_plan_execution(&db, &plan.meta.id, "start")
            .expect("plan approved");
        crate::db::plans::upsert_plan_item(
            &db,
            &plan.meta.id,
            &plan.items[0].id,
            Some("in_progress"),
            None,
            None,
            None,
            None,
        )
        .expect("active item set");

        let parts = load_session_recovery_context_parts(&db, session_id).unwrap();
        assert!(
            parts
                .plan_context
                .as_deref()
                .unwrap_or_default()
                .contains("Continue accepted plan")
        );
        let context = format_recovery_context_from_parts(&parts).unwrap();
        assert!(context.contains("ACCEPTED PLAN CONTEXT"));
        assert!(
            context.contains(
                "ACCEPTED PLAN CONTEXT for plan identity, progress, and active items only"
            )
        );
        assert!(context.contains(
            "Active item: Planning: [in_progress, Auto-only reasoning floor=medium] Carry the active item"
        ));
        assert!(!context.contains("Extremely detailed notes"));
    }

    #[test]
    fn recovery_context_parts_expose_recent_turn_metadata() {
        let db = Db::open(":memory:");
        let session_id = "session-6";
        seed_session(&db, session_id);
        insert_message(
            &db,
            session_id,
            "user",
            json!([{"type": "text", "text": "Inspect resume path"}]),
        );
        insert_message(
            &db,
            session_id,
            "assistant",
            json!([{"type": "text", "text": "Checking session bootstrap."}]),
        );

        let parts = load_session_recovery_context_parts(&db, session_id).unwrap();
        assert_eq!(parts.recent_turn_count, 2);
        assert!(parts.recent_turns.contains("Inspect resume path"));
        assert!(parts.recent_turns.contains("Checking session bootstrap."));
    }

    #[test]
    fn summarized_recovery_context_prefers_digest_over_raw_turn_dump() {
        let parts = RecoveryContextParts {
            compaction_summary: Some("Earlier state".to_string()),
            plan_context: None,
            recent_turns: "USER: Raw turn\n\nASSISTANT: Raw reply".to_string(),
            recent_turn_count: 2,
        };

        let context = format_summarized_recovery_context(
            &parts,
            "Completed: found the failing resume path\nIn progress: patching Codex recovery\nNext action: wire the digest into thread/start\nKey artifacts: src-server/src/claude/session_pool.rs",
        )
        .unwrap();

        assert!(context.contains("Earlier state"));
        assert!(
            context
                .contains("NEWEST RECOVERY DIGEST (authoritative for completed and current work)")
        );
        assert!(context.contains("Next action: wire the digest into thread/start"));
        assert!(
            context.contains("the newest persisted user turn replaces the interrupted request")
        );
        assert!(
            context
                .find("Next action: wire the digest into thread/start")
                .unwrap()
                < context.rfind("OLDER CHECKPOINT BACKGROUND").unwrap()
        );
        assert!(!context.contains("NEWEST PERSISTED TURNS"));
        assert!(!context.contains("USER: Raw turn"));
    }
}
