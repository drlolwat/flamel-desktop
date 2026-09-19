use super::*;

pub(super) fn handle(
    name: &str,
    args: &Value,
    _base_url: &str,
    _token: &str,
    session_id: &str,
    project_path: &str,
) -> Result<String, String> {
    match name {
        "code_nav"
        | "flamel_search"
        | "get_symbols_overview"
        | "find_symbol"
        | "find_referencing_symbols"
        | "get_imports"
        | "gather_context"
        | "dependency_graph" => {
            let (_, requested_action) =
                engineering_tool_action(name, args).expect("engineering tool must have an action");
            let action = match requested_action.as_str() {
                "flamel_search" => "search",
                "get_symbols_overview" => "symbols",
                "find_symbol" => "find",
                "find_referencing_symbols" => "refs",
                "get_imports" => "imports",
                "gather_context" => "context",
                "dependency_graph" => "deps",
                action => action,
            };
            match action {
                "search" => {
                    let pattern = args
                        .get("pattern")
                        .and_then(Value::as_str)
                        .ok_or("pattern required")?;
                    let requested_path = args.get("path").and_then(Value::as_str).unwrap_or("");
                    let full_path = if requested_path.is_empty() {
                        project_path.to_string()
                    } else {
                        resolve_path(requested_path, project_path)
                    };
                    let include = args.get("include").and_then(Value::as_str);
                    let context = args.get("context").and_then(Value::as_u64).unwrap_or(2);
                    let (records, warnings) =
                        run_rg_records(pattern, &full_path, context, include, project_path)?;
                    let match_count = records.iter().filter(|row| row["role"] == "match").count();
                    let context_count = records.len().saturating_sub(match_count);
                    let file_count = records
                        .iter()
                        .filter_map(|row| row["path"].as_str())
                        .collect::<HashSet<_>>()
                        .len();
                    let full_text = format_rg_records(&records, "No matches found.");
                    let (bounded, truncated) = bounded_rg_records(&records);
                    Ok(engineering_result(
                        "code_nav",
                        action,
                        full_text,
                        json!({
                            "query": pattern,
                            "root": requested_path,
                            "include": include,
                            "contextLines": context,
                            "matches": bounded,
                        }),
                        json!({
                            "matches": match_count,
                            "contexts": context_count,
                            "rows": records.len(),
                            "files": file_count,
                        }),
                        truncated,
                        warnings,
                    ))
                }
                "symbols" => {
                    let file = args
                        .get("relative_path")
                        .or_else(|| args.get("path"))
                        .and_then(Value::as_str)
                        .ok_or("relative_path required")?;
                    let full = resolve_path(file, project_path);
                    let indexed = mcp_symbols::index_file(std::path::Path::new(&full))?;
                    let all_symbols = indexed["symbols"].as_array().cloned().unwrap_or_default();
                    let (symbols, symbol_count, truncated) =
                        bounded_symbol_tree(&all_symbols, ENGINEERING_MAX_ROWS);
                    let mut full_text = format!("Symbols in {file}:\n");
                    if all_symbols.is_empty() {
                        full_text = format!("No symbols found in {file}");
                    } else {
                        for symbol in &all_symbols {
                            full_text.push_str(&format!(
                                "  [{}] {} (L{}-{})\n",
                                symbol["kind"].as_str().unwrap_or_default(),
                                symbol["name"].as_str().unwrap_or_default(),
                                symbol["startLine"].as_u64().unwrap_or(0),
                                symbol["endLine"].as_u64().unwrap_or(0),
                            ));
                            if let Some(methods) = symbol["methods"].as_array() {
                                for method in methods {
                                    full_text.push_str(&format!(
                                        "    [{}] {} (L{}-{})\n",
                                        method["kind"].as_str().unwrap_or_default(),
                                        method["name"].as_str().unwrap_or_default(),
                                        method["startLine"].as_u64().unwrap_or(0),
                                        method["endLine"].as_u64().unwrap_or(0),
                                    ));
                                }
                            }
                        }
                    }
                    Ok(engineering_result(
                        "code_nav",
                        action,
                        full_text,
                        json!({
                            "path": file,
                            "language": indexed["language"],
                            "symbols": symbols,
                        }),
                        json!({"symbols": symbol_count}),
                        truncated,
                        Vec::new(),
                    ))
                }
                "find" => {
                    let mut result =
                        mcp_symbols::find_symbol_result(&with_resolved_path(args, project_path))?;
                    let file = args
                        .get("relative_path")
                        .or_else(|| args.get("path"))
                        .and_then(Value::as_str)
                        .ok_or("relative_path required")?;
                    result["path"] = json!(file);
                    let start_line = result["range"]["startLine"].as_u64().unwrap_or(0) as usize;
                    let full_source = result["source"].as_str().unwrap_or_default().to_string();
                    let children = result["children"].as_array().cloned().unwrap_or_default();
                    let mut full_text = format!(
                        "Symbol '{}' in {} (L{}-{}):\n",
                        result["namePath"].as_str().unwrap_or_default(),
                        file,
                        start_line,
                        result["range"]["endLine"].as_u64().unwrap_or(0),
                    );
                    if !full_source.is_empty() {
                        full_text.push_str(&numbered_source(&full_source, start_line));
                        full_text.push('\n');
                    }
                    for child in &children {
                        full_text.push_str(&format!(
                            "  child: [{}] {} (L{}-{})\n",
                            child["kind"].as_str().unwrap_or_default(),
                            child["name"].as_str().unwrap_or_default(),
                            child["startLine"].as_u64().unwrap_or(0),
                            child["endLine"].as_u64().unwrap_or(0),
                        ));
                    }
                    let (source, source_truncated) =
                        bounded_utf8(&full_source, ENGINEERING_SOURCE_BYTES);
                    result["source"] = if full_source.is_empty() {
                        Value::Null
                    } else {
                        json!(source)
                    };
                    if let Some(items) = result["children"].as_array_mut() {
                        items.truncate(ENGINEERING_MAX_ROWS);
                        for child in items {
                            for field in ["name", "kind"] {
                                if let Some(value) = child[field].as_str() {
                                    child[field] =
                                        json!(bounded_utf8(value, ENGINEERING_FIELD_BYTES).0);
                                }
                            }
                        }
                    }
                    Ok(engineering_result(
                        "code_nav",
                        action,
                        full_text,
                        result,
                        json!({
                            "symbols": 1,
                            "children": children.len(),
                            "sourceLines": full_source.lines().count(),
                        }),
                        source_truncated || children.len() > ENGINEERING_MAX_ROWS,
                        Vec::new(),
                    ))
                }
                "refs" => {
                    let symbol = args
                        .get("symbol_name")
                        .or_else(|| args.get("name"))
                        .and_then(Value::as_str)
                        .ok_or("symbol_name required")?;
                    let requested_path = args
                        .get("relative_path")
                        .or_else(|| args.get("path"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let full_path = if requested_path.is_empty() {
                        project_path.to_string()
                    } else {
                        resolve_path(requested_path, project_path)
                    };
                    let context = args
                        .get("context_lines")
                        .and_then(Value::as_u64)
                        .unwrap_or(2);
                    let (records, warnings) =
                        run_rg_records(symbol, &full_path, context, None, project_path)?;
                    let match_count = records.iter().filter(|row| row["role"] == "match").count();
                    let context_count = records.len().saturating_sub(match_count);
                    let file_count = records
                        .iter()
                        .filter_map(|row| row["path"].as_str())
                        .collect::<HashSet<_>>()
                        .len();
                    let full_text = format_rg_records(
                        &records,
                        &format!("No references to '{symbol}' found in {requested_path}"),
                    );
                    let (bounded, truncated) = bounded_rg_records(&records);
                    Ok(engineering_result(
                        "code_nav",
                        action,
                        full_text,
                        json!({
                            "symbol": symbol,
                            "root": requested_path,
                            "contextLines": context,
                            "matches": bounded,
                        }),
                        json!({
                            "matches": match_count,
                            "contexts": context_count,
                            "rows": records.len(),
                            "files": file_count,
                        }),
                        truncated,
                        warnings,
                    ))
                }
                "imports" => {
                    let file = args
                        .get("file")
                        .and_then(Value::as_str)
                        .ok_or("file required")?;
                    let full = resolve_path(file, project_path);
                    let content = std::fs::read_to_string(&full)
                        .map_err(|error| format!("Cannot read {file}: {error}"))?;
                    let extension = std::path::Path::new(file)
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default();
                    let imports = engineering_imports(&content, extension);
                    let full_text = if imports.is_empty() {
                        format!("No imports in {file} ({} lines).", content.lines().count())
                    } else {
                        format!(
                            "{} imports in {file} ({} lines):\n{}",
                            imports.len(),
                            content.lines().count(),
                            imports
                                .iter()
                                .map(|entry| format!(
                                    "{:>6}\t{}",
                                    entry["line"].as_u64().unwrap_or(0),
                                    entry["text"].as_str().unwrap_or_default(),
                                ))
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )
                    };
                    let truncated = imports.len() > ENGINEERING_MAX_ROWS;
                    let bounded = imports
                        .iter()
                        .take(ENGINEERING_MAX_ROWS)
                        .cloned()
                        .collect::<Vec<_>>();
                    Ok(engineering_result(
                        "code_nav",
                        action,
                        full_text,
                        json!({
                            "path": file,
                            "language": engineering_language(file),
                            "totalLines": content.lines().count(),
                            "imports": bounded,
                        }),
                        json!({"imports": imports.len()}),
                        truncated,
                        Vec::new(),
                    ))
                }
                "context" => {
                    let task = args
                        .get("task")
                        .and_then(Value::as_str)
                        .ok_or("task required")?;
                    let max_files =
                        args.get("max_files").and_then(Value::as_u64).unwrap_or(5) as usize;
                    let stop_words = [
                        "the", "a", "an", "is", "in", "it", "to", "of", "and", "or", "for", "with",
                        "that", "this", "from", "by", "be", "are", "was", "has", "have", "do",
                        "does", "not", "on", "at", "as", "we", "our", "its", "can", "fix", "add",
                        "use", "make", "get", "set",
                    ];
                    let mut keywords = task
                        .split(|character: char| !character.is_alphanumeric() && character != '_')
                        .filter(|word| word.len() >= 3)
                        .map(str::to_lowercase)
                        .filter(|word| !stop_words.contains(&word.as_str()))
                        .collect::<Vec<_>>();
                    keywords.sort();
                    keywords.dedup();
                    if keywords.is_empty() {
                        return Err("No keywords extracted from task".to_string());
                    }
                    let pattern = keywords.join("|");
                    let output = flamel::mcp_parent_guard::command("rg")
                        .args([
                            "--color=never",
                            "--no-heading",
                            "--with-filename",
                            "-c",
                            "--glob",
                            "!target/**",
                            "--glob",
                            "!node_modules/**",
                            "--glob",
                            "!.git/**",
                            &pattern,
                            project_path,
                        ])
                        .output()
                        .map_err(|error| format!("rg not found: {error}"))?;
                    if !output.status.success() && output.status.code() != Some(1) {
                        return Err(format!(
                            "rg error: {}",
                            String::from_utf8_lossy(&output.stderr).trim()
                        ));
                    }
                    let mut scores = HashMap::<String, usize>::new();
                    for line in String::from_utf8_lossy(&output.stdout).lines() {
                        if let Some(separator) = line.rfind(':') {
                            let path = engineering_display_path(&line[..separator], project_path);
                            *scores.entry(path).or_insert(0) +=
                                line[separator + 1..].parse::<usize>().unwrap_or(0);
                        }
                    }
                    for keyword in &keywords {
                        for (path, score) in &mut scores {
                            if path.to_lowercase().contains(keyword) {
                                *score += 10;
                            }
                        }
                    }
                    let mut ranked = scores.into_iter().collect::<Vec<_>>();
                    ranked.sort_by(|left, right| {
                        right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0))
                    });
                    ranked.truncate(max_files);
                    let ranked_count = ranked.len();
                    let mut payload_truncated = ranked_count > ENGINEERING_MAX_ROWS;
                    let mut remaining_symbols = ENGINEERING_MAX_ROWS;
                    let mut files = Vec::new();
                    let mut full_text = if ranked.is_empty() {
                        "No relevant files found.".to_string()
                    } else {
                        format!("Top {ranked_count} files for: \"{task}\"")
                    };
                    for (path, score) in &ranked {
                        full_text.push_str(&format!("\n\n### {path} (score: {score})"));
                        if files.len() >= ENGINEERING_MAX_ROWS {
                            continue;
                        }
                        let full = resolve_path(path, project_path);
                        let indexed = mcp_symbols::index_file(std::path::Path::new(&full)).ok();
                        let all_symbols = indexed
                            .as_ref()
                            .and_then(|value| value["symbols"].as_array())
                            .cloned()
                            .unwrap_or_default();
                        let (symbols, _, symbols_truncated) =
                            bounded_symbol_tree(&all_symbols, remaining_symbols);
                        payload_truncated |= symbols_truncated;
                        remaining_symbols = remaining_symbols.saturating_sub(
                            symbols
                                .iter()
                                .map(|symbol| {
                                    1 + symbol["methods"].as_array().map(Vec::len).unwrap_or(0)
                                })
                                .sum::<usize>(),
                        );
                        for symbol in &symbols {
                            full_text.push_str(&format!(
                                "\n  [{}] {} (L{}-{})",
                                symbol["kind"].as_str().unwrap_or_default(),
                                symbol["name"].as_str().unwrap_or_default(),
                                symbol["startLine"].as_u64().unwrap_or(0),
                                symbol["endLine"].as_u64().unwrap_or(0),
                            ));
                        }
                        files.push(json!({
                            "path": path,
                            "score": score,
                            "language": indexed
                                .as_ref()
                                .and_then(|value| value["language"].as_str())
                                .unwrap_or_else(|| engineering_language(path)),
                            "symbols": symbols,
                        }));
                    }
                    Ok(engineering_result(
                        "code_nav",
                        action,
                        full_text,
                        json!({"task": task, "keywords": keywords, "files": files}),
                        json!({"files": ranked_count}),
                        payload_truncated,
                        Vec::new(),
                    ))
                }
                "deps" => {
                    let file = args
                        .get("file")
                        .and_then(Value::as_str)
                        .ok_or("file required")?;
                    let full = resolve_path(file, project_path);
                    let content = std::fs::read_to_string(&full)
                        .map_err(|error| format!("Cannot read {file}: {error}"))?;
                    let extension = std::path::Path::new(file)
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default();
                    let imports = engineering_imports(&content, extension);
                    let outbound = imports
                        .iter()
                        .map(|entry| {
                            json!({
                                "from": file,
                                "to": entry["text"],
                                "kind": "import",
                                "line": entry["line"],
                            })
                        })
                        .collect::<Vec<_>>();
                    let stem = std::path::Path::new(file)
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default();
                    let output = flamel::mcp_parent_guard::command("rg")
                        .args([
                            "--color=never",
                            "--with-filename",
                            "-l",
                            "--glob",
                            "!target/**",
                            "--glob",
                            "!node_modules/**",
                            "--glob",
                            "!.git/**",
                            stem,
                            project_path,
                        ])
                        .output()
                        .map_err(|error| format!("rg not found: {error}"))?;
                    if !output.status.success() && output.status.code() != Some(1) {
                        return Err(format!(
                            "rg error: {}",
                            String::from_utf8_lossy(&output.stderr).trim()
                        ));
                    }
                    let inbound = String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(|path| engineering_display_path(path, project_path))
                        .filter(|path| path != file)
                        .map(|path| json!({"from": path, "to": file, "kind": "reference"}))
                        .collect::<Vec<_>>();
                    let mut full_text = format!(
                        "Dependency graph for {file}:\n\nOutbound ({})\n",
                        outbound.len()
                    );
                    for edge in &outbound {
                        full_text
                            .push_str(&format!("  {}\n", edge["to"].as_str().unwrap_or_default()));
                    }
                    full_text.push_str(&format!("\nInbound ({})\n", inbound.len()));
                    for edge in &inbound {
                        full_text.push_str(&format!(
                            "  {}\n",
                            edge["from"].as_str().unwrap_or_default()
                        ));
                    }
                    let truncated = outbound.len() > ENGINEERING_MAX_ROWS
                        || inbound.len() > ENGINEERING_MAX_ROWS;
                    Ok(engineering_result(
                        "code_nav",
                        action,
                        full_text,
                        json!({
                            "path": file,
                            "outbound": outbound
                                .iter()
                                .take(ENGINEERING_MAX_ROWS)
                                .cloned()
                                .collect::<Vec<_>>(),
                            "inbound": inbound
                                .iter()
                                .take(ENGINEERING_MAX_ROWS)
                                .cloned()
                                .collect::<Vec<_>>(),
                        }),
                        json!({"outbound": outbound.len(), "inbound": inbound.len()}),
                        truncated,
                        Vec::new(),
                    ))
                }
                _ => Err(format!(
                    "Unknown code_nav action '{action}'. Use: search, symbols, find, refs, imports, context, deps"
                )),
            }
        }

        // ── File editing (consolidated: replace/replace_content/multi_replace/replace_symbol/insert_before/insert_after/create/delete/diff_apply/format/preview) ──
        "file_edit"
        | "flamel_replace"
        | "replace_content"
        | "multi_replace"
        | "replace_symbol_body"
        | "insert_before_symbol"
        | "insert_after_symbol"
        | "create_text_file"
        | "diff_apply"
        | "format_file"
        | "preview_edit" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or(match name {
                    "replace_content" => "replace_content",
                    "multi_replace" => "multi_replace",
                    "replace_symbol_body" => "replace_symbol",
                    "insert_before_symbol" => "insert_before",
                    "insert_after_symbol" => "insert_after",
                    "create_text_file" => "create",
                    "diff_apply" => "diff_apply",
                    "format_file" => "format",
                    "preview_edit" => "preview",
                    _ => "replace",
                });
            match action {
                "replace" | "flamel_replace" => {
                    let file = args
                        .get("file")
                        .or_else(|| args.get("path"))
                        .or_else(|| args.get("relative_path"))
                        .and_then(|v| v.as_str())
                        .ok_or("file required")?;
                    let old_string = args
                        .get("old_string")
                        .or_else(|| args.get("old"))
                        .or_else(|| args.get("needle"))
                        .and_then(|v| v.as_str())
                        .ok_or("old_string required")?;
                    let new_string = args
                        .get("new_string")
                        .or_else(|| args.get("new"))
                        .or_else(|| args.get("repl"))
                        .or_else(|| args.get("replacement"))
                        .and_then(|v| v.as_str())
                        .ok_or("new_string required")?;
                    let full_path = resolve_path(file, project_path);
                    check_read_before_edit(&full_path)?;
                    let content = std::fs::read_to_string(&full_path)
                        .map_err(|e| format!("Cannot read {file}: {e}"))?;
                    fn normalize_quotes(s: &str) -> String {
                        s.replace('\u{201C}', "\"")
                            .replace('\u{201D}', "\"")
                            .replace('\u{2018}', "'")
                            .replace('\u{2019}', "'")
                            .replace('\u{2032}', "'")
                            .replace('\u{FF02}', "\"")
                            .replace('\u{FF07}', "'")
                    }
                    let actual_needle = if content.contains(old_string) {
                        old_string.to_string()
                    } else {
                        let norm = normalize_quotes(old_string);
                        if norm != old_string && content.contains(norm.as_str()) {
                            norm
                        } else {
                            return Err(format!("old_string not found in {file}"));
                        }
                    };
                    let count = content.matches(actual_needle.as_str()).count();
                    if count > 1 {
                        return Err(format!(
                            "old_string matches {count} times in {file} — make it more specific"
                        ));
                    }
                    let new_content = content.replacen(actual_needle.as_str(), new_string, 1);
                    backup_file(&full_path, &session_id);
                    std::fs::write(&full_path, &new_content)
                        .map_err(|e| format!("Cannot write {file}: {e}"))?;
                    record_file_written(&full_path);
                    let diff =
                        mcp_symbols::make_unified_diff(&content, &actual_needle, new_string, file);
                    let (added, removed) = mcp_symbols::unified_diff_stats(&diff);
                    let mut out = if diff.is_empty() {
                        format!("Replaced in {file}")
                    } else {
                        format!("Replaced in {file} (+{added}/-{removed})\n\n{diff}")
                    };
                    Ok(out)
                }
                "replace_content" => {
                    let resolved = with_resolved_path(args, project_path);
                    let path = resolved
                        .get("relative_path")
                        .or_else(|| resolved.get("path"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !path.is_empty() {
                        check_read_before_edit(path)?;
                        backup_file(path, session_id);
                    }
                    let result = mcp_symbols::replace_content(&resolved)?;
                    if !path.is_empty() {
                        record_file_written(path);
                    }
                    Ok(result)
                }
                "multi_replace" => {
                    let edits = coerce_array(args.get("edits"));
                    let edits_arr = edits
                        .as_array()
                        .ok_or("edits required — array of {file, old_string, new_string}")?;
                    let mut staged: HashMap<String, (String, String)> = HashMap::new();
                    let mut file_order: Vec<String> = Vec::new();
                    let mut applied: Vec<String> = Vec::new();
                    let mut diffs: Vec<String> = Vec::new();
                    let mut added_total = 0usize;
                    let mut removed_total = 0usize;

                    for edit in edits_arr {
                        let file = edit
                            .get("file")
                            .and_then(|v| v.as_str())
                            .ok_or("each edit needs 'file'")?;
                        let old_s = edit
                            .get("old_string")
                            .and_then(|v| v.as_str())
                            .ok_or("each edit needs 'old_string'")?;
                        let new_s = edit
                            .get("new_string")
                            .and_then(|v| v.as_str())
                            .ok_or("each edit needs 'new_string'")?;
                        let full = resolve_path(file, project_path);
                        check_read_before_edit(&full)?;
                        if !staged.contains_key(&full) {
                            let content = std::fs::read_to_string(&full)
                                .map_err(|e| format!("Cannot read {file}: {e}"))?;
                            staged.insert(full.clone(), (file.to_string(), content));
                            file_order.push(full.clone());
                        }
                        let (_, content) = staged
                            .get_mut(&full)
                            .ok_or_else(|| format!("internal error staging {file}"))?;
                        let count = content.matches(old_s).count();
                        if count == 0 {
                            return Err(format!(
                                "old_string not found in {file} — no edits applied."
                            ));
                        }
                        if count > 1 {
                            return Err(format!(
                                "old_string matches {count} times in {file} — no edits applied."
                            ));
                        }
                        let diff = mcp_symbols::make_unified_diff(content, old_s, new_s, file);
                        let (added, removed) = mcp_symbols::unified_diff_stats(&diff);
                        if !diff.is_empty() {
                            diffs.push(diff);
                        }
                        added_total += added;
                        removed_total += removed;
                        *content = content.replacen(old_s, new_s, 1);
                        applied.push(format!("  ✓ {file}"));
                    }

                    for full in &file_order {
                        let (rel, content) = staged
                            .get(full)
                            .ok_or_else(|| format!("internal error writing {full}"))?;
                        backup_file(full, session_id);
                        std::fs::write(full, content)
                            .map_err(|e| format!("Cannot write {rel}: {e}"))?;
                        record_file_written(full);
                    }

                    let mut out = format!(
                        "Applied {}/{} edits (+{added_total}/-{removed_total}):\n{}",
                        applied.len(),
                        edits_arr.len(),
                        applied.join("\n")
                    );
                    if !diffs.is_empty() {
                        out.push_str("\n\n");
                        out.push_str(&diffs.join("\n"));
                    }
                    Ok(out)
                }
                "replace_symbol" | "replace_symbol_body" => {
                    let resolved = with_resolved_path(args, project_path);
                    let path = resolved
                        .get("relative_path")
                        .or_else(|| resolved.get("path"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !path.is_empty() {
                        check_read_before_edit(path)?;
                        backup_file(path, session_id);
                    }
                    let result = mcp_symbols::replace_symbol_body(&resolved)?;
                    if !path.is_empty() {
                        record_file_written(path);
                    }
                    Ok(result)
                }
                "insert_before" | "insert_before_symbol" => {
                    let resolved = with_resolved_path(args, project_path);
                    let path = resolved
                        .get("relative_path")
                        .or_else(|| resolved.get("path"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !path.is_empty() {
                        check_read_before_edit(path)?;
                        backup_file(path, session_id);
                    }
                    let result = mcp_symbols::insert_before_symbol(&resolved)?;
                    if !path.is_empty() {
                        record_file_written(path);
                    }
                    Ok(result)
                }
                "insert_after" | "insert_after_symbol" => {
                    let resolved = with_resolved_path(args, project_path);
                    let path = resolved
                        .get("relative_path")
                        .or_else(|| resolved.get("path"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !path.is_empty() {
                        check_read_before_edit(path)?;
                        backup_file(path, session_id);
                    }
                    let result = mcp_symbols::insert_after_symbol(&resolved)?;
                    if !path.is_empty() {
                        record_file_written(path);
                    }
                    Ok(result)
                }
                "create" | "create_text_file" => {
                    let resolved = with_resolved_path(args, project_path);
                    let rel = resolved
                        .get("relative_path")
                        .or_else(|| resolved.get("path"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let full = resolve_path(rel, project_path);
                    let exists = std::path::Path::new(&full).exists();
                    if exists {
                        check_read_before_edit(&full)?;
                        backup_file(&full, session_id);
                    }
                    let result = mcp_fs::create_text_file(&resolved)?;
                    record_file_written(&full);
                    if exists {
                        Ok(format!("⚠ OVERWROTE existing file: {rel}\n{result}"))
                    } else {
                        Ok(format!("Created: {rel}\n{result}"))
                    }
                }
                "delete" => {
                    let file = args
                        .get("file")
                        .or_else(|| args.get("path"))
                        .or_else(|| args.get("relative_path"))
                        .and_then(|v| v.as_str())
                        .ok_or("file required")?;
                    let full = resolve_path(file, project_path);
                    let meta =
                        std::fs::metadata(&full).map_err(|e| format!("Cannot stat {file}: {e}"))?;
                    if meta.is_dir() {
                        return Err(format!(
                            "{file} is a directory. file_edit(action=\"delete\") only deletes files."
                        ));
                    }
                    check_read_before_edit(&full)?;
                    backup_file(&full, session_id);
                    std::fs::remove_file(&full)
                        .map_err(|e| format!("Cannot delete {file}: {e}"))?;
                    record_file_deleted(&full);
                    Ok(format!("Deleted: {file}"))
                }
                "diff_apply" => {
                    let diff = args
                        .get("diff")
                        .and_then(|v| v.as_str())
                        .ok_or("diff required")?;
                    if diff.trim().is_empty() {
                        return Err("diff is empty".to_string());
                    }
                    let tmp = format!("/tmp/flamel-patch-{}.diff", uuid::Uuid::new_v4());
                    std::fs::write(&tmp, diff).map_err(|e| format!("Failed to write temp: {e}"))?;
                    let dry = flamel::mcp_parent_guard::command("patch")
                        .args(["--dry-run", "-p1", "-i", &tmp])
                        .current_dir(project_path)
                        .output()
                        .map_err(|e| {
                            let _ = std::fs::remove_file(&tmp);
                            format!("patch not found: {e}")
                        })?;
                    if !dry.status.success() {
                        let _ = std::fs::remove_file(&tmp);
                        return Err(format!(
                            "Patch would fail (dry run):\n{}{}",
                            String::from_utf8_lossy(&dry.stdout),
                            String::from_utf8_lossy(&dry.stderr)
                        ));
                    }
                    let apply = flamel::mcp_parent_guard::command("patch")
                        .args(["-p1", "-i", &tmp])
                        .current_dir(project_path)
                        .output()
                        .map_err(|e| format!("patch failed: {e}"))?;
                    let _ = std::fs::remove_file(&tmp);
                    let out = format!(
                        "{}{}",
                        String::from_utf8_lossy(&apply.stdout),
                        String::from_utf8_lossy(&apply.stderr)
                    );
                    if !apply.status.success() {
                        Err(format!("Patch failed:\n{out}"))
                    } else {
                        Ok(format!("Patch applied:\n{out}"))
                    }
                }
                "format" | "format_file" => {
                    let file = args
                        .get("file")
                        .and_then(|v| v.as_str())
                        .ok_or("file required")?;
                    let full = resolve_path(file, project_path);
                    let ext = std::path::Path::new(file)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    let original = std::fs::read_to_string(&full)
                        .map_err(|e| format!("Cannot read {file}: {e}"))?;
                    let fmt_cmd = match ext {
                        "rs" => format!("rustfmt \"{full}\""),
                        "ts" | "tsx" | "js" | "jsx" | "css" | "json" => {
                            format!("npx --yes prettier --write \"{full}\"")
                        }
                        "py" => format!("black \"{full}\""),
                        "go" => format!("gofmt -w \"{full}\""),
                        _ => return Err(format!("Unsupported file type '.{ext}'")),
                    };
                    let mut formatter = flamel::mcp_parent_guard::command("sh");
                    configure_shell_child(&mut formatter);
                    let output = formatter
                        .arg("-c")
                        .arg(&fmt_cmd)
                        .current_dir(project_path)
                        .output()
                        .map_err(|e| format!("Formatter failed: {e}"))?;
                    if !output.status.success() {
                        return Err(format!(
                            "Formatter failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        ));
                    }
                    let formatted = std::fs::read_to_string(&full)
                        .map_err(|e| format!("Cannot re-read {file}: {e}"))?;
                    if formatted == original {
                        Ok(format!("{file}: already formatted."))
                    } else {
                        let changed = original
                            .lines()
                            .zip(formatted.lines())
                            .filter(|(a, b)| a != b)
                            .count()
                            + original.lines().count().abs_diff(formatted.lines().count());
                        Ok(format!("{file}: formatted ({changed} lines changed)."))
                    }
                }
                "preview" | "preview_edit" => {
                    let file = args
                        .get("file")
                        .and_then(|v| v.as_str())
                        .ok_or("file required")?;
                    let old_s = args
                        .get("old_string")
                        .and_then(|v| v.as_str())
                        .ok_or("old_string required")?;
                    let new_s = args
                        .get("new_string")
                        .and_then(|v| v.as_str())
                        .ok_or("new_string required")?;
                    let full = resolve_path(file, project_path);
                    let content = std::fs::read_to_string(&full)
                        .map_err(|e| format!("Cannot read {file}: {e}"))?;
                    let count = content.matches(old_s).count();
                    if count == 0 {
                        return Err(format!("old_string not found in {file}"));
                    }
                    if count > 1 {
                        return Err(format!(
                            "old_string matches {count} times — be more specific"
                        ));
                    }
                    let new_content = content.replacen(old_s, new_s, 1);
                    let orig: Vec<&str> = content.lines().collect();
                    let nw: Vec<&str> = new_content.lines().collect();
                    let mut diff: Vec<String> = Vec::new();
                    let max = orig.len().max(nw.len());
                    for i in 0..max {
                        let o = orig.get(i).copied().unwrap_or("");
                        let n = nw.get(i).copied().unwrap_or("");
                        if o != n {
                            diff.push(format!("-{o}"));
                            if !n.is_empty() {
                                diff.push(format!("+{n}"));
                            }
                        }
                    }
                    if diff.is_empty() {
                        return Ok("No visible diff.".to_string());
                    }
                    Ok(format!(
                        "Preview of changes to {file}:\n{}",
                        diff.join("\n")
                    ))
                }
                _ => Err(format!(
                    "Unknown file_edit action '{action}'. Use: replace, replace_content, multi_replace, replace_symbol, insert_before, insert_after, create, delete, diff_apply, format, preview"
                )),
            }
        }

        // ── Consolidated file_read tool ──────────────────────────────────
        "file_read" | "flamel_read" | "read_file" | "batch_read" | "file_summary"
        | "flamel_glob" | "find_file" | "list_dir" => {
            let (_, requested_action) =
                engineering_tool_action(name, args).expect("engineering tool must have an action");
            let action = match requested_action.as_str() {
                "flamel_read" | "read_file" => "read",
                "batch_read" => "batch",
                "file_summary" => "summary",
                "flamel_glob" => "glob",
                "find_file" => "find",
                action => action,
            };
            match action {
                "read" => {
                    let file = args
                        .get("file")
                        .or_else(|| args.get("relative_path"))
                        .and_then(Value::as_str)
                        .ok_or("file required")?;
                    let requested_start = args.get("start_line").and_then(Value::as_u64);
                    let requested_end = args.get("end_line").and_then(Value::as_u64);
                    let tail = args.get("tail").and_then(Value::as_u64);
                    let full_path = resolve_path(file, project_path);
                    let content = std::fs::read_to_string(&full_path)
                        .map_err(|error| format!("Cannot read {file}: {error}"))?;
                    let lines = content.lines().collect::<Vec<_>>();
                    let total = lines.len();
                    let (from, to) = if let Some(tail) = tail {
                        (total.saturating_sub(tail as usize), total)
                    } else {
                        (
                            requested_start
                                .map(|line| (line as usize).saturating_sub(1))
                                .unwrap_or(0)
                                .min(total),
                            requested_end
                                .map(|line| line as usize)
                                .unwrap_or(total)
                                .min(total),
                        )
                    };
                    if from > to {
                        return Err(format!(
                            "invalid line range: start {} is after end {}",
                            requested_start.unwrap_or(1),
                            requested_end.unwrap_or(total as u64)
                        ));
                    }
                    let source = lines[from..to].join("\n");
                    let full_text = if total == 0 {
                        "(empty file)".to_string()
                    } else {
                        format!(
                            "{}\n\n[{file}: lines {}-{} of {total}]",
                            numbered_source(&source, from + 1),
                            from + 1,
                            to,
                        )
                    };
                    let (bounded_source, truncated) =
                        bounded_utf8(&source, ENGINEERING_SOURCE_BYTES);
                    record_file_read(&full_path);
                    Ok(engineering_result(
                        "file_read",
                        action,
                        full_text,
                        json!({
                            "files": [{
                                "path": file,
                                "requestedRange": {
                                    "startLine": requested_start,
                                    "endLine": requested_end,
                                    "tail": tail,
                                },
                                "resolvedRange": {
                                    "startLine": if total == 0 { 0 } else { from + 1 },
                                    "endLine": to,
                                },
                                "totalLines": total,
                                "language": engineering_language(file),
                                "source": bounded_source,
                            }],
                        }),
                        json!({"files": 1, "lines": to.saturating_sub(from), "totalLines": total}),
                        truncated,
                        Vec::new(),
                    ))
                }
                "batch" => {
                    let files = coerce_array(args.get("files"));
                    let requested = files
                        .as_array()
                        .ok_or("files required — array of paths or {path, start_line, end_line}")?;
                    let mut payload_files = Vec::new();
                    let mut full_parts = Vec::new();
                    let mut payload_bytes = 0usize;
                    let mut returned_lines = 0usize;
                    let mut total_lines = 0usize;
                    let mut truncated = requested.len() > ENGINEERING_MAX_ROWS;
                    for file_value in requested {
                        let (path, requested_start, requested_end) = match file_value {
                            Value::String(path) => (path.as_str(), None, None),
                            Value::Object(object) => (
                                object
                                    .get("path")
                                    .and_then(Value::as_str)
                                    .ok_or("each file object must have a 'path' field")?,
                                object.get("start_line").and_then(Value::as_u64),
                                object.get("end_line").and_then(Value::as_u64),
                            ),
                            _ => return Err(
                                "each entry must be a string path or {path, start_line, end_line}"
                                    .to_string(),
                            ),
                        };
                        let full = resolve_path(path, project_path);
                        let content = std::fs::read_to_string(&full)
                            .map_err(|error| format!("Cannot read {path}: {error}"))?;
                        let lines = content.lines().collect::<Vec<_>>();
                        let total = lines.len();
                        let from = requested_start
                            .map(|line| (line as usize).saturating_sub(1))
                            .unwrap_or(0)
                            .min(total);
                        let to = requested_end
                            .map(|line| line as usize)
                            .unwrap_or(total)
                            .min(total);
                        if from > to {
                            return Err(format!(
                                "invalid line range for {path}: start {} is after end {}",
                                requested_start.unwrap_or(1),
                                requested_end.unwrap_or(total as u64)
                            ));
                        }
                        let source = lines[from..to].join("\n");
                        let header = if requested_start.is_some() || requested_end.is_some() {
                            format!("=== {path} (lines {}-{to} of {total}) ===", from + 1)
                        } else {
                            format!("=== {path} ({total} lines) ===")
                        };
                        full_parts
                            .push(format!("{header}\n{}", numbered_source(&source, from + 1)));
                        record_file_read(&full);
                        returned_lines += to.saturating_sub(from);
                        total_lines += total;
                        if payload_files.len() >= ENGINEERING_MAX_ROWS {
                            continue;
                        }
                        let remaining = ENGINEERING_SOURCE_BYTES.saturating_sub(payload_bytes);
                        let (bounded_source, source_truncated) = bounded_utf8(&source, remaining);
                        payload_bytes += bounded_source.len();
                        truncated |= source_truncated;
                        payload_files.push(json!({
                            "path": path,
                            "requestedRange": {
                                "startLine": requested_start,
                                "endLine": requested_end,
                            },
                            "resolvedRange": {
                                "startLine": if total == 0 { 0 } else { from + 1 },
                                "endLine": to,
                            },
                            "totalLines": total,
                            "language": engineering_language(path),
                            "source": bounded_source,
                        }));
                    }
                    Ok(engineering_result(
                        "file_read",
                        action,
                        full_parts.join("\n\n"),
                        json!({"files": payload_files}),
                        json!({
                            "files": requested.len(),
                            "lines": returned_lines,
                            "totalLines": total_lines,
                        }),
                        truncated,
                        Vec::new(),
                    ))
                }
                "summary" => {
                    let file = args
                        .get("file")
                        .and_then(Value::as_str)
                        .ok_or("file required")?;
                    let full = resolve_path(file, project_path);
                    let content = std::fs::read_to_string(&full)
                        .map_err(|error| format!("Cannot read {file}: {error}"))?;
                    let lines = content.lines().collect::<Vec<_>>();
                    let extension = std::path::Path::new(file)
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default();
                    let purpose = lines
                        .iter()
                        .take(20)
                        .find(|line| {
                            let trimmed = line.trim_start();
                            trimmed.starts_with("//!")
                                || trimmed.starts_with("///")
                                || (trimmed.starts_with("//") && trimmed.len() > 4)
                                || trimmed.starts_with('#')
                                || trimmed.starts_with("/*")
                        })
                        .map(|line| {
                            line.trim()
                                .trim_start_matches("//!")
                                .trim_start_matches("///")
                                .trim_start_matches("//")
                                .trim_start_matches('#')
                                .trim_start_matches("/*")
                                .trim()
                                .to_string()
                        })
                        .unwrap_or_default();
                    let imports = engineering_imports(&content, extension);
                    let mut warnings = Vec::new();
                    let indexed = match mcp_symbols::index_file(std::path::Path::new(&full)) {
                        Ok(value) => Some(value),
                        Err(error) => {
                            warnings.push(format!("Symbols unavailable: {error}"));
                            None
                        }
                    };
                    let all_symbols = indexed
                        .as_ref()
                        .and_then(|value| value["symbols"].as_array())
                        .cloned()
                        .unwrap_or_default();
                    let (symbols, symbol_count, symbols_truncated) =
                        bounded_symbol_tree(&all_symbols, ENGINEERING_MAX_ROWS);
                    let imports_truncated = imports.len() > ENGINEERING_MAX_ROWS;
                    let bounded_imports = imports
                        .iter()
                        .take(ENGINEERING_MAX_ROWS)
                        .cloned()
                        .collect::<Vec<_>>();
                    let mut full_text =
                        format!("File: {file} ({} lines, .{extension})", lines.len());
                    if !purpose.is_empty() {
                        full_text.push_str(&format!("\nPurpose: {purpose}"));
                    }
                    if !imports.is_empty() {
                        full_text.push_str(&format!("\nImports: {}", imports.len()));
                    }
                    if !all_symbols.is_empty() {
                        full_text.push_str(&format!("\nSymbols: {symbol_count}"));
                        for symbol in &all_symbols {
                            full_text.push_str(&format!(
                                "\n  [{}] {} (L{}-{})",
                                symbol["kind"].as_str().unwrap_or_default(),
                                symbol["name"].as_str().unwrap_or_default(),
                                symbol["startLine"].as_u64().unwrap_or(0),
                                symbol["endLine"].as_u64().unwrap_or(0),
                            ));
                        }
                    }
                    record_file_read(&full);
                    Ok(engineering_result(
                        "file_read",
                        action,
                        full_text,
                        json!({
                            "path": file,
                            "language": engineering_language(file),
                            "totalLines": lines.len(),
                            "purpose": purpose,
                            "imports": bounded_imports,
                            "symbols": symbols,
                        }),
                        json!({
                            "files": 1,
                            "totalLines": lines.len(),
                            "imports": imports.len(),
                            "symbols": symbol_count,
                        }),
                        imports_truncated || symbols_truncated,
                        warnings,
                    ))
                }
                "glob" => {
                    let pattern = args
                        .get("pattern")
                        .and_then(Value::as_str)
                        .ok_or("pattern required")?;
                    let requested_path = args.get("path").and_then(Value::as_str).unwrap_or("");
                    let full_path = if requested_path.is_empty() {
                        project_path.to_string()
                    } else {
                        resolve_path(requested_path, project_path)
                    };
                    let output = flamel::mcp_parent_guard::command("fd")
                        .arg("--glob")
                        .arg(pattern)
                        .arg("--type")
                        .arg("f")
                        .arg("--color")
                        .arg("never")
                        .arg(&full_path)
                        .output()
                        .or_else(|_| {
                            flamel::mcp_parent_guard::command("find")
                                .arg(&full_path)
                                .arg("-type")
                                .arg("f")
                                .arg("-name")
                                .arg(pattern)
                                .output()
                        })
                        .map_err(|error| format!("glob failed: {error}"))?;
                    if !output.status.success() {
                        return Err(format!(
                            "glob failed: {}",
                            String::from_utf8_lossy(&output.stderr).trim()
                        ));
                    }
                    let stdout = String::from_utf8(output.stdout)
                        .map_err(|_| "glob returned a non-UTF-8 path".to_string())?;
                    let paths = stdout
                        .lines()
                        .map(|path| {
                            json!({
                                "path": engineering_display_path(path, project_path),
                                "kind": "file",
                            })
                        })
                        .collect::<Vec<_>>();
                    let full_text = if paths.is_empty() {
                        "No files matched.".to_string()
                    } else {
                        format!(
                            "{}\n\n[{} files]",
                            paths
                                .iter()
                                .filter_map(|entry| entry["path"].as_str())
                                .collect::<Vec<_>>()
                                .join("\n"),
                            paths.len()
                        )
                    };
                    let truncated = paths.len() > ENGINEERING_MAX_ROWS;
                    Ok(engineering_result(
                        "file_read",
                        action,
                        full_text,
                        json!({
                            "root": requested_path,
                            "pattern": pattern,
                            "paths": paths
                                .iter()
                                .take(ENGINEERING_MAX_ROWS)
                                .cloned()
                                .collect::<Vec<_>>(),
                        }),
                        json!({"paths": paths.len()}),
                        truncated,
                        Vec::new(),
                    ))
                }
                "find" => {
                    let result = mcp_fs::find_file_result(&with_resolved_path(args, project_path))?;
                    let paths = result["paths"].as_array().cloned().unwrap_or_default();
                    let full_text = if paths.is_empty() {
                        format!(
                            "No files matching '{}' in {}",
                            result["query"].as_str().unwrap_or_default(),
                            args.get("relative_path")
                                .or_else(|| args.get("path"))
                                .and_then(Value::as_str)
                                .unwrap_or("."),
                        )
                    } else {
                        format!(
                            "Found {} file(s):\n{}",
                            paths.len(),
                            paths
                                .iter()
                                .filter_map(|entry| entry["path"].as_str())
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                    };
                    let truncated = paths.len() > ENGINEERING_MAX_ROWS;
                    Ok(engineering_result(
                        "file_read",
                        action,
                        full_text,
                        json!({
                            "root": args
                                .get("relative_path")
                                .or_else(|| args.get("path"))
                                .and_then(Value::as_str)
                                .unwrap_or("."),
                            "query": result["query"],
                            "mode": result["mode"],
                            "paths": paths
                                .iter()
                                .take(ENGINEERING_MAX_ROWS)
                                .cloned()
                                .collect::<Vec<_>>(),
                        }),
                        json!({
                            "paths": paths.len(),
                            "matches": result["total"],
                        }),
                        truncated,
                        Vec::new(),
                    ))
                }
                "list_dir" => {
                    let result = mcp_fs::list_dir_result(&with_resolved_path(args, project_path))?;
                    let requested_path = args
                        .get("relative_path")
                        .or_else(|| args.get("path"))
                        .and_then(Value::as_str)
                        .unwrap_or(".");
                    let mut entries = result["entries"].as_array().cloned().unwrap_or_default();
                    for entry in &mut entries {
                        if let Some(path) = entry["path"].as_str() {
                            entry["path"] = json!(engineering_display_path(path, project_path));
                        }
                    }
                    let full_text = format!(
                        "{requested_path}/\n{}",
                        entries
                            .iter()
                            .map(|entry| {
                                let name = entry["name"].as_str().unwrap_or_default();
                                if entry["kind"] == "directory" {
                                    format!("{name}/")
                                } else {
                                    name.to_string()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                    let truncated = entries.len() > ENGINEERING_MAX_ROWS;
                    Ok(engineering_result(
                        "file_read",
                        action,
                        full_text,
                        json!({
                            "root": requested_path,
                            "paths": entries
                                .iter()
                                .take(ENGINEERING_MAX_ROWS)
                                .cloned()
                                .collect::<Vec<_>>(),
                        }),
                        json!({"paths": entries.len()}),
                        truncated,
                        Vec::new(),
                    ))
                }
                _ => Err(format!(
                    "Unknown file_read action '{action}'. Valid: read, batch, summary, glob, find, list_dir"
                )),
            }
        }

        // ── Consolidated code_quality tool ───────────────────────────────
        "code_quality" | "build_errors" | "run_tests" | "explain_error" | "find_code_issues" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or(name);
            match action {
                "build" | "build_errors" => {
                    let cwd = args
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(|c| resolve_path(c, project_path))
                        .unwrap_or_else(|| project_path.to_string());
                    let is_rust = std::path::Path::new(&format!("{cwd}/Cargo.toml")).exists()
                        || std::path::Path::new(&format!("{project_path}/Cargo.toml")).exists();
                    let is_ts = std::path::Path::new(&format!("{cwd}/tsconfig.json")).exists()
                        || std::path::Path::new(&format!("{project_path}/tsconfig.json")).exists();
                    let command = args
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            if is_rust {
                                "cargo check --message-format json 2>&1".to_string()
                            } else if is_ts {
                                "npx tsc --noEmit 2>&1".to_string()
                            } else {
                                "make check 2>&1".to_string()
                            }
                        });
                    let (tx, rx) = std::sync::mpsc::channel();
                    let cwd2 = cwd.clone();
                    let cmd2 = command.clone();
                    std::thread::spawn(move || {
                        let mut child = flamel::mcp_parent_guard::command("sh");
                        configure_shell_child(&mut child);
                        tx.send(child.arg("-c").arg(&cmd2).current_dir(&cwd2).output())
                            .ok();
                    });
                    let output = match rx.recv_timeout(std::time::Duration::from_secs(180)) {
                        Ok(Ok(o)) => o,
                        Ok(Err(e)) => return Err(format!("Failed to run: {e}")),
                        Err(_) => return Err("Build check timed out after 180s".to_string()),
                    };
                    let combined = format!(
                        "{}{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                    if output.status.success() {
                        return Ok("No errors. Build/typecheck passed.".to_string());
                    }
                    // Try parsing Rust JSON format
                    if command.contains("--message-format json") {
                        let mut errors: Vec<String> = Vec::new();
                        for line in combined.lines() {
                            if let Ok(msg) = serde_json::from_str::<Value>(line) {
                                if msg.get("reason").and_then(|r| r.as_str())
                                    == Some("compiler-message")
                                {
                                    if msg["message"].get("level").and_then(|l| l.as_str())
                                        == Some("error")
                                    {
                                        let text = msg["message"]["rendered"]
                                            .as_str()
                                            .or_else(|| msg["message"]["message"].as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        if !text.is_empty() {
                                            errors.push(text);
                                        }
                                    }
                                }
                            }
                        }
                        if !errors.is_empty() {
                            let n = errors.len();
                            return Ok(format!(
                                "{n} error{} found:\n\n{}\n→ Use explain_error with a specific error message to jump straight to the source.",
                                if n != 1 { "s" } else { "" },
                                errors
                                    .iter()
                                    .take(10)
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join("\n---\n")
                            ));
                        }
                    }
                    // Fallback: filter to error lines
                    let error_lines: Vec<&str> = combined
                        .lines()
                        .filter(|l| {
                            let t = l.trim();
                            t.contains("error") || t.starts_with("-->") || t.contains("FAILED")
                        })
                        .take(50)
                        .collect();
                    Ok(format!(
                        "Build failed. Errors:\n{}\n→ Use code_quality(action=\"explain\", error=\"...\") to jump to source.",
                        error_lines.join("\n")
                    ))
                }
                "test" | "run_tests" => {
                    let cwd = args
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(|c| resolve_path(c, project_path))
                        .unwrap_or_else(|| project_path.to_string());
                    let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("");
                    let is_rust = std::path::Path::new(&format!("{cwd}/Cargo.toml")).exists();
                    let command = args
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            if is_rust {
                                if filter.is_empty() {
                                    "cargo test 2>&1".to_string()
                                } else {
                                    format!("cargo test {filter} 2>&1")
                                }
                            } else {
                                "npm test -- --reporter=verbose 2>&1".to_string()
                            }
                        });
                    let (tx, rx) = std::sync::mpsc::channel();
                    let cwd2 = cwd.clone();
                    let cmd2 = command.clone();
                    std::thread::spawn(move || {
                        let mut child = flamel::mcp_parent_guard::command("sh");
                        configure_shell_child(&mut child);
                        tx.send(child.arg("-c").arg(&cmd2).current_dir(&cwd2).output())
                            .ok();
                    });
                    let output = match rx.recv_timeout(std::time::Duration::from_secs(300)) {
                        Ok(Ok(o)) => o,
                        Ok(Err(e)) => return Err(format!("Failed to run tests: {e}")),
                        Err(_) => return Err("Tests timed out after 300s".to_string()),
                    };
                    let combined = format!(
                        "{}{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                    let mut passed = 0usize;
                    let mut failed = 0usize;
                    let mut failures: Vec<String> = Vec::new();
                    let mut capture = false;
                    let mut current = String::new();
                    let mut current_name = String::new();
                    for line in combined.lines() {
                        if line.starts_with("test ") && line.ends_with("... ok") {
                            passed += 1;
                            capture = false;
                        } else if line.starts_with("test ") && line.ends_with("... FAILED") {
                            failed += 1;
                            if !current.is_empty() {
                                failures.push(format!(
                                    "❌ {current_name}\n{}",
                                    &current[..current.len().min(600)]
                                ));
                            }
                            current_name = line
                                .trim_start_matches("test ")
                                .trim_end_matches("... FAILED")
                                .trim()
                                .to_string();
                            current = String::new();
                            capture = true;
                        } else if capture && !line.trim().is_empty() {
                            current.push_str(line);
                            current.push('\n');
                        }
                    }
                    if !current.is_empty() && failed > 0 {
                        failures.push(format!(
                            "❌ {current_name}\n{}",
                            &current[..current.len().min(600)]
                        ));
                    }
                    if output.status.success() || failed == 0 {
                        Ok(format!("✓ All tests passed: {passed} passed, 0 failed."))
                    } else {
                        Ok(format!(
                            "{passed} passed, {failed} FAILED\n\n{}",
                            failures
                                .iter()
                                .take(5)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join("\n\n---\n\n")
                        ))
                    }
                }
                "explain" | "explain_error" => {
                    let error = args
                        .get("error")
                        .and_then(|v| v.as_str())
                        .ok_or("error required")?;
                    let ctx = args
                        .get("context_lines")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(10) as usize;
                    let mut locations: Vec<(String, usize)> = Vec::new();
                    for line in error.lines() {
                        if let Some(pos) = line.find("-->") {
                            let rest = line[pos + 3..].trim();
                            if let Some(colon) = rest.find(':') {
                                let path = &rest[..colon];
                                if let Ok(ln) = rest[colon + 1..]
                                    .split(':')
                                    .next()
                                    .unwrap_or("")
                                    .parse::<usize>()
                                {
                                    let full = resolve_path(path, project_path);
                                    if std::path::Path::new(&full).exists() {
                                        locations.push((path.to_string(), ln));
                                    }
                                }
                            }
                        }
                        if let Some(paren) = line.find('(') {
                            let before = line[..paren].trim();
                            let after = &line[paren + 1..];
                            if before.contains('.') && !before.contains(' ') {
                                if let Ok(ln) = after
                                    .split(',')
                                    .next()
                                    .unwrap_or("")
                                    .split(')')
                                    .next()
                                    .unwrap_or("")
                                    .parse::<usize>()
                                {
                                    if ln > 0 {
                                        let full = resolve_path(before, project_path);
                                        if std::path::Path::new(&full).exists() {
                                            locations.push((before.to_string(), ln));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    locations.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
                    locations.truncate(5);
                    if locations.is_empty() {
                        return Ok(format!("No file:line refs found.\n\nError:\n{error}"));
                    }
                    let mut parts = vec![format!("Error:\n{error}\n")];
                    for (file, line_no) in &locations {
                        let full = resolve_path(file, project_path);
                        match std::fs::read_to_string(&full) {
                            Err(e) => {
                                parts.push(format!("--- {file}:{line_no} (cannot read: {e}) ---"))
                            }
                            Ok(content) => {
                                let lines: Vec<&str> = content.lines().collect();
                                let total = lines.len();
                                let from = line_no.saturating_sub(1).saturating_sub(ctx).min(total);
                                let to = (line_no + ctx).min(total);
                                let mut numbered = String::new();
                                for (i, l) in lines[from..to].iter().enumerate() {
                                    let ln = from + i + 1;
                                    let marker = if ln == *line_no { ">>>" } else { "   " };
                                    numbered.push_str(&format!("{marker} {:>5}\t{l}\n", ln));
                                }
                                parts.push(format!("--- {file}:{line_no} ---\n{numbered}"));
                            }
                        }
                    }
                    Ok(parts.join("\n"))
                }
                "issues" | "find_code_issues" => {
                    let path_filter = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    let root = if path_filter.is_empty() {
                        project_path.to_string()
                    } else {
                        resolve_path(path_filter, project_path)
                    };
                    let patterns: &[(&str, &str)] = &[
                        ("TODO|FIXME|HACK|XXX", "Code debt"),
                        (r"console\.(log|warn|error|debug)", "Debug console"),
                        (r"println!|eprintln!|dbg!", "Debug prints (Rust)"),
                        (r"\.unwrap\(\)|\.expect\(", "Panic sites (Rust)"),
                    ];
                    let mut sections: Vec<String> = Vec::new();
                    for (pattern, label) in patterns {
                        let out = flamel::mcp_parent_guard::command("rg")
                            .args([
                                "--color=never",
                                "--with-filename",
                                "--line-number",
                                "--no-heading",
                                "--glob",
                                "!target/**",
                                "--glob",
                                "!node_modules/**",
                                "--glob",
                                "!.git/**",
                                pattern,
                                &root,
                            ])
                            .output()
                            .unwrap_or_else(|_| std::process::Output {
                                status: std::process::ExitStatus::default(),
                                stdout: vec![],
                                stderr: vec![],
                            });
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        let lines: Vec<String> = stdout
                            .lines()
                            .take(20)
                            .map(|l| l.replace(&format!("{project_path}/"), ""))
                            .collect();
                        if !lines.is_empty() {
                            sections.push(format!(
                                "### {} ({} found)\n{}",
                                label,
                                lines.len(),
                                lines.join("\n")
                            ));
                        }
                    }
                    if sections.is_empty() {
                        return Ok("No code issues found.".to_string());
                    }
                    Ok(format!("Code issues:\n\n{}", sections.join("\n\n")))
                }
                _ => Err(format!(
                    "Unknown code_quality action '{action}'. Use: build, test, explain, issues"
                )),
            }
        }

        // (gather_context consolidated into code_nav above)
        "get_recent_changes" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
            let path_filter = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let mut cmd = flamel::mcp_parent_guard::command("git");
            cmd.args([
                "log",
                &format!("-{limit}"),
                "--stat",
                "--no-merges",
                "--format=--- %h  %s%n    by %an (%ar)",
            ])
            .current_dir(project_path);
            if !path_filter.is_empty() {
                cmd.arg("--").arg(path_filter);
            }
            let output = cmd.output().map_err(|e| format!("git not found: {e}"))?;
            if !output.status.success() {
                return Err(format!(
                    "git log failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            let raw = String::from_utf8_lossy(&output.stdout).to_string();
            if raw.trim().is_empty() {
                return Ok("No commits found.".to_string());
            }
            let filtered: String = raw
                .lines()
                .filter(|l| !l.contains("/target/") && !l.contains("node_modules/"))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(format!(
                "Recent {limit} commit{}{}\n\n{filtered}",
                if limit != 1 { "s" } else { "" },
                if path_filter.is_empty() {
                    String::new()
                } else {
                    format!(" in '{path_filter}'")
                }
            ))
        }
        _ => Err(format!("Unknown engineering tool: {name}")),
    }
}
