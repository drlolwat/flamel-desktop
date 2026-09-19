#![allow(unused_imports)]
//! memory_mcp — standalone stdio JSON-RPC MCP server for persistent memory.
//! Backed by SQLite + FTS5. Each user+project gets isolated memory scope.

use flamel::mcp_args::normalize_tool_arguments;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

use flamel::db::{
    brain_store::{self, BrainDb, BrainQuery, BrainRecord, BrainScope, BrainWrite},
    memory_store::{self, MemoryDb},
};

fn main() {
    flamel::mcp_parent_guard::install();
    let user_id = std::env::var("FLAMEL_USER_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| flamel::cli_mcp::discover_local_cli_runtime().map(|runtime| runtime.user_id))
        .unwrap_or_else(|| flamel::FLAMEL_LOCAL_USER_ID.to_string());
    let project_path = flamel::cli_mcp::current_project_path();

    // Open memories.db next to the main flamel.db
    let db_path = std::env::var("FLAMEL_MEMORIES_DB").unwrap_or_else(|_| {
        let mut p = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
        p.push("@flamel");
        p.push("memories.db");
        p.to_string_lossy().to_string()
    });
    let db = MemoryDb::open(&db_path);
    let brain = BrainDb::open(&db_path);

    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let response = handle_request(&req, &db, &brain, &user_id, &project_path);
        if let Some(resp) = response {
            let mut out = stdout.lock();
            let _ = serde_json::to_writer(&mut out, &resp);
            let _ = out.write_all(b"\n");
            let _ = out.flush();
        }
    }
}

fn handle_request(
    req: &Value,
    db: &MemoryDb,
    brain: &BrainDb,
    user_id: &str,
    project_path: &str,
) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method")?.as_str()?;

    if method == "initialize" {
        let protocol_version = flamel::mcp_gateway::negotiated_protocol(
            req.get("params")
                .and_then(|params| params.get("protocolVersion"))
                .and_then(Value::as_str),
        );
        return Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": protocol_version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "flamel-memory", "version": "1.0.0"},
            }
        }));
    }

    if method.starts_with("notifications/") {
        return None;
    }

    if method == "tools/list" {
        return Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"tools": tools_list()},
        }));
    }

    if method == "tools/call" {
        let params = req.get("params")?;
        let name = params.get("name")?.as_str()?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let args = match normalize_tool_arguments(name, args, &tools_list()) {
            Ok(normalized) => normalized.arguments,
            Err(e) => {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"content": [{"type": "text", "text": format!("Error: {e}")}], "isError": true},
                }));
            }
        };

        match handle_tool_call(name, &args, db, brain, user_id, project_path) {
            Ok(text) => {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"content": [{"type": "text", "text": text}]},
                }));
            }
            Err(e) => {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"content": [{"type": "text", "text": format!("Error: {e}")}], "isError": true},
                }));
            }
        }
    }

    id.map(|id| {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": "Method not found"},
        })
    })
}

fn string_array(value: Option<&Value>, field: &str) -> Result<Option<Vec<String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("{field} must be an array"))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{field} entries must be strings"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn brain_scope_from_value(value: &Value) -> Result<BrainScope, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "scope must be an object".to_owned())?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "scope.kind required".to_owned())?;
    let id = object.get("id").and_then(Value::as_str).unwrap_or_default();
    Ok(BrainScope {
        kind: kind.to_owned(),
        id: id.to_owned(),
    })
}

fn brain_scopes(args: &Value) -> Result<Vec<BrainScope>, String> {
    if let Some(values) = args.get("scopes") {
        return values
            .as_array()
            .ok_or_else(|| "scopes must be an array".to_owned())?
            .iter()
            .map(brain_scope_from_value)
            .collect();
    }
    args.get("scope")
        .map(brain_scope_from_value)
        .transpose()
        .map(|scope| scope.into_iter().collect())
}

