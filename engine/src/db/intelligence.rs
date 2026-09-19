use crate::{db::Db, mcp_symbols};
use ignore::WalkBuilder;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSummary {
    pub project_path: String,
    pub scanned: i64,
    pub indexed: i64,
    pub unchanged: i64,
    pub removed: i64,
    pub errors: Vec<String>,
}

fn imports(content: &str) -> Vec<Value> {
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            if trimmed.starts_with("use ")
                || trimmed.starts_with("mod ")
                || trimmed.starts_with("import ")
                || trimmed.starts_with("export ")
                || trimmed.contains("require(")
            {
                Some(json!({"line":index+1,"text":trimmed.chars().take(300).collect::<String>()}))
            } else {
                None
            }
        })
        .take(300)
        .collect()
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

const EMBEDDING_DIMENSIONS: usize = 128;

fn local_embedding(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    for token in text
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .filter(|token| token.len() > 1)
        .take(20_000)
    {
        let digest = Sha256::digest(token.to_ascii_lowercase().as_bytes());
        let index = usize::from(u16::from_be_bytes([digest[0], digest[1]])) % EMBEDDING_DIMENSIONS;
        vector[index] += if digest[2] & 1 == 0 { 1.0 } else { -1.0 };
    }
    let magnitude = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        for value in &mut vector {
            *value /= magnitude;
        }
    }
    vector
}

fn similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() {
        return 0.0;
    }
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

pub fn index_project(db: &Db, project_path: &str) -> Result<IndexSummary, String> {
    let root = PathBuf::from(project_path)
        .canonicalize()
        .map_err(|e| format!("Project is unavailable: {e}"))?;
    if !root.is_dir() {
        return Err("Project path is not a directory".into());
    }
    let existing: std::collections::HashMap<String, String> = {
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT relative_path,content_hash FROM project_index_files WHERE project_path=?1",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map([project_path], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect()
    };
    let mut seen = std::collections::HashSet::new();
    let mut summary = IndexSummary {
        project_path: project_path.into(),
        scanned: 0,
        indexed: 0,
        unchanged: 0,
        removed: 0,
        errors: vec![],
    };
    for entry in WalkBuilder::new(&root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .build()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if metadata.len() > 2_000_000 {
            continue;
        }
        let relative = match path.strip_prefix(&root) {
            Ok(v) => v.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        if relative.starts_with(".git/")
            || relative.contains("/node_modules/")
            || relative.contains("/target/")
        {
            continue;
        }
        let supported = mcp_symbols::detect_language(&relative).is_some();
        if !supported
            && !matches!(
                Path::new(&relative).extension().and_then(|v| v.to_str()),
                Some(
                    "md" | "toml" | "json" | "yaml" | "yml" | "py" | "go" | "java" | "css" | "html"
                )
            )
        {
            continue;
        }
        summary.scanned += 1;
        seen.insert(relative.clone());
        let bytes = match fs::read(path) {
            Ok(v) => v,
            Err(e) => {
                summary.errors.push(format!("{relative}: {e}"));
                continue;
            }
        };
        let digest = hash(&bytes);
        if existing.get(&relative) == Some(&digest) {
            summary.unchanged += 1;
            continue;
        }
        let content = String::from_utf8_lossy(&bytes);
        let parsed = if supported {
            mcp_symbols::index_file(path).unwrap_or_else(|_| json!({"language":"","symbols":[]}))
        } else {
            json!({"language":Path::new(&relative).extension().and_then(|v|v.to_str()).unwrap_or(""),"symbols":[]})
        };
        let embedding = local_embedding(&format!("{relative}\n{content}"));
        let now = chrono::Utc::now().to_rfc3339();
        db.conn().execute("INSERT INTO project_index_files (project_path,relative_path,content_hash,language,size_bytes,symbols_json,imports_json,embedding_json,indexed_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(project_path,relative_path) DO UPDATE SET content_hash=excluded.content_hash,language=excluded.language,size_bytes=excluded.size_bytes,symbols_json=excluded.symbols_json,imports_json=excluded.imports_json,embedding_json=excluded.embedding_json,indexed_at=excluded.indexed_at",
            rusqlite::params![project_path,relative,digest,parsed["language"].as_str().unwrap_or(""),metadata.len() as i64,parsed["symbols"].to_string(),json!(imports(&content)).to_string(),json!(embedding).to_string(),now]).map_err(|e|e.to_string())?;
        summary.indexed += 1;
    }
    for relative in existing.keys().filter(|path| !seen.contains(*path)) {
        summary.removed += db
            .conn()
            .execute(
                "DELETE FROM project_index_files WHERE project_path=?1 AND relative_path=?2",
                rusqlite::params![project_path, relative],
            )
            .unwrap_or(0) as i64;
    }
    Ok(summary)
}

fn tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|v| v.len() > 1)
        .map(str::to_ascii_lowercase)
        .collect()
}

