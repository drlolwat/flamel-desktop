use crate::db::Db;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheckpointState {
    pub version: u32,
    pub goals: Vec<String>,
    pub decisions: Vec<String>,
    pub completed_work: Vec<String>,
    pub remaining_work: Vec<String>,
    pub key_artifacts: Vec<String>,
    pub source_event_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContextRetrieval {
    pub event_id: String,
    pub role: String,
    pub content: String,
    pub score: f64,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContextDiagnostics {
    pub total_events: i64,
    pub active_events: i64,
    pub archived_revisions: i64,
    pub checkpoint_count: i64,
    pub active_checkpoint_id: Option<String>,
    pub checkpoint_model: Option<String>,
    pub checkpoint_created_at: Option<String>,
    pub estimated_tokens_after: Option<i64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextMaintenance {
    pub archived_revisions: usize,
    pub reclaimed_pages: usize,
}

fn content_hash(content: &str) -> String {
    hex::encode(Sha256::digest(content.as_bytes()))
}

pub fn record_message_revision_with_conn(
    conn: &Connection,
    session_id: &str,
    message_id: i64,
    role: &str,
    content: &str,
    created_at: &str,
) {
    let hash = content_hash(content);
    let source_id = message_id.to_string();
    let existing: Option<(i64, String)> = conn
        .query_row(
            "SELECT sequence, content_hash FROM context_events
             WHERE session_id=?1 AND source_kind='session_message' AND source_id=?2
             ORDER BY active DESC, sequence DESC LIMIT 1",
            params![session_id, &source_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();

    if let Some((sequence, existing_hash)) = existing {
        if existing_hash == hash {
            let _ = conn.execute(
                "UPDATE context_events SET role=?1, created_at=?2, active=1 WHERE sequence=?3",
                params![role, created_at, sequence],
            );
            return;
        }

        // The transcript row is the durable source of truth. During a live
        // turn its materialized context event is updated in place so token
        // streaming cannot create an unbounded series of full-message copies.
        // Keeping the event_id stable also keeps checkpoint references valid.
        let _ = conn.execute(
            "DELETE FROM context_events
             WHERE session_id=?1 AND source_kind='session_message' AND source_id=?2
               AND content_hash=?3 AND sequence<>?4",
            params![session_id, &source_id, &hash, sequence],
        );
        let _ = conn.execute(
            "UPDATE context_events
             SET role=?1, content=?2, content_hash=?3, created_at=?4, active=1
             WHERE sequence=?5",
            params![role, content, hash, created_at, sequence],
        );
        return;
    }

    let event_id = uuid::Uuid::new_v4().to_string();
    let _ = conn.execute(
        "INSERT OR IGNORE INTO context_events
         (event_id, session_id, source_kind, source_id, role, content, content_hash, created_at)
         VALUES (?1, ?2, 'session_message', ?3, ?4, ?5, ?6, ?7)",
        params![
            event_id, session_id, source_id, role, content, hash, created_at
        ],
    );
}

/// Incrementally removes legacy partial snapshots created by releases that
/// stored a complete context event on every streamed assistant update.
pub fn maintain_context_ledger(db: &Db, batch_size: usize) -> ContextMaintenance {
    let batch_size = batch_size.clamp(1, 10_000) as i64;
    let conn = db.conn();
    let archived_revisions = conn
        .execute(
            "DELETE FROM context_events WHERE sequence IN (
                SELECT sequence FROM context_events
                WHERE source_kind='session_message' AND active=0
                ORDER BY sequence ASC LIMIT ?1
             )",
            [batch_size],
        )
        .unwrap_or(0);
    let pages_before = conn
        .query_row("PRAGMA freelist_count", [], |row| row.get::<_, usize>(0))
        .unwrap_or(0);
    // This is a no-op on older databases until the one-time supervised
    // compaction converts them to auto_vacuum=INCREMENTAL.
    let _ = conn.execute_batch("PRAGMA incremental_vacuum(2000)");
    let pages_after = conn
        .query_row("PRAGMA freelist_count", [], |row| row.get::<_, usize>(0))
        .unwrap_or(pages_before);
    ContextMaintenance {
        archived_revisions,
        reclaimed_pages: pages_before.saturating_sub(pages_after),
    }
}

pub fn backfill_session_ledger(db: &Db, session_id: &str) {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT id, role, message, created_at FROM session_messages
         WHERE session_id=?1 ORDER BY id ASC",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return,
    };
    let rows = match stmt.query_map([session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    }) {
        Ok(rows) => rows.filter_map(Result::ok).collect::<Vec<_>>(),
        Err(_) => return,
    };
    drop(stmt);
    for (id, role, content, created_at) in rows {
        record_message_revision_with_conn(&conn, session_id, id, &role, &content, &created_at);
    }
}

pub fn deactivate_message_events_from(db: &Db, session_id: &str, message_id: i64) {
    let _ = db.conn().execute(
        "UPDATE context_events SET active=0
         WHERE session_id=?1 AND source_kind='session_message'
           AND CAST(source_id AS INTEGER) >= ?2",
        params![session_id, message_id],
    );
}

fn section_lines(summary: &str, heading: &str, next_headings: &[&str]) -> Vec<String> {
    let Some(start) = summary.find(heading) else {
        return Vec::new();
    };
    let body = &summary[start + heading.len()..];
    let end = next_headings
        .iter()
        .filter_map(|next| body.find(next))
        .min()
        .unwrap_or(body.len());
    body[..end]
        .lines()
        .map(|line| line.trim().trim_start_matches(['-', '*', ' ']).trim())
        .filter(|line| !line.is_empty() && !line.eq_ignore_ascii_case("unknown"))
        .map(str::to_string)
        .collect()
}

pub fn checkpoint_state_from_summary(
    summary: &str,
    source_event_ids: Vec<String>,
) -> ContextCheckpointState {
    ContextCheckpointState {
        version: 1,
        goals: section_lines(
            summary,
            "Goals:",
            &[
                "Decisions landed:",
                "Completed work:",
                "Remaining work:",
                "Key artifacts:",
            ],
        ),
        decisions: section_lines(
            summary,
            "Decisions landed:",
            &["Completed work:", "Remaining work:", "Key artifacts:"],
        ),
        completed_work: section_lines(
            summary,
            "Completed work:",
            &["Remaining work:", "Key artifacts:"],
        ),
        remaining_work: section_lines(summary, "Remaining work:", &["Key artifacts:"]),
        key_artifacts: section_lines(summary, "Key artifacts:", &[]),
        source_event_ids,
    }
}

pub fn validate_checkpoint_summary(summary: &str) -> Result<(), String> {
    let required = [
        "Current session:",
        "Goals:",
        "Decisions landed:",
        "Completed work:",
        "Remaining work:",
        "Key artifacts:",
    ];
    let missing = required
        .iter()
        .copied()
        .filter(|heading| !summary.contains(heading))
        .collect::<Vec<_>>();
    if summary.trim().is_empty() {
        Err("checkpoint summary is empty".to_string())
    } else if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "checkpoint summary is missing {}",
            missing.join(", ")
        ))
    }
}