fn brain_write_from_args(
    args: &Value,
    project_path: &str,
    existing: Option<&BrainRecord>,
) -> Result<BrainWrite, String> {
    let scope = if let Some(scope) = args.get("scope") {
        brain_scope_from_value(scope)?
    } else if args.get("scope_kind").is_some() || args.get("scope_id").is_some() {
        BrainScope {
            kind: args
                .get("scope_kind")
                .and_then(Value::as_str)
                .unwrap_or("personal")
                .to_owned(),
            id: args
                .get("scope_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }
    } else if let Some(existing) = existing {
        existing.scope.clone()
    } else if args
        .get("project_scope")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        BrainScope::project(project_path)
    } else {
        BrainScope::personal()
    };
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| existing.map(|record| record.content.clone()))
        .ok_or_else(|| "content required".to_owned())?;
    let tags = string_array(args.get("tags"), "tags")?
        .or_else(|| existing.map(|record| record.tags.clone()))
        .unwrap_or_default();
    let provenance = args
        .get("provenance")
        .cloned()
        .or_else(|| existing.map(|record| record.provenance.clone()))
        .unwrap_or_else(|| json!({"source":"explicit memory tool"}));
    let artifact_refs = match args.get("artifact_refs") {
        Some(value) => value
            .as_array()
            .cloned()
            .ok_or_else(|| "artifact_refs must be an array".to_owned())?,
        None => existing
            .map(|record| record.artifact_refs.clone())
            .unwrap_or_default(),
    };
    let expires_at = if args.get("expires_at").is_some() {
        args.get("expires_at").and_then(Value::as_i64)
    } else {
        existing.and_then(|record| record.expires_at)
    };
    Ok(BrainWrite {
        assistant_id: existing
            .map(|record| record.assistant_id.clone())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "memory-mcp".into()),
        surface: existing
            .map(|record| record.surface.clone())
            .unwrap_or_else(BrainScope::private_surface),
        revision: existing.map(|record| record.revision).unwrap_or(1),
        kind: args
            .get("record_kind")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| existing.map(|record| record.kind.clone()))
            .unwrap_or_else(|| "fact".into()),
        project_path: if scope.kind == "project" {
            scope.id.clone()
        } else {
            existing
                .map(|record| record.project_path.clone())
                .unwrap_or_else(|| project_path.to_owned())
        },
        scope,
        content,
        tags,
        provenance,
        artifact_refs,
        confidence: args
            .get("confidence")
            .and_then(Value::as_f64)
            .or_else(|| existing.map(|record| record.confidence))
            .unwrap_or(1.0),
        sensitivity: args
            .get("sensitivity")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| existing.map(|record| record.sensitivity.clone()))
            .unwrap_or_else(|| "normal".into()),
        retention: args
            .get("retention")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| existing.map(|record| record.retention.clone()))
            .unwrap_or_else(|| "durable".into()),
        pinned: args
            .get("pinned")
            .and_then(Value::as_bool)
            .or_else(|| existing.map(|record| record.pinned))
            .unwrap_or(false),
        expires_at,
    })
}