pub fn context_bundle(db: &Db, project_path: &str, query: &str, max_tokens: usize) -> Value {
    let wanted = tokens(query);
    let query_embedding = local_embedding(query);
    let conn = db.conn();
    let mut stmt=match conn.prepare("SELECT relative_path,language,symbols_json,imports_json,embedding_json,indexed_at FROM project_index_files WHERE project_path=?1"){Ok(v)=>v,Err(_)=>return json!({"query":query,"sources":[]})};
    let mut scored = stmt
        .query_map([project_path], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map(|rows| {
            rows.filter_map(Result::ok)
                .filter_map(
                    |(path, language, symbols, imports, embedding, indexed_at)| {
                        let haystack = format!("{path} {symbols} {imports}").to_ascii_lowercase();
                        let lexical_score = wanted
                            .iter()
                            .map(|token| {
                                if path.to_ascii_lowercase().contains(token) {
                                    20
                                } else if haystack.contains(token) {
                                    6
                                } else {
                                    0
                                }
                            })
                            .sum::<i64>();
                        let file_embedding =
                            serde_json::from_str::<Vec<f32>>(&embedding).unwrap_or_default();
                        let semantic_score =
                            (similarity(&query_embedding, &file_embedding).max(0.0) * 12.0).round()
                                as i64;
                        let score = lexical_score + semantic_score;
                        (score > 0).then_some((
                            score,
                            lexical_score,
                            semantic_score,
                            path,
                            language,
                            symbols,
                            imports,
                            indexed_at,
                        ))
                    },
                )
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut budget = max_tokens.max(200);
    let mut sources = vec![];
    for (score, lexical_score, semantic_score, path, language, symbols, imports, indexed_at) in
        scored
    {
        let symbols: Value = serde_json::from_str(&symbols).unwrap_or_else(|_| json!([]));
        let imports: Value = serde_json::from_str(&imports).unwrap_or_else(|_| json!([]));
        let estimate = (path.len() + symbols.to_string().len() + imports.to_string().len()) / 4;
        if estimate > budget && !sources.is_empty() {
            continue;
        }
        budget = budget.saturating_sub(estimate);
        sources.push(json!({"path":path,"language":language,"score":score,"lexicalScore":lexical_score,"semanticScore":semantic_score,"reason":"local semantic similarity plus path, symbol, and dependency evidence","symbols":symbols,"dependencies":imports,"freshness":indexed_at}));
        if budget == 0 {
            break;
        }
    }
    json!({"query":query,"projectPath":project_path,"tokenBudget":max_tokens,"estimatedTokensUsed":max_tokens.saturating_sub(budget),"sources":sources})
}

pub fn verification_graph(project_path: &str) -> Value {
    let root = Path::new(project_path);
    let mut checks = vec![];
    if root.join("Cargo.toml").exists() {
        checks.extend([json!({"id":"rust-format","command":"cargo fmt --check","program":"cargo","args":["fmt","--check"],"kind":"format","dependsOn":[]}),json!({"id":"rust-check","command":"cargo check","program":"cargo","args":["check"],"kind":"typecheck","dependsOn":["rust-format"]}),json!({"id":"rust-test","command":"cargo test","program":"cargo","args":["test"],"kind":"test","dependsOn":["rust-check"]})]);
    }
    if let Ok(raw) = fs::read_to_string(root.join("package.json")) {
        if let Ok(package) = serde_json::from_str::<Value>(&raw) {
            if let Some(scripts) = package.get("scripts").and_then(Value::as_object) {
                for (name, _) in scripts {
                    let kind = if name.contains("test") {
                        "test"
                    } else if name.contains("lint") {
                        "lint"
                    } else if name.contains("type") || name == "check" {
                        "typecheck"
                    } else if name.contains("format") {
                        "format"
                    } else if name.contains("build") {
                        "build"
                    } else {
                        continue;
                    };
                    checks.push(json!({"id":format!("npm-{name}"),"command":format!("npm run {name}"),"program":"npm","args":["run",name],"kind":kind,"dependsOn":[]}));
                }
            }
        }
    }
    if root.join("pyproject.toml").exists() || root.join("pytest.ini").exists() {
        checks.push(
            json!({"id":"python-test","command":"python -m pytest","program":"python","args":["-m","pytest"],"kind":"test","dependsOn":[]}),
        );
    }
    if root.join("go.mod").exists() {
        checks.extend([
            json!({"id":"go-vet","command":"go vet ./...","program":"go","args":["vet","./..."],"kind":"lint","dependsOn":[]}),
            json!({"id":"go-test","command":"go test ./...","program":"go","args":["test","./..."],"kind":"test","dependsOn":["go-vet"]}),
        ]);
    }
    json!({"projectPath":project_path,"checks":checks})
}

fn verification_command(project_path: &str, check: &Value) -> Value {
    let command = check.get("command").and_then(Value::as_str).unwrap_or("");
    let program = check.get("program").and_then(Value::as_str).unwrap_or("");
    let args = check
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let started = Instant::now();
    let output = Command::new(program)
        .args(args)
        .current_dir(project_path)
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            json!({
                "id": check.get("id"), "kind": check.get("kind"), "command": command,
                "status": if output.status.success() { "passed" } else { "failed" },
                "exitCode": output.status.code(),
                "durationMs": started.elapsed().as_millis() as u64,
                "stdout": stdout.chars().take(40_000).collect::<String>(),
                "stderr": stderr.chars().take(40_000).collect::<String>(),
            })
        }
        Err(error) => json!({
            "id": check.get("id"), "kind": check.get("kind"), "command": command,
            "status": "error", "durationMs": started.elapsed().as_millis() as u64,
            "error": error.to_string(),
        }),
    }
}

pub fn run_verification(project_path: &str, selected: &[String]) -> Result<Value, String> {
    let root = PathBuf::from(project_path)
        .canonicalize()
        .map_err(|error| format!("Project is unavailable: {error}"))?;
    if !root.is_dir() {
        return Err("Project path is not a directory".into());
    }
    let graph = verification_graph(project_path);
    let all = graph
        .get("checks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut pending = all
        .into_iter()
        .filter(|check| {
            selected.is_empty()
                || check
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|id| selected.iter().any(|selected| selected == id))
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let requested = pending.len();
    let started = Instant::now();
    let mut results = Vec::<Value>::new();
    while !pending.is_empty() {
        let completed = results
            .iter()
            .filter_map(|result| result.get("id").and_then(Value::as_str))
            .collect::<std::collections::HashSet<_>>();
        let mut ready = Vec::new();
        let mut blocked = Vec::new();
        let current = std::mem::take(&mut pending);
        for check in current {
            let deps = check
                .get("dependsOn")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let dependency_failed = deps.iter().filter_map(Value::as_str).any(|dependency| {
                results.iter().any(|result| {
                    result.get("id").and_then(Value::as_str) == Some(dependency)
                        && result.get("status").and_then(Value::as_str) != Some("passed")
                })
            });
            if dependency_failed {
                blocked.push(json!({"id":check.get("id"),"kind":check.get("kind"),"command":check.get("command"),"status":"skipped","reason":"dependency failed"}));
            } else if deps
                .iter()
                .filter_map(Value::as_str)
                .all(|dependency| completed.contains(dependency))
            {
                ready.push(check);
            } else {
                pending.push(check);
            }
        }
        results.extend(blocked);
        if ready.is_empty() {
            for check in pending.drain(..) {
                results.push(json!({"id":check.get("id"),"kind":check.get("kind"),"command":check.get("command"),"status":"skipped","reason":"dependency cycle or unselected dependency"}));
            }
            break;
        }
        let path = root.to_string_lossy().to_string();
        let mut wave = std::thread::scope(|scope| {
            ready
                .into_iter()
                .map(|check| {
                    let path = path.clone();
                    scope.spawn(move || verification_command(&path, &check))
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| {
                    handle.join().unwrap_or_else(
                        |_| json!({"status":"error","error":"verification worker panicked"}),
                    )
                })
                .collect::<Vec<_>>()
        });
        results.append(&mut wave);
    }
    let passed = results
        .iter()
        .filter(|result| result.get("status") == Some(&json!("passed")))
        .count();
    let failed = results
        .iter()
        .filter(|result| {
            matches!(
                result.get("status").and_then(Value::as_str),
                Some("failed" | "error")
            )
        })
        .count();
    Ok(
        json!({"projectPath":project_path,"requested":requested,"passed":passed,"failed":failed,
        "durationMs":started.elapsed().as_millis() as u64,"status":if failed==0{"passed"}else{"failed"},"results":results}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verification_detects_this_repo() {
        let graph = verification_graph(env!("CARGO_MANIFEST_DIR"));
        assert!(
            graph["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["id"] == "rust-test")
        );
    }

    #[test]
    fn local_embeddings_are_deterministic_and_normalized() {
        let first = local_embedding("OAuth token refresh and authentication");
        let second = local_embedding("OAuth token refresh and authentication");
        assert_eq!(first, second);
        assert!((similarity(&first, &second) - 1.0).abs() < 0.0001);
        assert_eq!(first.len(), EMBEDDING_DIMENSIONS);
    }
}