pub fn activate_checkpoint(
    db: &Db,
    session_id: &str,
    summary: &str,
    preserved_tail_messages: i64,
    tokens_before: i64,
    estimated_tokens_after: i64,
    model: &str,
    reasoning_effort: &str,
) -> Result<String, String> {
    backfill_session_ledger(db, session_id);
    validate_checkpoint_summary(summary)?;
    let mut conn = db.conn();
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    let bounds: (Option<i64>, Option<i64>) = tx
        .query_row(
            "SELECT MIN(sequence), MAX(sequence) FROM context_events
             WHERE session_id=?1 AND active=1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let mut source_event_ids = Vec::new();
    if let Some(end) = bounds.1 {
        let mut stmt = tx
            .prepare(
                "SELECT event_id FROM context_events
                 WHERE session_id=?1 AND active=1 AND sequence<=?2
                 ORDER BY sequence ASC",
            )
            .map_err(|error| error.to_string())?;
        source_event_ids = stmt
            .query_map(params![session_id, end], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .collect();
    }
    let state = checkpoint_state_from_summary(summary, source_event_ids.clone());
    let state_json = serde_json::to_string(&state).map_err(|error| error.to_string())?;
    let validation = json!({
        "valid": true,
        "sourceEventCount": source_event_ids.len(),
        "summaryHash": content_hash(summary),
    })
    .to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute(
        "UPDATE context_checkpoints SET status='superseded'
         WHERE session_id=?1 AND status='active'",
        [session_id],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO context_checkpoints
         (id, session_id, status, summary, state_json, source_start_sequence,
          source_end_sequence, preserved_tail_messages, tokens_before,
          estimated_tokens_after, model, reasoning_effort, validation_json,
          created_at, activated_at)
         VALUES (?1, ?2, 'active', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
        params![
            id,
            session_id,
            summary,
            state_json,
            bounds.0,
            bounds.1,
            preserved_tail_messages,
            tokens_before,
            estimated_tokens_after,
            model,
            reasoning_effort,
            validation,
            now,
        ],
    )
    .map_err(|error| error.to_string())?;
    // The session counter represents the provider context that will be used by
    // the next turn, not lifetime token usage. Lower it atomically with the
    // checkpoint so a completed compaction cannot immediately retrigger from
    // the superseded provider thread's cumulative usage.
    tx.execute(
        "UPDATE sessions SET input_tokens=?1, last_checkpoint_at=?2, updated_at=?2
         WHERE id=?3",
        params![estimated_tokens_after.max(0), now, session_id],
    )
    .map_err(|error| error.to_string())?;
    for (kind, values) in [
        ("goal", &state.goals),
        ("decision", &state.decisions),
        ("completed_work", &state.completed_work),
        ("remaining_work", &state.remaining_work),
        ("artifact", &state.key_artifacts),
    ] {
        for (index, content) in values.iter().enumerate() {
            let fact_id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO context_facts
                 (id, checkpoint_id, session_id, kind, canonical_key, content,
                  source_event_ids, last_verified_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    fact_id,
                    id,
                    session_id,
                    kind,
                    format!("{kind}:{index}"),
                    content,
                    serde_json::to_string(&source_event_ids).unwrap_or_else(|_| "[]".into()),
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;
        }
    }
    tx.commit().map_err(|error| error.to_string())?;
    Ok(id)
}

pub fn active_checkpoint(db: &Db, session_id: &str) -> Option<(String, ContextCheckpointState)> {
    let conn = db.conn();
    let (summary, state_json): (String, String) = conn
        .query_row(
            "SELECT summary, state_json FROM context_checkpoints
             WHERE session_id=?1 AND status='active' ORDER BY activated_at DESC LIMIT 1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .ok()??;
    let state = serde_json::from_str(&state_json).ok()?;
    Some((summary, state))
}

fn terms(query: &str) -> HashSet<String> {
    query
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .map(str::to_ascii_lowercase)
        .filter(|term| term.len() >= 3)
        .collect()
}

pub fn retrieve_with_archive(
    db: &Db,
    session_id: &str,
    query: &str,
    limit: usize,
    include_archived: bool,
) -> Vec<ContextRetrieval> {
    backfill_session_ledger(db, session_id);
    let query_terms = terms(query);
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT event_id, role, content, sequence, active FROM context_events
         WHERE session_id=?1 AND (?2=1 OR active=1) ORDER BY sequence DESC LIMIT 500",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map(params![session_id, include_archived as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, bool>(4)?,
        ))
    }) {
        Ok(rows) => rows.filter_map(Result::ok).collect::<Vec<_>>(),
        Err(_) => return Vec::new(),
    };
    let newest = rows.first().map(|row| row.3).unwrap_or(0).max(1);
    let mut scored = rows
        .into_iter()
        .filter_map(|(event_id, role, content, sequence, active)| {
            let normalized = content.to_ascii_lowercase();
            let overlap = query_terms
                .iter()
                .filter(|term| normalized.contains(term.as_str()))
                .count();
            if !query_terms.is_empty() && overlap == 0 {
                return None;
            }
            let lexical = if query_terms.is_empty() {
                0.0
            } else {
                overlap as f64 / query_terms.len() as f64
            };
            let recency = sequence as f64 / newest as f64;
            Some(ContextRetrieval {
                event_id,
                role,
                content,
                score: lexical * 0.86 + recency * 0.14,
                active,
            })
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.score.total_cmp(&a.score));
    scored.truncate(limit);
    scored
}

pub fn retrieve(db: &Db, session_id: &str, query: &str, limit: usize) -> Vec<ContextRetrieval> {
    retrieve_with_archive(db, session_id, query, limit, false)
}

pub fn diagnostics(db: &Db, session_id: &str) -> ContextDiagnostics {
    backfill_session_ledger(db, session_id);
    let conn = db.conn();
    let (total_events, active_events) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(active), 0) FROM context_events WHERE session_id=?1",
            [session_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap_or_default();
    let checkpoint_count = conn
        .query_row(
            "SELECT COUNT(*) FROM context_checkpoints WHERE session_id=?1",
            [session_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or_default();
    let active_checkpoint = conn
        .query_row(
            "SELECT id, model, created_at, estimated_tokens_after
             FROM context_checkpoints WHERE session_id=?1 AND status='active'
             ORDER BY activated_at DESC LIMIT 1",
            [session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .ok()
        .flatten();
    ContextDiagnostics {
        total_events,
        active_events,
        archived_revisions: total_events.saturating_sub(active_events),
        checkpoint_count,
        active_checkpoint_id: active_checkpoint.as_ref().map(|value| value.0.clone()),
        checkpoint_model: active_checkpoint.as_ref().map(|value| value.1.clone()),
        checkpoint_created_at: active_checkpoint.as_ref().map(|value| value.2.clone()),
        estimated_tokens_after: active_checkpoint.map(|value| value.3),
    }
}

pub fn prompt_context(db: &Db, session_id: &str, query: &str, max_chars: usize) -> Option<String> {
    let (summary, state) = active_checkpoint(db, session_id)?;
    let mut output = format!(
        "FLAMEL LOSSLESS CONTEXT CHECKPOINT (verified; original events remain retrievable):\n{}\n",
        summary.trim()
    );
    let source_ids = state
        .source_event_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let retrievals = retrieve(db, session_id, query, 8);
    if !retrievals.is_empty() {
        output.push_str("\nRETRIEVED SOURCE EVENTS (prefer these exact records over inference):\n");
        for event in retrievals {
            if !source_ids.contains(&event.event_id) {
                continue;
            }
            let remaining = max_chars.saturating_sub(output.chars().count());
            if remaining < 160 {
                break;
            }
            let content = event
                .content
                .chars()
                .take(remaining.min(1_200))
                .collect::<String>();
            output.push_str(&format!(
                "- [{} {}] {}\n",
                event.event_id, event.role, content
            ));
        }
    }
    Some(output.chars().take(max_chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sessions::{UpsertSession, persist_session_message, upsert_session};

    fn db() -> Db {
        let db = Db::open(":memory:");
        upsert_session(
            &db,
            &UpsertSession {
                id: "session-1".into(),
                claude_session_id: None,
                project_path: "/repo".into(),
                title: Some("Context".into()),
                message_count: Some(0),
                pinned: None,
                model: Some("gpt-5.6-sol".into()),
                input_tokens: Some(0),
                output_tokens: Some(0),
                provider: Some("codex".into()),
                user_id: Some("user-1".into()),
                runner_id: None,
            },
        );
        db
    }

    #[test]
    fn ledger_keeps_distinct_revisions_and_deduplicates_identical_snapshots() {
        let db = db();
        persist_session_message(&db, "session-1", "user", "first exact request");
        persist_session_message(&db, "session-1", "assistant", "first answer");
        backfill_session_ledger(&db, "session-1");
        backfill_session_ledger(&db, "session-1");
        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM context_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn ledger_updates_streaming_snapshots_in_place_with_a_stable_event_id() {
        let db = db();
        let conn = db.conn();
        record_message_revision_with_conn(
            &conn,
            "session-1",
            42,
            "assistant",
            "partial streamed answer",
            "2026-08-06T00:00:00Z",
        );
        let first_event_id: String = conn
            .query_row(
                "SELECT event_id FROM context_events WHERE source_id='42'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        record_message_revision_with_conn(
            &conn,
            "session-1",
            42,
            "assistant",
            "complete streamed answer",
            "2026-08-06T00:00:01Z",
        );
        let final_event_id: String = conn
            .query_row(
                "SELECT event_id FROM context_events WHERE source_id='42'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(conn);

        let (total, active): (i64, i64) = db
            .conn()
            .query_row(
                "SELECT COUNT(*), SUM(active) FROM context_events WHERE source_id='42'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(active, 1);
        assert_eq!(first_event_id, final_event_id);
        let results = retrieve(&db, "session-1", "streamed answer", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "complete streamed answer");
        let audit_results = retrieve_with_archive(&db, "session-1", "streamed answer", 10, true);
        assert_eq!(audit_results.len(), 1);
        assert_eq!(
            audit_results.iter().filter(|result| result.active).count(),
            1
        );
    }

    #[test]
    fn maintenance_removes_legacy_archived_streaming_revisions_in_batches() {
        let db = db();
        let conn = db.conn();
        for index in 0..3 {
            conn.execute(
                "INSERT INTO context_events
                 (event_id, session_id, source_kind, source_id, role, content,
                  content_hash, active, created_at)
                 VALUES (?1, 'session-1', 'session_message', '42', 'assistant',
                         ?2, ?3, 0, '2026-08-06T00:00:00Z')",
                params![
                    format!("legacy-{index}"),
                    format!("partial-{index}"),
                    format!("hash-{index}")
                ],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO context_events
             (event_id, session_id, source_kind, source_id, role, content,
              content_hash, active, created_at)
             VALUES ('current', 'session-1', 'session_message', '42', 'assistant',
                     'complete', 'current-hash', 1, '2026-08-06T00:00:01Z')",
            [],
        )
        .unwrap();
        drop(conn);

        assert_eq!(maintain_context_ledger(&db, 2).archived_revisions, 2);
        assert_eq!(maintain_context_ledger(&db, 2).archived_revisions, 1);
        let remaining: (i64, i64) = db
            .conn()
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(active), 0) FROM context_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(remaining, (1, 1));
    }

    #[test]
    fn activates_only_validated_checkpoint_and_retrieves_sources() {
        let db = db();
        db.conn()
            .execute(
                "UPDATE sessions SET input_tokens=5000 WHERE id='session-1'",
                [],
            )
            .unwrap();
        persist_session_message(
            &db,
            "session-1",
            "user",
            "Keep the billing API backwards compatible",
        );
        let summary = "Current session:\nMigrating billing safely\nGoals:\nKeep billing compatible\nDecisions landed:\nUse v2 route\nCompleted work:\nSchema added\nRemaining work:\nRun migration\nKey artifacts:\nsrc/billing.rs";
        activate_checkpoint(
            &db,
            "session-1",
            summary,
            15,
            1000,
            300,
            "gpt-5.6-luna",
            "medium",
        )
        .unwrap();
        let session_tokens: i64 = db
            .conn()
            .query_row(
                "SELECT input_tokens FROM sessions WHERE id='session-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(session_tokens, 300);
        let context = prompt_context(&db, "session-1", "billing migration", 6000).unwrap();
        assert!(context.contains("Keep billing compatible"));
        assert!(context.contains("first") || context.contains("billing"));
    }

    #[test]
    fn rejects_unstructured_checkpoint_without_replacing_active_state() {
        let db = db();
        assert!(
            activate_checkpoint(
                &db,
                "session-1",
                "short prose",
                15,
                100,
                50,
                "model",
                "medium"
            )
            .is_err()
        );
        assert!(active_checkpoint(&db, "session-1").is_none());
    }
}
