use serde_json::Value;
use std::path::Path;

// ── Fuzzy file search ─────────────────────────────────────────────────────────

/// Nucleo-style subsequence scoring. Returns None if query is not a subsequence of path.
fn fuzzy_score(path: &str, query: &str) -> Option<i32> {
    let lpath = path.to_lowercase();
    let lquery = query.to_lowercase();
    let pb = lpath.as_bytes();
    let qb = lquery.as_bytes();
    let mut score = 0i32;
    let mut pi = 0usize;
    let mut last_match: Option<usize> = None;

    for &qc in qb {
        let pos = pb[pi..].iter().position(|&b| b == qc)?;
        let abs = pi + pos;
        score += 16; // SCORE_MATCH
        if abs == 0 {
            score += 8;
        } // BONUS_FIRST_CHAR
        if let Some(lm) = last_match {
            if abs == lm + 1 {
                score += 4; // BONUS_CONSECUTIVE
            } else {
                score -= 3 + (abs - lm - 1).min(10) as i32; // PENALTY_GAP
            }
        }
        // BONUS_BOUNDARY: preceded by path separator or common delimiters
        if abs > 0 && matches!(pb[abs - 1], b'/' | b'_' | b'-' | b'.') {
            score += 8;
        }
        last_match = Some(abs);
        pi = abs + 1;
    }

    // Test file penalty
    if path.contains("/test") || path.contains(".test.") || path.contains(".spec.") {
        score = (score as f64 * 0.95) as i32;
    }
    Some(score)
}

/// Collect all files under `dir`, skipping common noise dirs.
fn collect_files(dir: &str) -> Vec<String> {
    let skip = [
        "node_modules",
        ".git",
        "target",
        "dist",
        ".next",
        "__pycache__",
        ".cache",
    ];
    let mut out = Vec::new();
    let Ok(walker) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut stack = vec![dir.to_string()];
    while let Some(current) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && name != ".env" {
                continue;
            }
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_dir() {
                if !skip.contains(&name.as_str()) {
                    stack.push(entry.path().display().to_string());
                }
            } else if ft.is_file() {
                out.push(entry.path().display().to_string());
            }
        }
    }
    let _ = walker; // suppress unused warning
    out
}

pub fn read_file(args: &Value) -> Result<String, String> {
    let path = args
        .get("relative_path")
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
        .ok_or("path required")?;
    let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let start = args
        .get("start_line")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).saturating_sub(1))
        .unwrap_or(0);
    let end = args
        .get("end_line")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(total))
        .unwrap_or(total);

    let selected: String = lines[start..end.min(total)]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>4}\t{}\n", start + i + 1, line))
        .collect();

    Ok(format!(
        "File: {path} (lines {}-{} of {})\n{}",
        start + 1,
        end,
        total,
        selected
    ))
}

pub fn find_file_result(args: &Value) -> Result<Value, String> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or("pattern required")?;
    let dir = args
        .get("relative_path")
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
    let prefix = format!("{}/", dir.trim_end_matches('/'));
    let is_glob = pattern.contains('*') || pattern.contains('?') || pattern.contains('[');

    let (mode, total, paths) = if is_glob {
        let full_pattern = if pattern.contains('/') {
            format!("{dir}/{pattern}")
        } else {
            format!("{dir}/**/{pattern}")
        };
        let mut matches: Vec<String> = glob::glob(&full_pattern)
            .map_err(|e| format!("invalid glob pattern: {e}"))?
            .filter_map(|entry| entry.ok())
            .filter(|path| path.is_file())
            .map(|path| {
                let displayed = path.display().to_string();
                displayed
                    .strip_prefix(&prefix)
                    .unwrap_or(&displayed)
                    .to_string()
            })
            .collect();
        matches.sort();
        let total = matches.len();
        let paths = matches
            .into_iter()
            .map(|path| serde_json::json!({"path": path, "kind": "file"}))
            .collect::<Vec<_>>();
        ("glob", total, paths)
    } else {
        let files = collect_files(dir);
        let mut scored: Vec<(i32, String)> = files
            .iter()
            .filter_map(|file| {
                let relative = file.strip_prefix(&prefix).unwrap_or(file.as_str());
                let filename = relative.split('/').last().unwrap_or(relative);
                let score = fuzzy_score(filename, pattern)
                    .map(|score| score + 20)
                    .or_else(|| fuzzy_score(relative, pattern))?;
                Some((score, relative.to_string()))
            })
            .collect();
        scored.sort_by(|left, right| right.0.cmp(&left.0));
        let total = scored.len();
        scored.truncate(limit);
        let paths = scored
            .into_iter()
            .map(|(score, path)| serde_json::json!({"path": path, "kind": "file", "score": score}))
            .collect::<Vec<_>>();
        ("fuzzy", total, paths)
    };

    Ok(serde_json::json!({
        "root": dir,
        "query": pattern,
        "mode": mode,
        "total": total,
        "paths": paths,
    }))
}

pub fn find_file(args: &Value) -> Result<String, String> {
    let result = find_file_result(args)?;
    let pattern = result["query"].as_str().unwrap_or_default();
    let dir = result["root"].as_str().unwrap_or(".");
    let paths = result["paths"].as_array().cloned().unwrap_or_default();
    if paths.is_empty() {
        Ok(format!("No files matching '{pattern}' in {dir}"))
    } else {
        let lines = paths
            .iter()
            .filter_map(|entry| entry["path"].as_str())
            .collect::<Vec<_>>();
        Ok(format!(
            "Found {} file(s):\n{}",
            lines.len(),
            lines.join("\n")
        ))
    }
}

pub fn list_dir_result(args: &Value) -> Result<Value, String> {
    let path = args
        .get("relative_path")
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    let mut entries = std::fs::read_dir(path)
        .map_err(|e| format!("read_dir error: {e}"))?
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let kind = if entry.path().is_dir() {
                "directory"
            } else {
                "file"
            };
            serde_json::json!({
                "name": name,
                "path": entry.path().display().to_string(),
                "kind": kind,
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["name"].as_str().unwrap_or_default())
    });
    Ok(serde_json::json!({"root": path, "entries": entries}))
}

pub fn list_dir(args: &Value) -> Result<String, String> {
    let result = list_dir_result(args)?;
    let path = result["root"].as_str().unwrap_or(".");
    let entries = result["entries"].as_array().cloned().unwrap_or_default();
    let lines = entries
        .iter()
        .map(|entry| {
            let name = entry["name"].as_str().unwrap_or_default();
            if entry["kind"] == "directory" {
                format!("{name}/")
            } else {
                name.to_string()
            }
        })
        .collect::<Vec<_>>();
    Ok(format!("{path}/\n{}", lines.join("\n")))
}

pub fn create_text_file(args: &Value) -> Result<String, String> {
    let path = args
        .get("relative_path")
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
        .ok_or("path required")?;
    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");

    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir error: {e}"))?;
        }
    }
    std::fs::write(path, content).map_err(|e| format!("write error: {e}"))?;
    Ok(format!("Created {path} ({} bytes)", content.len()))
}