fn handle_tool_call(
    name: &str,
    args: &Value,
    db: &MemoryDb,
    brain: &BrainDb,
    user_id: &str,
    project_path: &str,
) -> Result<String, String> {
    let resolved: String;
    let name = if name == "memory" {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");
        resolved = if action.starts_with("memory_") {
            action.to_string()
        } else {
            format!("memory_{action}")
        };
        resolved.as_str()
    } else {
        name
    };

    match name {
        "memory_remember" => {
            let input = brain_write_from_args(args, project_path, None)?;
            let outcome = brain_store::add_brain_record(brain, user_id, &input)?;
            serde_json::to_string(&outcome).map_err(|error| error.to_string())
        }
        "memory_recall" => {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .ok_or("query required")?;
            let kinds = string_array(args.get("record_kinds"), "record_kinds")?.unwrap_or_default();
            let request = BrainQuery {
                query: query.to_owned(),
                assistant_id: Some("memory-mcp".into()),
                surface: Some(BrainScope::private_surface()),
                scopes: brain_scopes(args)?,
                kinds,
                limit: args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize,
                max_bytes: args
                    .get("max_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(64 * 1024) as usize,
                include_superseded: args
                    .get("include_superseded")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            };
            let result = brain_store::search_brain_records(brain, user_id, &request, None)?;
            serde_json::to_string(&result).map_err(|error| error.to_string())
        }
        "memory_inspect" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or("id required")?;
            let record = brain_store::get_brain_record(brain, id, user_id)?;
            serde_json::to_string(&record).map_err(|error| error.to_string())
        }
        "memory_edit" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or("id required")?;
            let current = brain_store::get_brain_record(brain, id, user_id)?;
            let input = brain_write_from_args(args, project_path, Some(&current))?;
            let record = brain_store::update_brain_record(brain, id, user_id, &input)?;
            serde_json::to_string(&record).map_err(|error| error.to_string())
        }
        "memory_forget" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or("id required")?;
            brain_store::forget_brain_record(brain, id, user_id)?;
            Ok(json!({"ok":true,"id":id,"forgotten":true}).to_string())
        }
        "memory_export" => {
            let (records, truncated, bytes) = brain_store::export_brain_records(
                brain,
                user_id,
                Some("memory-mcp"),
                Some(&BrainScope::private_surface()),
                &brain_scopes(args)?,
                args.get("include_superseded")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                args.get("max_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(256 * 1024) as usize,
            )?;
            Ok(json!({
                "schemaVersion":brain_store::BRAIN_SCHEMA_VERSION,
                "records":records,
                "truncated":truncated,
                "bytes":bytes
            })
            .to_string())
        }
        "memory_cleanup" => {
            let deleted = brain_store::cleanup_expired_brain_records(brain, user_id)?;
            Ok(json!({"ok":true,"deleted":deleted}).to_string())
        }
        "memory_brain_pin" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or("id required")?;
            let pinned = args.get("pinned").and_then(Value::as_bool).unwrap_or(true);
            brain_store::pin_brain_record(brain, id, user_id, pinned)?;
            Ok(json!({"ok":true,"id":id,"pinned":pinned}).to_string())
        }
        "memory_brain_supersede" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or("id required")?;
            let replacement = args
                .get("replacement_id")
                .and_then(Value::as_str)
                .ok_or("replacement_id required")?;
            brain_store::supersede_brain_record(brain, id, replacement, user_id)?;
            Ok(json!({"ok":true,"id":id,"supersededBy":replacement}).to_string())
        }
        "memory_add" => {
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or("content required")?;
            let tags: Vec<String> = args
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let scope = args
                .get("project_scope")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let pp = if scope { project_path } else { "" };
            let mem = memory_store::add_memory(db, user_id, pp, content, &tags)?;
            let provenance = args
                .get("provenance")
                .cloned()
                .unwrap_or_else(|| json!({"source":"explicit memory tool"}));
            memory_store::set_evidence(
                db,
                &mem.id,
                user_id,
                &provenance,
                args.get("confidence")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0),
                args.get("source_kind")
                    .and_then(Value::as_str)
                    .unwrap_or("user_statement"),
                args.get("source_id").and_then(Value::as_str),
                args.get("pinned").and_then(Value::as_bool).unwrap_or(false),
                args.get("retention")
                    .and_then(Value::as_str)
                    .unwrap_or("durable"),
                args.get("expires_at").and_then(Value::as_i64),
            )?;
            Ok(json!({"id": mem.id, "content": mem.content, "tags": mem.tags, "project_path": mem.project_path,"provenance":provenance}).to_string())
        }
        "memory_search" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or("query required")?;
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
            let scope = args
                .get("project_scope")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let pp = if scope { Some(project_path) } else { None };
            let results = memory_store::search_memories(db, query, user_id, pp, limit);
            Ok(json!({"count": results.len(), "memories": results}).to_string())
        }
        "memory_list" => {
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
            let scope = args
                .get("project_scope")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let pp = if scope { Some(project_path) } else { None };
            let results = memory_store::list_memories(db, user_id, pp, limit);
            Ok(json!({"count": results.len(), "memories": results}).to_string())
        }
        "memory_update" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("id required")?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or("content required")?;
            let tags: Vec<String> = args
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            memory_store::update_memory(db, id, user_id, content, &tags)?;
            Ok(json!({"ok": true, "id": id}).to_string())
        }
        "memory_delete" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("id required")?;
            memory_store::delete_memory(db, id, user_id)?;
            Ok(json!({"ok": true, "id": id}).to_string())
        }
        "memory_pin" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or("id required")?;
            let pinned = args.get("pinned").and_then(Value::as_bool).unwrap_or(true);
            memory_store::pin_memory(db, id, user_id, pinned)?;
            Ok(json!({"ok":true,"id":id,"pinned":pinned}).to_string())
        }
        "memory_supersede" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or("id required")?;
            let replacement = args
                .get("replacement_id")
                .and_then(Value::as_str)
                .ok_or("replacement_id required")?;
            memory_store::supersede_memory(db, id, user_id, replacement)?;
            Ok(json!({"ok":true,"id":id,"supersededBy":replacement}).to_string())
        }
        "memory_clear" => {
            let scope = args
                .get("project_scope")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let pp = if scope { Some(project_path) } else { None };
            let count = memory_store::clear_memories(db, user_id, pp)?;
            Ok(json!({"ok": true, "deleted": count}).to_string())
        }
        _ => Err(format!("Unknown tool: {name}")),
    }
}

