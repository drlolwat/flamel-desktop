use super::*;

pub(super) fn handle(
    name: &str,
    args: &Value,
    base_url: &str,
    token: &str,
    session_id: &str,
    project_path: &str,
) -> Result<String, String> {
    match name {
        "decision_checkpoint" => Ok(json!({
            "status": "recorded",
            "question": args.get("question"),
            "constraints": args.get("constraints"),
            "evidence": args.get("evidence"),
            "decision": args.get("decision"),
            "risks": args.get("risks"),
            "next_action": args.get("next_action"),
        })
        .to_string()),

        "assistant" => Ok(call_api(
            "POST",
            "/api/internal/flamel-assistant/operation",
            Some(args),
            base_url,
            token,
        )?
        .to_string()),

        "job" => Ok(call_api(
            "POST",
            "/api/internal/flamel-jobs/operation",
            Some(args),
            base_url,
            token,
        )?
        .to_string()),

        "AskUserQuestion" => {
            if session_id.trim().is_empty() {
                return Err(
                    "FLAMEL_SESSION_ID not set — cannot surface AskUserQuestion in the chat UI"
                        .to_string(),
                );
            }
            let question = args
                .get("question")
                .and_then(|v| v.as_str())
                .ok_or("question required")?;
            let request_id = format!("ask-{}", uuid::Uuid::new_v4());

            let mut body = json!({
                "requestId": request_id,
                "sessionId": session_id,
                "question": question,
                "allowFreeText": args.get("allowFreeText").and_then(|v| v.as_bool()).unwrap_or(true),
            });
            if let Some(choices) = args.get("choices") {
                body["choices"] = choices.clone();
            }
            if let Ok(run_id) = std::env::var("FLAMEL_RUN_ID") {
                let run_id = run_id.trim();
                if !run_id.is_empty() {
                    body["runId"] = Value::String(run_id.to_string());
                }
            }

            let resp = call_api("POST", "/api/ask", Some(&body), base_url, token)?;
            let answer = resp
                .get("answer")
                .and_then(|v| v.as_str())
                .filter(|answer| !answer.trim().is_empty())
                .ok_or("AskUserQuestion returned no answer")?
                .to_string();
            if let Some(owner) = resp.get("deliveryOwner").and_then(Value::as_str) {
                call_api(
                    "POST",
                    &format!("/api/ask/{request_id}/consume"),
                    Some(&json!({"deliveryOwner": owner})),
                    base_url,
                    token,
                )?;
            }
            Ok(answer)
        }

        "plan"
        | "flamel_plan_create"
        | "flamel_plan_get"
        | "flamel_plan_list"
        | "flamel_plan_update_item"
        | "flamel_plan_add_items"
        | "flamel_plan_delete_item"
        | "flamel_plan_replace_items"
        | "flamel_plan_rename"
        | "flamel_plan_mark_finished"
        | "flamel_plan_start"
        | "flamel_plan_resume"
        | "flamel_plan_pause"
        | "flamel_plan_block" => {
            // Resolve action: either from the "action" param or inferred from legacy tool name
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    // Legacy compat: strip "flamel_plan_" prefix to get action
                    name.strip_prefix("flamel_plan_").or(Some(name))
                })
                .ok_or("action required")?;
            match action {
                "create" => {
                    let plan = call_api(
                        "POST",
                        "/api/plans",
                        Some(&json!({
                            "title": args.get("title"),
                            "description": args.get("description"),
                            "items": coerce_array(args.get("items")),
                            "sessionId": session_id,
                            "projectPath": project_path,
                        })),
                        base_url,
                        token,
                    )?;
                    let payload = plan_marker_payload(&plan, true);
                    Ok(format!(
                        "PLAN_CREATED:{}\n\n{}",
                        payload,
                        format_plan(&plan)
                    ))
                }
                "require_browser_review" | "browser_review" => {
                    let plan_id = args["plan_id"].as_str().ok_or("plan_id required")?;
                    let body = if action == "require_browser_review" {
                        json!({"browserReviewRequired":true})
                    } else {
                        let mut review = args["browser_review"].clone();
                        if let Some(obj) = review.as_object_mut() {
                            obj.insert("sessionId".into(), json!(session_id));
                        }
                        json!({"browserReview":review})
                    };
                    call_api(
                        "PUT",
                        &format!("/api/plans/{plan_id}"),
                        Some(&body),
                        base_url,
                        token,
                    )?;
                    let plan = call_api(
                        "GET",
                        &format!("/api/plans/{plan_id}"),
                        None,
                        base_url,
                        token,
                    )?;
                    Ok(format!(
                        "PLAN_UPDATED:{}\n\n{}\nBrowser review: {}",
                        plan_marker_payload(&plan, true),
                        format_plan(&plan),
                        plan["browserReview"]
                    ))
                }
                "replace_items" => {
                    let plan_id = args
                        .get("plan_id")
                        .and_then(|v| v.as_str())
                        .ok_or("plan_id required")?;
                    let plan = call_api(
                        "PUT",
                        &format!("/api/plans/{plan_id}/items"),
                        Some(&json!({
                            "items": coerce_array(args.get("items")),
                        })),
                        base_url,
                        token,
                    )?;
                    let payload = plan_marker_payload(&plan, true);
                    Ok(format!(
                        "PLAN_UPDATED:{}\n\n{}",
                        payload,
                        format_plan(&plan)
                    ))
                }
                "rename" => {
                    let plan_id = args
                        .get("plan_id")
                        .and_then(|v| v.as_str())
                        .ok_or("plan_id required")?;
                    let _plan = call_api(
                        "PUT",
                        &format!("/api/plans/{plan_id}"),
                        Some(&json!({
                            "title": args.get("title"),
                            "description": args.get("description"),
                        })),
                        base_url,
                        token,
                    )?;
                    let payload = json!({"planId": plan_id, "title": args.get("title")});
                    Ok(format!("PLAN_RENAMED:{payload}"))
                }
                "update_item" => {
                    let plan_id = args
                        .get("plan_id")
                        .and_then(|v| v.as_str())
                        .ok_or("plan_id required")?;
                    let item_id = args
                        .get("item_id")
                        .and_then(|v| v.as_str())
                        .ok_or("item_id required")?;
                    let mut body = json!({});
                    if let Some(s) = args.get("status") {
                        body["status"] = s.clone();
                    }
                    if let Some(t) = args.get("text") {
                        body["text"] = t.clone();
                    }
                    if let Some(n) = args.get("notes") {
                        body["notes"] = n.clone();
                    }
                    if let Some(s) = args.get("section") {
                        body["section"] = s.clone();
                    }
                    if let Some(floor) = args.get("reasoning_floor") {
                        body["reasoningFloor"] = floor.clone();
                    }
                    let item = call_api(
                        "PUT",
                        &format!("/api/plans/{plan_id}/items/{item_id}"),
                        Some(&body),
                        base_url,
                        token,
                    )
                    .map_err(|e| {
                        if e.contains("404") {
                            format!("item_id \"{item_id}\" not found in plan \"{plan_id}\". Call plan(action=\"get\", plan_id=\"{plan_id}\") to get current item IDs.")
                        } else if e.contains("409") || e.contains("approval_required") {
                            format!("Plan \"{plan_id}\" is awaiting approval. After explicit user approval, call plan(action=\"start\", plan_id=\"{plan_id}\") before marking items in_progress or done.")
                        } else if e.contains("400") || e.contains("422") {
                            format!("Bad request updating item - check plan_id and item_id are valid UUIDs. Error: {e}")
                        } else {
                            e
                        }
                    })?;
                    let payload = plan_item_marker_payload(plan_id, &item);
                    let progress_instruction = match args.get("status").and_then(|v| v.as_str()) {
                        Some("done") => {
                            "Item completion is recorded. Before any work or tool call for another item, mark that next item in_progress. Do not defer completed item updates until the end of the plan."
                        }
                        Some("in_progress") => {
                            "Active item is recorded. Mark it done immediately after its own acceptance checks pass, before starting another item."
                        }
                        Some("blocked") => {
                            "Blocked item is recorded. Mark the next actionable item in_progress before starting its work."
                        }
                        _ => "Item update is recorded.",
                    };
                    Ok(format!("ITEM_UPDATED:{payload}\n\n{progress_instruction}"))
                }
                "add_items" => {
                    let plan_id = args
                        .get("plan_id")
                        .and_then(|v| v.as_str())
                        .ok_or("plan_id required")?;
                    let result = call_api(
                        "POST",
                        &format!("/api/plans/{plan_id}/items"),
                        Some(&json!({
                            "items": coerce_array(args.get("items")),
                        })),
                        base_url,
                        token,
                    )?;
                    let new_items = result["items"].as_array().cloned().unwrap_or_default();
                    let added_count = new_items.len();
                    let payload = json!({
                        "planId": plan_id,
                        "title": result["title"],
                        "description": result["description"],
                        "items": new_items,
                        "versionCount": result["versionCount"],
                        "finished": result["finished"],
                        "executionStatus": result["executionStatus"],
                        "approvalStatus": result["approvalStatus"],
                        "activeItemId": result["activeItemId"],
                        "approvalRequestedAt": result["approvalRequestedAt"],
                        "approvedAt": result["approvedAt"],
                        "executionStartedAt": result["executionStartedAt"],
                        "executionUpdatedAt": result["executionUpdatedAt"],
                    });
                    Ok(format!(
                        "ITEMS_ADDED:{payload}\n\nAdded {added_count} item(s) to plan {plan_id}. Full details are in the Plan panel."
                    ))
                }
                "get" => {
                    let plan_id = args
                        .get("plan_id")
                        .and_then(|v| v.as_str())
                        .ok_or("plan_id required")?;
                    let plan = call_api(
                        "GET",
                        &format!("/api/plans/{plan_id}"),
                        None,
                        base_url,
                        token,
                    )?;
                    let payload = plan_marker_payload(&plan, true);
                    Ok(format!("PLAN_GET:{}\n\n{}", payload, format_plan(&plan)))
                }
                "delete_item" => {
                    let plan_id = args
                        .get("plan_id")
                        .and_then(|v| v.as_str())
                        .ok_or("plan_id required")?;
                    let item_id = args
                        .get("item_id")
                        .and_then(|v| v.as_str())
                        .ok_or("item_id required")?;
                    call_api(
                        "DELETE",
                        &format!("/api/plans/{plan_id}/items/{item_id}"),
                        None,
                        base_url,
                        token,
                    )?;
                    Ok(format!(
                        "ITEM_DELETED:{}",
                        json!({"planId": plan_id, "itemId": item_id})
                    ))
                }
                "list" => {
                    let plans = call_api(
                        "GET",
                        &build_plan_list_path(args, session_id),
                        None,
                        base_url,
                        token,
                    )?;
                    let plans_arr = plans.as_array().unwrap_or(&Vec::new()).clone();
                    if plans_arr.is_empty() {
                        return Ok("No plans found.".to_string());
                    }
                    let mut lines = vec![
                        "Found plans (use plan(action=\"get\", plan_id=...) to see items):"
                            .to_string(),
                    ];
                    for p in &plans_arr {
                        let item_count = p["itemCount"].as_i64().unwrap_or(0);
                        let done_count = p["doneCount"].as_i64().unwrap_or(0);
                        let pct = if item_count > 0 {
                            (done_count * 100) / item_count
                        } else {
                            0
                        };
                        let updated = p["updatedAt"]
                            .as_str()
                            .unwrap_or("")
                            .split('T')
                            .next()
                            .unwrap_or("");
                        lines.push(format!(
                            "- \"{}\" (plan_id: {}) — {}/{} done ({}%), updated {}",
                            p["title"].as_str().unwrap_or(""),
                            p["id"].as_str().unwrap_or(""),
                            done_count,
                            item_count,
                            pct,
                            updated
                        ));
                    }
                    Ok(lines.join("\n"))
                }
                "mark_finished" => {
                    let plan_id = args
                        .get("plan_id")
                        .and_then(|v| v.as_str())
                        .ok_or("plan_id required")?;
                    let plan = call_api(
                        "PUT",
                        &format!("/api/plans/{plan_id}"),
                        Some(&json!({"finished": true})),
                        base_url,
                        token,
                    )?;
                    let payload = plan_marker_payload(&plan, false);
                    Ok(format!("PLAN_FINISHED:{payload}"))
                }
                "start" | "resume" | "pause" | "block" => {
                    let plan_id = args
                        .get("plan_id")
                        .and_then(|v| v.as_str())
                        .ok_or("plan_id required")?;
                    let plan = call_api(
                        "PUT",
                        &format!("/api/plans/{plan_id}/execution"),
                        Some(&json!({
                            "action": action,
                            "approvalSessionId": session_id,
                        })),
                        base_url,
                        token,
                    )?;
                    let payload = plan_marker_payload(&plan, true);
                    Ok(format!(
                        "PLAN_EXECUTION_UPDATED:{payload}\n\n{}",
                        format_plan(&plan)
                    ))
                }
                _ => Err(format!(
                    "unknown plan action: {action}. Valid: create, get, list, start, resume, pause, block, update_item, add_items, delete_item, replace_items, rename, mark_finished"
                )),
            }
        }

        // ── Consolidated file_history tool ───────────────────────────────
        "file_history" | "revert_file" | "list_file_edits" | "checkpoint" | "rollback" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or(name);
            match action {
                "revert" | "revert_file" => {
                    let file = args
                        .get("file")
                        .and_then(|v| v.as_str())
                        .ok_or("file required")?;
                    let steps = args.get("steps").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                    if session_id.is_empty() {
                        return Err("FLAMEL_SESSION_ID not set — cannot revert".to_string());
                    }
                    let full_path = if std::path::Path::new(file).is_absolute() {
                        file.to_string()
                    } else {
                        format!("{project_path}/{file}")
                    };
                    let sanitized = full_path.replace(['/', '\\', ':'], "_");
                    let dir = format!("/tmp/flamel-history/{session_id}/{sanitized}");
                    let mut backups: Vec<_> = std::fs::read_dir(&dir)
                        .map_err(|_| format!("No edit history for {file}"))?
                        .flatten()
                        .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
                        .collect();
                    backups.sort_by_key(|e| e.file_name());
                    backups.reverse(); // newest first
                    let target = backups.get(steps - 1).ok_or(format!(
                        "Only {} backup(s) available for {file}",
                        backups.len()
                    ))?;
                    let content = std::fs::read(target.path())
                        .map_err(|e| format!("Cannot read backup: {e}"))?;
                    backup_file(&full_path, session_id); // backup current state before reverting
                    std::fs::write(&full_path, &content)
                        .map_err(|e| format!("Cannot write {file}: {e}"))?;
                    let ts_ms: u64 = target
                        .file_name()
                        .to_string_lossy()
                        .trim_end_matches(".bak")
                        .parse()
                        .unwrap_or(0);
                    let age_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0)
                        .saturating_sub(ts_ms)
                        / 1000;
                    let age = if age_secs < 60 {
                        format!("{age_secs}s ago")
                    } else if age_secs < 3600 {
                        format!("{}m ago", age_secs / 60)
                    } else {
                        format!("{}h ago", age_secs / 3600)
                    };
                    Ok(format!(
                        "Reverted {file} to backup from {age} ({steps} edit(s) back)"
                    ))
                }
                "list" | "list_file_edits" => {
                    if session_id.is_empty() {
                        return Err("FLAMEL_SESSION_ID not set".to_string());
                    }
                    let file = args.get("file").and_then(|v| v.as_str());
                    let history_root = format!("/tmp/flamel-history/{session_id}");

                    let mut results: Vec<String> = Vec::new();
                    let scan_dirs: Vec<String> = if let Some(f) = file {
                        let full_path = if std::path::Path::new(f).is_absolute() {
                            f.to_string()
                        } else {
                            format!("{project_path}/{f}")
                        };
                        let sanitized = full_path.replace(['/', '\\', ':'], "_");
                        vec![format!("{history_root}/{sanitized}")]
                    } else {
                        std::fs::read_dir(&history_root)
                            .ok()
                            .map(|rd| {
                                rd.flatten()
                                    .filter(|e| e.path().is_dir())
                                    .map(|e| e.path().display().to_string())
                                    .collect()
                            })
                            .unwrap_or_default()
                    };

                    for dir in scan_dirs {
                        let dir_name = dir.split('/').last().unwrap_or("").to_string();
                        // Reverse the sanitization to show a readable path
                        let display_path = dir_name.trim_start_matches('_').replacen('_', "/", 5);
                        let mut backups: Vec<_> = std::fs::read_dir(&dir)
                            .ok()
                            .map(|rd| {
                                rd.flatten()
                                    .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
                                    .collect()
                            })
                            .unwrap_or_default();
                        backups.sort_by_key(|e| e.file_name());
                        backups.reverse();
                        for (i, b) in backups.iter().enumerate() {
                            let ts_ms: u64 = b
                                .file_name()
                                .to_string_lossy()
                                .trim_end_matches(".bak")
                                .parse()
                                .unwrap_or(0);
                            let age_secs = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0)
                                .saturating_sub(ts_ms)
                                / 1000;
                            let age = if age_secs < 60 {
                                format!("{age_secs}s ago")
                            } else if age_secs < 3600 {
                                format!("{}m ago", age_secs / 60)
                            } else {
                                format!("{}h ago", age_secs / 3600)
                            };
                            let size = b.metadata().map(|m| m.len()).unwrap_or(0);
                            results.push(format!(
                                "[{}] {} — backup #{} ({}, {}B)",
                                display_path,
                                age,
                                i + 1,
                                b.file_name().to_string_lossy(),
                                size
                            ));
                        }
                    }

                    if results.is_empty() {
                        Ok("No file edit history for this session.".to_string())
                    } else {
                        Ok(results.join("\n"))
                    }
                }
                "checkpoint" => {
                    let name = args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unnamed");
                    let status = flamel::mcp_parent_guard::command("git")
                        .args(["status", "--porcelain"])
                        .current_dir(project_path)
                        .output()
                        .map_err(|e| format!("git not found: {e}"))?;
                    if String::from_utf8_lossy(&status.stdout).trim().is_empty() {
                        return Ok("Nothing to checkpoint — working tree is clean.".to_string());
                    }
                    let stash_msg = format!("flamel-checkpoint-{name}");
                    let out = flamel::mcp_parent_guard::command("git")
                        .args(["stash", "push", "--include-untracked", "-m", &stash_msg])
                        .current_dir(project_path)
                        .output()
                        .map_err(|e| format!("git stash failed: {e}"))?;
                    if !out.status.success() {
                        return Err(format!(
                            "Checkpoint failed: {}",
                            String::from_utf8_lossy(&out.stderr)
                        ));
                    }
                    Ok(format!(
                        "Checkpoint '{name}' saved. Restore with: file_history(action=\"rollback\", name=\"{name}\")"
                    ))
                }
                "rollback" => {
                    let name = args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .ok_or("name required")?;
                    let pattern = format!("flamel-checkpoint-{name}");
                    let list = flamel::mcp_parent_guard::command("git")
                        .args(["stash", "list"])
                        .current_dir(project_path)
                        .output()
                        .map_err(|e| format!("git not found: {e}"))?;
                    let list_str = String::from_utf8_lossy(&list.stdout).to_string();
                    let stash_ref = list_str
                        .lines()
                        .find(|l| l.contains(&pattern))
                        .and_then(|l| l.split(':').next())
                        .ok_or_else(|| format!("Checkpoint '{name}' not found.\n{list_str}"))?
                        .to_string();
                    let out = flamel::mcp_parent_guard::command("git")
                        .args(["stash", "pop", &stash_ref])
                        .current_dir(project_path)
                        .output()
                        .map_err(|e| format!("git stash pop failed: {e}"))?;
                    if !out.status.success() {
                        return Err(format!(
                            "Rollback failed: {}",
                            String::from_utf8_lossy(&out.stderr)
                        ));
                    }
                    Ok(format!(
                        "Rolled back to checkpoint '{name}'.\n{}",
                        String::from_utf8_lossy(&out.stdout).trim()
                    ))
                }
                _ => Err(format!(
                    "Unknown file_history action '{action}'. Use: revert, list, checkpoint, rollback"
                )),
            }
        }

        "flamel_context_stats" => {
            if session_id.is_empty() {
                return Err("FLAMEL_SESSION_ID not set — cannot retrieve context stats".to_string());
            }
            let stats = call_api(
                "GET",
                &format!("/api/sessions/{session_id}/context-stats"),
                None,
                base_url,
                token,
            )?;
            let input = stats["input_tokens"].as_i64().unwrap_or(0);
            let output = stats["output_tokens"].as_i64().unwrap_or(0);
            let window = stats["context_window"].as_i64().unwrap_or(200_000);
            let fill = stats["fill_pct"].as_f64().unwrap_or(0.0);
            let remaining = stats["remaining_tokens"].as_i64().unwrap_or(0);
            let threshold = stats["auto_compact_threshold"].as_i64().unwrap_or(180_000);
            let turns = stats["turns_remaining"].as_i64();

            let warning = if fill >= 90.0 {
                " ⚠ CRITICAL — compact soon"
            } else if fill >= 75.0 {
                " ⚠ getting full"
            } else {
                ""
            };

            let turns_str = turns
                .map(|t| format!("\nEstimated turns remaining: ~{t}"))
                .unwrap_or_default();
            Ok(format!(
                "Context: {input} / {window} tokens ({fill:.1}% full){warning}\nOutput so far: {output} tokens\nRemaining: {remaining} tokens\nAuto-compact triggers at: {threshold} tokens{turns_str}"
            ))
        }

        "diagnostics" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("runtime_logs");
            match action {
                "runtime_logs" | "logs" => {
                    let diagnostics =
                        call_api("GET", &build_runtime_logs_path(args), None, base_url, token)?;
                    Ok(format_runtime_logs(&diagnostics))
                }
                _ => Err(format!(
                    "unknown diagnostics action: {action}. Valid: runtime_logs"
                )),
            }
        }

        "capability_search" => {
            let automation_grants = automation_capability_grants()?;
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let category = args
                .get("category")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let connected_only = args
                .get("connected_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let path = if project_path.is_empty() {
                "/api/capabilities/catalog".to_string()
            } else {
                format!(
                    "/api/capabilities/catalog?projectPath={}",
                    urlencoding::encode(project_path)
                )
            };
            let response = call_api("GET", &path, None, base_url, token)?;
            let mut matches: Vec<Value> = response
                .get("capabilities")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|entry| {
                    let haystack = format!(
                        "{} {} {} {}",
                        entry.get("title").and_then(Value::as_str).unwrap_or(""),
                        entry
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                        entry.get("category").and_then(Value::as_str).unwrap_or(""),
                        entry.get("slug").and_then(Value::as_str).unwrap_or("")
                    )
                    .to_ascii_lowercase();
                    (query.is_empty() || haystack.contains(&query))
                        && (category.is_empty()
                            || entry
                                .get("category")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .eq_ignore_ascii_case(&category))
                        && (!connected_only
                            || entry
                                .get("connected")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                            || entry
                                .get("builtin")
                                .and_then(Value::as_bool)
                                .unwrap_or(false))
                        && automation_grants.as_ref().is_none_or(|grants| {
                            entry
                                .get("id")
                                .and_then(Value::as_str)
                                .is_some_and(|id| grants.contains_key(id))
                        })
                })
                .take(20)
                .cloned()
                .collect();
            for entry in &mut matches {
                if entry.get("connected").and_then(Value::as_bool) == Some(true)
                    && entry.get("builtin").and_then(Value::as_bool) != Some(true)
                {
                    if let Some(id) = entry.get("id").and_then(Value::as_str).map(str::to_string) {
                        if let Ok(tools) = call_api(
                            "GET",
                            &format!("/api/capabilities/{}/tools", urlencoding::encode(&id)),
                            None,
                            base_url,
                            token,
                        ) {
                            let mut reviewed_tools =
                                tools.get("tools").cloned().unwrap_or_else(|| json!([]));
                            if let Some(grants) = automation_grants.as_ref()
                                && let Some(allowed_tools) = grants.get(&id)
                                && let Some(tools) = reviewed_tools.as_array_mut()
                            {
                                tools.retain(|tool| {
                                    tool.get("name")
                                        .and_then(Value::as_str)
                                        .is_some_and(|name| allowed_tools.contains(name))
                                });
                            }
                            entry["tools"] = reviewed_tools;
                        }
                        if let Ok(surfaces) = call_api(
                            "GET",
                            &format!("/api/capabilities/{}/surfaces", urlencoding::encode(&id)),
                            None,
                            base_url,
                            token,
                        ) {
                            entry["surfaces"] = surfaces
                                .get("surfaces")
                                .cloned()
                                .unwrap_or_else(|| json!([]));
                        }
                    }
                }
            }
            Ok(json!({
                "count": matches.len(),
                "capabilities": matches,
                "admin_managed": response.get("adminManaged").cloned().unwrap_or(json!(true)),
            })
            .to_string())
        }

        "capability_execute" => {
            let automation_grants = automation_capability_grants()?;
            let capability_id = args
                .get("capability_id")
                .and_then(Value::as_str)
                .ok_or("capability_id required")?;
            let tool_name = args
                .get("tool_name")
                .and_then(Value::as_str)
                .ok_or("tool_name required")?;
            if automation_grants.as_ref().is_some_and(|grants| {
                grants
                    .get(capability_id)
                    .is_none_or(|tools| !tools.contains(tool_name))
            }) {
                return Err(format!(
                    "automation capability {capability_id}/{tool_name} was not explicitly granted"
                ));
            }
            let mut path = format!(
                "/api/capabilities/{}/execute",
                urlencoding::encode(capability_id)
            );
            if !project_path.is_empty() {
                path.push_str(&format!(
                    "?projectPath={}",
                    urlencoding::encode(project_path)
                ));
            }
            let body = json!({"toolName":tool_name,"arguments":args.get("arguments").cloned().unwrap_or_else(||json!({})),"sessionId":session_id,"background":args.get("background").and_then(Value::as_bool).unwrap_or(false)});
            let response = call_api("POST", &path, Some(&body), base_url, token)?;
            Ok(response.to_string())
        }

        "project_intelligence" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("context");
            let path = args
                .get("project_path")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .unwrap_or(project_path);
            if path.is_empty() {
                return Err("project_path required".into());
            }
            let (endpoint, body) = match action {
                "index" => (
                    "/api/project-intelligence/index",
                    json!({"projectPath":path}),
                ),
                "verification" => (
                    "/api/project-intelligence/verification",
                    json!({"projectPath":path}),
                ),
                "verify" => (
                    "/api/project-intelligence/verify",
                    json!({"projectPath":path,"checks":args.get("checks").cloned().unwrap_or_else(||json!([]))}),
                ),
                "context" => (
                    "/api/project-intelligence/context",
                    json!({"projectPath":path,"query":args.get("query").and_then(Value::as_str).unwrap_or(""),"maxTokens":args.get("max_tokens").and_then(Value::as_u64).unwrap_or(4000)}),
                ),
                _ => return Err("action must be index, context, verification, or verify".into()),
            };
            Ok(call_api("POST", endpoint, Some(&body), base_url, token)?.to_string())
        }

        "skill_search" => {
            let query = args.get("query").and_then(Value::as_str).unwrap_or("");
            let scoped_project = args
                .get("project_path")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or(project_path);
            let provider = std::env::var("FLAMEL_PROVIDER").unwrap_or_default();
            Ok(call_api(
                "GET",
                &format!(
                    "/api/capabilities/skills?query={}&projectPath={}&provider={}",
                    urlencoding::encode(query),
                    urlencoding::encode(scoped_project),
                    urlencoding::encode(&provider),
                ),
                None,
                base_url,
                token,
            )?
            .to_string())
        }

        "skill_catalog" => {
            let action = args.get("action").and_then(Value::as_str).unwrap_or("list");
            let id = args.get("id").and_then(Value::as_str).unwrap_or("");
            let scoped_project = args
                .get("project_path")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or(project_path);
            let encoded_id = urlencoding::encode(id);
            let response = match action {
                "list" | "search" => call_api(
                    "GET",
                    &format!(
                        "/api/capabilities/skills?query={}&projectPath={}&provider={}",
                        urlencoding::encode(
                            args.get("query").and_then(Value::as_str).unwrap_or("")
                        ),
                        urlencoding::encode(scoped_project),
                        urlencoding::encode(
                            args.get("provider")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                        ),
                    ),
                    None,
                    base_url,
                    token,
                ),
                "inspect" => {
                    if id.is_empty() {
                        return Err("id required for inspect".to_string());
                    }
                    call_api(
                        "GET",
                        &format!(
                            "/api/capabilities/skills/{encoded_id}?projectPath={}",
                            urlencoding::encode(scoped_project)
                        ),
                        None,
                        base_url,
                        token,
                    )
                }
                "propose" => call_api(
                    "POST",
                    "/api/capabilities/skills",
                    Some(&json!({
                        "slug":args.get("slug").and_then(Value::as_str).unwrap_or(""),
                        "skill":args.get("skill").cloned().unwrap_or_else(||json!({})),
                    })),
                    base_url,
                    token,
                ),
                "review" => {
                    if id.is_empty() {
                        return Err("id required for review".to_string());
                    }
                    call_api(
                        "POST",
                        &format!("/api/capabilities/skills/{encoded_id}/review"),
                        Some(&json!({
                            "version":args.get("version").and_then(Value::as_str).unwrap_or(""),
                            "action":args.get("review_action").and_then(Value::as_str).unwrap_or("reject"),
                        })),
                        base_url,
                        token,
                    )
                }
                "add_version" => {
                    if id.is_empty() {
                        return Err("id required for add_version".to_string());
                    }
                    call_api(
                        "POST",
                        &format!("/api/capabilities/skills/{encoded_id}/versions"),
                        args.get("skill"),
                        base_url,
                        token,
                    )
                }
                "install" => {
                    if id.is_empty() {
                        return Err("id required for install".to_string());
                    }
                    call_api(
                        "POST",
                        &format!("/api/capabilities/skills/{encoded_id}/install"),
                        Some(&json!({
                            "version":args.get("version"),
                            "projectPath":if scoped_project.is_empty() { Value::Null } else { json!(scoped_project) },
                            "providerProjectionEnabled":args
                                .get("provider_projection_enabled")
                                .and_then(Value::as_bool)
                                .unwrap_or(true),
                        })),
                        base_url,
                        token,
                    )
                }
                "rollback" => {
                    if id.is_empty() {
                        return Err("id required for rollback".to_string());
                    }
                    call_api(
                        "POST",
                        &format!("/api/capabilities/skills/{encoded_id}/rollback"),
                        Some(&json!({"projectPath":if scoped_project.is_empty() { Value::Null } else { json!(scoped_project) }})),
                        base_url,
                        token,
                    )
                }
                "enable" | "disable" => {
                    if id.is_empty() {
                        return Err("id required for enable or disable".to_string());
                    }
                    call_api(
                        "PATCH",
                        &format!("/api/capabilities/skills/{encoded_id}"),
                        Some(&json!({
                            "enabled":action == "enable",
                            "projectPath":if scoped_project.is_empty() { Value::Null } else { json!(scoped_project) },
                            "providerProjectionEnabled":args.get("provider_projection_enabled"),
                        })),
                        base_url,
                        token,
                    )
                }
                "uninstall" => {
                    if id.is_empty() {
                        return Err("id required for uninstall".to_string());
                    }
                    call_api(
                        "DELETE",
                        &format!("/api/capabilities/skills/{encoded_id}"),
                        Some(&json!({"projectPath":if scoped_project.is_empty() { Value::Null } else { json!(scoped_project) }})),
                        base_url,
                        token,
                    )
                }
                _ => Err("action must be list, search, inspect, propose, review, add_version, install, rollback, enable, disable, or uninstall".to_string()),
            }?;
            Ok(response.to_string())
        }

        "work" => {
            let action = args.get("action").and_then(Value::as_str).unwrap_or("list");
            let id = args.get("id").and_then(Value::as_str).unwrap_or("");
            let pp = args
                .get("project_path")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .unwrap_or(project_path);
            let response = match action {
                "list" | "search" => call_api(
                    "GET",
                    &format!(
                        "/api/work-items?projectPath={}&query={}",
                        urlencoding::encode(pp),
                        urlencoding::encode(
                            args.get("query").and_then(Value::as_str).unwrap_or("")
                        )
                    ),
                    None,
                    base_url,
                    token,
                )?,
                "get" => call_api(
                    "GET",
                    &format!("/api/work-items/{id}"),
                    None,
                    base_url,
                    token,
                )?,
                "create" => {
                    let mut body = args.get("item").cloned().unwrap_or_else(|| json!({}));
                    if let Some(object) = body.as_object_mut() {
                        object.insert("projectPath".into(), json!(pp));
                    }
                    call_api("POST", "/api/work-items", Some(&body), base_url, token)?
                }
                "update" | "assign" | "review" | "link_plan" | "release_status" => {
                    let body = args.get("item").cloned().unwrap_or_else(|| json!({}));
                    call_api(
                        "PUT",
                        &format!("/api/work-items/{id}"),
                        Some(&body),
                        base_url,
                        token,
                    )?
                }
                "comment" => {
                    let body =
                        json!({"body":args.get("body").and_then(Value::as_str).unwrap_or("")});
                    call_api(
                        "POST",
                        &format!("/api/work-items/{id}/comments"),
                        Some(&body),
                        base_url,
                        token,
                    )?
                }
                "add_dependency" => {
                    let body = json!({"dependsOnId":args.get("depends_on_id").and_then(Value::as_str).unwrap_or("")});
                    call_api(
                        "POST",
                        &format!("/api/work-items/{id}/dependencies"),
                        Some(&body),
                        base_url,
                        token,
                    )?
                }
                "add_artifact" => {
                    let body = args.get("artifact").cloned().unwrap_or_else(|| json!({}));
                    call_api(
                        "POST",
                        &format!("/api/work-items/{id}/artifacts"),
                        Some(&body),
                        base_url,
                        token,
                    )?
                }
                _ => return Err("unknown work action".into()),
            };
            Ok(response.to_string())
        }
        _ => Err(format!("Unknown control tool: {name}")),
    }
}