fn tools_list() -> Value {
    json!([
        {
            "name": "memory",
            "description": "Evidence-based persistent memory and the versioned Flamel Assistant brain. Legacy manual actions remain available: add, search, list, update, pin, supersede, delete, clear. Brain actions: remember, recall, inspect, edit, forget, export, cleanup, brain_pin, brain_supersede. Brain records support personal, project, Chattle space/channel, conversation, and Job scopes with provenance, artifact references, confidence, sensitivity, retention, expiry, deduplication, and supersession. Automatic capture and recall are controlled separately.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["add", "search", "list", "update", "pin", "supersede", "delete", "clear", "remember", "recall", "inspect", "edit", "forget", "export", "cleanup", "brain_pin", "brain_supersede"], "description": "Memory action (default: list). Use brain actions for Flamel Assistant records."},
                    "content": {"type": "string", "description": "Memory content (for add/update)"},
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "Tags (for add/update)"},
                    "project_scope": {"type": "boolean", "description": "Scope to current project (default: true for add/clear, false for search/list)"},
                    "query": {"type": "string", "description": "Search query (for search)"},
                    "limit": {"type": "integer", "description": "Max results (for search/list, default: 20)"},
                    "id": {"type": "string", "description": "Memory ID"},
                    "replacement_id": {"type":"string","description":"Replacement memory ID (supersede)"},
                    "provenance": {"type":"object","description":"Evidence source with path, plan, session, or result references"},
                    "confidence": {"type":"number","minimum":0,"maximum":1},
                    "source_kind": {"type":"string","enum":["user_statement","accepted_decision","plan","verified_result","file_history"]},
                    "source_id": {"type":"string"},
                    "record_kind": {"type":"string","enum":["preference","fact","decision","entity","goal","episode","procedure"],"description":"Brain record kind"},
                    "scope": {"type":"object","properties":{"kind":{"type":"string","enum":["personal","project","chattle_space","chattle_channel","conversation","job"]},"id":{"type":"string"}},"required":["kind"],"description":"Exact brain scope. Personal uses an empty id."},
                    "scopes": {"type":"array","items":{"type":"object","properties":{"kind":{"type":"string"},"id":{"type":"string"}},"required":["kind"]},"description":"Exact scopes to recall or export"},
                    "record_kinds": {"type":"array","items":{"type":"string"},"description":"Brain kinds to recall"},
                    "artifact_refs": {"type":"array","items":{"type":"object"},"description":"Credential-free source or artifact references"},
                    "sensitivity": {"type":"string","enum":["normal","private","restricted"]},
                    "include_superseded": {"type":"boolean"},
                    "max_bytes": {"type":"integer","minimum":1024,"maximum":262144,"description":"Bounded result/export budget"},
                    "pinned": {"type":"boolean"},
                    "retention": {"type":"string","enum":["session","project","durable","ttl"]},
                    "expires_at": {"type":"integer","description":"Optional Unix expiry; required for ttl retention"}
                }
            },
            "outputSchema":{"type":"object"}
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> (MemoryDb, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "flamel-memory-scope-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        (MemoryDb::open(path.to_string_lossy().as_ref()), path)
    }

    #[test]
    fn memory_records_are_isolated_by_user() {
        let (db, path) = test_db();
        let owner = memory_store::add_memory(&db, "owner", "/project", "private", &[])
            .expect("owner memory");

        assert!(memory_store::list_memories(&db, "other", None, 20).is_empty());
        assert!(memory_store::update_memory(&db, &owner.id, "other", "changed", &[]).is_err());
        assert!(memory_store::pin_memory(&db, &owner.id, "other", true).is_err());
        assert!(memory_store::delete_memory(&db, &owner.id, "other").is_err());
        assert_eq!(
            memory_store::clear_memories(&db, "other", None).expect("clear other"),
            0
        );

        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn supersession_requires_both_memories_to_have_the_same_owner() {
        let (db, path) = test_db();
        let source = memory_store::add_memory(&db, "owner", "/project", "source", &[])
            .expect("source memory");
        let foreign = memory_store::add_memory(&db, "other", "/project", "foreign", &[])
            .expect("foreign memory");
        assert!(memory_store::supersede_memory(&db, &source.id, "owner", &foreign.id).is_err());

        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn memory_mcp_brain_actions_round_trip_with_scope_and_owner_fencing() {
        let path = std::env::temp_dir().join(format!(
            "flamel-memory-brain-mcp-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let db = MemoryDb::open(path.to_string_lossy().as_ref());
        let brain = BrainDb::open(path.to_string_lossy().as_ref());
        let remembered = handle_tool_call(
            "memory",
            &json!({
                "action":"remember",
                "record_kind":"decision",
                "content":"Use the native assistant runtime",
                "scope":{"kind":"chattle_channel","id":"channel-1"},
                "provenance":{"source":"accepted decision","session_id":"session-1"}
            }),
            &db,
            &brain,
            "owner",
            "/project",
        )
        .expect("remember");
        let remembered: Value = serde_json::from_str(&remembered).unwrap();
        let id = remembered["record"]["id"].as_str().unwrap();
        assert_eq!(remembered["record"]["scope"]["kind"], "chattle_channel");
        assert!(
            handle_tool_call(
                "memory",
                &json!({"action":"inspect","id":id}),
                &db,
                &brain,
                "other",
                "/project",
            )
            .is_err()
        );

        let recalled = handle_tool_call(
            "memory",
            &json!({
                "action":"recall",
                "query":"native runtime",
                "scopes":[{"kind":"chattle_channel","id":"channel-1"}],
                "max_bytes":4096
            }),
            &db,
            &brain,
            "owner",
            "/project",
        )
        .expect("recall");
        let recalled: Value = serde_json::from_str(&recalled).unwrap();
        assert_eq!(recalled["hits"].as_array().unwrap().len(), 1);
        assert_eq!(recalled["retrievalMode"], "lexical");

        handle_tool_call(
            "memory",
            &json!({"action":"forget","id":id}),
            &db,
            &brain,
            "owner",
            "/project",
        )
        .expect("forget");
        assert!(brain_store::get_brain_record(&brain, id, "owner").is_err());
        drop(brain);
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn memory_tool_schema_exposes_brain_administration_without_enabling_automation() {
        let catalog = tools_list();
        let actions = catalog[0]["inputSchema"]["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        for action in [
            "remember",
            "recall",
            "inspect",
            "edit",
            "forget",
            "export",
            "cleanup",
            "brain_pin",
            "brain_supersede",
        ] {
            assert!(actions.contains(&action), "missing brain action {action}");
        }
        assert!(
            !tools_list()[0]["description"]
                .as_str()
                .unwrap()
                .contains("automatically captures")
        );
    }
}
