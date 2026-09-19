//! flamel Docs MCP — stdio JSON-RPC server for fetching library documentation.
//! Fetches docs from public registries (no API keys), caches in local SQLite.
//! Supports: Rust, JS/TS, Python, Go, Ruby, PHP, Java/Kotlin, Swift, C#/.NET, Elixir, Dart/Flutter.

use flamel::mcp_args::normalize_tool_arguments;
use reqwest::blocking::Client;
use rusqlite::Connection;
use scraper::{Html, Selector};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// Cache TTL: 24 hours
const CACHE_TTL_SECS: u64 = 86400;
const MAX_DOC_SIZE: usize = 100_000; // 100KB max per doc fetch

fn main() {
    flamel::mcp_parent_guard::install();
    let db = open_cache_db();
    let client = Client::builder()
        .user_agent("flamel-docs/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("failed to build HTTP client");

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

        let response = handle_request(&req, &db, &client);
        if let Some(resp) = response {
            let mut out = stdout.lock();
            let _ = serde_json::to_writer(&mut out, &resp);
            let _ = out.write_all(b"\n");
            let _ = out.flush();
        }
    }
}

// ---------------------------------------------------------------------------
// MCP protocol handler
// ---------------------------------------------------------------------------

fn handle_request(req: &Value, db: &Connection, client: &Client) -> Option<Value> {
    let id = req.get("id");
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
                "serverInfo": {"name": "docs_mcp", "version": "0.1.0"},
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

        match handle_tool_call(name, &args, db, client) {
            Ok(text) => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"content": [{"type": "text", "text": text}], "isError": false},
            })),
            Err(e) => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"content": [{"type": "text", "text": format!("Error: {e}")}], "isError": true},
            })),
        }
    } else {
        id.map(|id| {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": "Method not found"},
            })
        })
    }
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn tools_list() -> Value {
    json!([
        {
            "name": "resolve_library",
            "description": "Look up a library/package by name and optional language. Returns metadata: name, version, description, and available documentation URL. Use this first to confirm the library exists before calling get_docs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Library/package name (e.g. 'tokio', 'react', 'flask')"},
                    "language": {"type": "string", "description": "Language hint: rust, javascript, typescript, python, go, ruby, php, java, kotlin, swift, csharp, elixir, dart. Auto-detected if omitted."}
                },
                "required": ["name"]
            }
        },
        {
            "name": "get_docs",
            "description": "Fetch documentation for a library. Returns the README, API docs, or relevant doc sections. Results are cached locally for 24h.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "library": {"type": "string", "description": "Library name (e.g. 'tokio', 'react', 'flask')"},
                    "language": {"type": "string", "description": "Language: rust, javascript, typescript, python, go, ruby, php, java, kotlin, swift, csharp, elixir, dart"},
                    "topic": {"type": "string", "description": "Optional: specific topic to search for within the docs (e.g. 'async runtime', 'hooks', 'routing')"},
                    "version": {"type": "string", "description": "Optional: specific version (e.g. '1.0', 'latest'). Defaults to latest."}
                },
                "required": ["library"]
            }
        },
        {
            "name": "search_flamel_docs",
            "description": "Search the canonical Flamel product documentation for Desktop, Server, Git, Actions, runners, MCP, configuration, and troubleshooting guidance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Plain-language Flamel product or operations question"},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 10, "default": 5}
                },
                "required": ["query"]
            }
        },
        {
            "name": "get_flamel_doc",
            "description": "Read one canonical Flamel documentation page by the path returned from search_flamel_docs, such as server/install or actions/secrets.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Canonical documentation path without a leading slash"}
                },
                "required": ["path"]
            }
        }
    ])
}

// ---------------------------------------------------------------------------
// Tool dispatch
// ---------------------------------------------------------------------------

fn handle_tool_call(
    name: &str,
    args: &Value,
    db: &Connection,
    client: &Client,
) -> Result<String, String> {
    match name {
        "resolve_library" => {
            let lib_name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("name required")?;
            let lang = args.get("language").and_then(|v| v.as_str());
            resolve_library(lib_name, lang, client)
        }
        "get_docs" => {
            let library = args
                .get("library")
                .and_then(|v| v.as_str())
                .ok_or("library required")?;
            let language = args.get("language").and_then(|v| v.as_str());
            let topic = args.get("topic").and_then(|v| v.as_str());
            let version = args.get("version").and_then(|v| v.as_str());
            get_docs(library, language, topic, version, db, client)
        }
        "search_flamel_docs" => {
            let query = args
                .get("query")
                .and_then(|value| value.as_str())
                .ok_or("query required")?;
            let max_results = args
                .get("max_results")
                .and_then(|value| value.as_u64())
                .unwrap_or(5)
                .clamp(1, 10) as usize;
            search_flamel_docs(query, max_results, client)
        }
        "get_flamel_doc" => {
            let path = args
                .get("path")
                .and_then(|value| value.as_str())
                .ok_or("path required")?;
            get_flamel_doc(path, client)
        }
        _ => Err(format!("Unknown tool: {name}")),
    }
}

// ---------------------------------------------------------------------------
// Language detection
// ---------------------------------------------------------------------------

fn flamel_docs_base() -> String {
    std::env::var("FLAMEL_DOCS_URL")
        .unwrap_or_else(|_| "https://docs.flamel.dev".to_string())
        .trim_end_matches('/')
        .to_string()
}

fn search_flamel_docs(query: &str, max_results: usize, client: &Client) -> Result<String, String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Err("query must not be empty".to_string());
    }
    let url = format!("{}/api/v1/search-index.json", flamel_docs_base());
    let response: Value = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .map_err(|error| format!("Flamel Docs request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Flamel Docs returned an error: {error}"))?
        .json()
        .map_err(|error| format!("Invalid Flamel Docs response: {error}"))?;
    let documents = response
        .get("documents")
        .and_then(Value::as_array)
        .ok_or("Flamel Docs search index is missing documents")?;

    let mut matches: Vec<(usize, &Value)> = documents
        .iter()
        .filter_map(|document| {
            score_flamel_document(&query, document).map(|score| (score, document))
        })
        .collect();
    matches.sort_by(|left, right| right.0.cmp(&left.0));

    let results: Vec<Value> = matches
        .into_iter()
        .take(max_results)
        .map(|(_, document)| {
            json!({
                "path": document.get("slug").and_then(Value::as_str).unwrap_or(""),
                "title": document.get("title").and_then(Value::as_str).unwrap_or(""),
                "description": document.get("description").and_then(Value::as_str).unwrap_or(""),
                "section": document.get("section").and_then(Value::as_str).unwrap_or(""),
                "url": format!(
                    "{}/{}",
                    flamel_docs_base(),
                    document.get("slug").and_then(Value::as_str).unwrap_or("")
                )
            })
        })
        .collect();

    Ok(json!({ "query": query, "results": results }).to_string())
}

fn search_tokens(text: &str) -> HashSet<String> {
    text.split(|character: char| !character.is_ascii_alphanumeric())
        .filter_map(canonical_search_token)
        .collect()
}

fn canonical_search_token(token: &str) -> Option<String> {
    let token = token.trim().to_ascii_lowercase();
    if token.len() < 2
        || matches!(
            token.as_str(),
            "a" | "an"
                | "and"
                | "are"
                | "can"
                | "do"
                | "does"
                | "for"
                | "from"
                | "how"
                | "i"
                | "in"
                | "is"
                | "it"
                | "of"
                | "on"
                | "or"
                | "our"
                | "the"
                | "their"
                | "this"
                | "to"
                | "we"
                | "what"
                | "when"
                | "where"
                | "which"
                | "with"
                | "without"
                | "you"
                | "your"
                | "flamel"
        )
    {
        return None;
    }

    let canonical = match token.as_str() {
        "actions" => "action",
        "artifacts" => "artifact",
        "builds" | "building" | "built" => "build",
        "credentials" => "credential",
        "deployed" | "deploying" | "deployment" | "deployments" => "deploy",
        "environments" => "environment",
        "executors" => "executor",
        "hosted" | "hosting" => "host",
        "packages" => "package",
        "published" | "publishing" => "publish",
        "releases" => "release",
        "repositories" | "repository" | "repos" => "repo",
        "runners" => "runner",
        "secrets" => "secret",
        "uploaded" | "uploading" | "uploads" => "upload",
        "workflows" => "workflow",
        other => other,
    };
    Some(canonical.to_string())
}

fn query_term_groups(query: &str) -> Vec<HashSet<String>> {
    search_tokens(query)
        .into_iter()
        .map(|term| {
            let mut group = HashSet::from([term.clone()]);
            let related: &[&str] = match term.as_str() {
                "action" => &["workflow"],
                "artifact" => &["build", "package", "upload"],
                "build" => &["artifact", "package"],
                "credential" => &["secret", "token"],
                "deploy" => &["environment", "publish", "release"],
                "git" => &["repo"],
                "host" => &["standalone", "embedded", "runner"],
                "publish" => &["release", "artifact", "upload"],
                "release" => &["publish", "artifact"],
                "repo" => &["git"],
                "runner" => &["executor", "standalone", "embedded", "host"],
                "secret" => &["credential", "token"],
                "ssh" => &["deploy", "secret", "credential"],
                "upload" => &["artifact", "publish"],
                "workflow" => &["action"],
                _ => &[],
            };
            group.extend(related.iter().map(|value| (*value).to_string()));
            group
        })
        .collect()
}

fn score_flamel_document(query: &str, document: &Value) -> Option<usize> {
    let title = document.get("title").and_then(Value::as_str).unwrap_or("");
    let description = document
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let section = document
        .get("section")
        .and_then(Value::as_str)
        .unwrap_or("");
    let text = document.get("text").and_then(Value::as_str).unwrap_or("");

    let groups = query_term_groups(query);
    if groups.is_empty() {
        return None;
    }
    let title_tokens = search_tokens(title);
    let description_tokens = search_tokens(description);
    let section_tokens = search_tokens(section);
    let text_tokens = search_tokens(text);
    let mut coverage = 0usize;
    let mut score = 0usize;

    for group in &groups {
        let title_match = !title_tokens.is_disjoint(group);
        let description_match = !description_tokens.is_disjoint(group);
        let section_match = !section_tokens.is_disjoint(group);
        let text_match = !text_tokens.is_disjoint(group);
        if title_match || description_match || section_match || text_match {
            coverage += 1;
            score += usize::from(title_match) * 18
                + usize::from(description_match) * 9
                + usize::from(section_match) * 7
                + usize::from(text_match) * 2;
        }
    }

    if coverage == 0 {
        return None;
    }
    score += coverage * 12;
    if coverage * 2 >= groups.len() {
        score += 20;
    }
    Some(score)
}

fn get_flamel_doc(path: &str, client: &Client) -> Result<String, String> {
    let path = path.trim().trim_matches('/');
    if path.is_empty()
        || path.contains("..")
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_'))
    {
        return Err("path must be a canonical Flamel documentation path".to_string());
    }
    let url = format!("{}/api/v1/raw/current/{path}", flamel_docs_base());
    let text = client
        .get(&url)
        .header("Accept", "text/markdown")
        .send()
        .map_err(|error| format!("Flamel Docs request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Flamel document was not found: {error}"))?
        .text()
        .map_err(|error| format!("Could not read Flamel document: {error}"))?;
    Ok(truncate_docs(&text))
}

fn detect_language(name: &str, client: &Client) -> &'static str {
    // Try each registry in popularity order
    let checks: &[(&str, &str)] = &[
        ("javascript", &format!("https://registry.npmjs.org/{name}")),
        ("python", &format!("https://pypi.org/pypi/{name}/json")),
        ("rust", &format!("https://crates.io/api/v1/crates/{name}")),
        (
            "ruby",
            &format!("https://rubygems.org/api/v1/gems/{name}.json"),
        ),
        ("php", &format!("https://repo.packagist.org/p2/{name}.json")),
        ("dart", &format!("https://pub.dev/api/packages/{name}")),
        ("elixir", &format!("https://hex.pm/api/packages/{name}")),
    ];

    for (lang, url) in checks {
        if let Ok(resp) = client.head(*url).send() {
            if resp.status().is_success() {
                return lang;
            }
        }
    }
    "javascript" // fallback
}

// ---------------------------------------------------------------------------
// resolve_library
// ---------------------------------------------------------------------------

fn resolve_library(name: &str, lang: Option<&str>, client: &Client) -> Result<String, String> {
    let language = lang.unwrap_or_else(|| detect_language(name, client));

    match language {
        "rust" => resolve_crates_io(name, client),
        "javascript" | "typescript" | "js" | "ts" => resolve_npm(name, client),
        "python" | "py" => resolve_pypi(name, client),
        "go" | "golang" => resolve_go(name, client),
        "ruby" | "rb" => resolve_rubygems(name, client),
        "php" => resolve_packagist(name, client),
        "java" | "kotlin" | "kt" => resolve_maven(name, client),
        "swift" => resolve_swift(name, client),
        "csharp" | "cs" | "dotnet" => resolve_nuget(name, client),
        "elixir" | "ex" => resolve_hex(name, client),
        "dart" | "flutter" => resolve_pub_dev(name, client),
        _ => Err(format!(
            "Unsupported language: {language}. Supported: rust, javascript, python, go, ruby, php, java, kotlin, swift, csharp, elixir, dart"
        )),
    }
}

// ---------------------------------------------------------------------------
// Registry resolvers
// ---------------------------------------------------------------------------

fn resolve_crates_io(name: &str, client: &Client) -> Result<String, String> {
    let url = format!("https://crates.io/api/v1/crates/{name}");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    let c = resp.get("crate").ok_or("crate not found on crates.io")?;
    Ok(json!({
        "name": name,
        "language": "rust",
        "version": c.get("max_version").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "description": c.get("description").and_then(|v| v.as_str()).unwrap_or(""),
        "docs_url": format!("https://docs.rs/{name}"),
        "repository": c.get("repository").and_then(|v| v.as_str()).unwrap_or(""),
    })
    .to_string())
}

fn resolve_npm(name: &str, client: &Client) -> Result<String, String> {
    let url = format!("https://registry.npmjs.org/{name}/latest");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    if resp.get("error").is_some() {
        return Err(format!("Package '{name}' not found on npm"));
    }
    Ok(json!({
        "name": name,
        "language": "javascript",
        "version": resp.get("version").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "description": resp.get("description").and_then(|v| v.as_str()).unwrap_or(""),
        "docs_url": format!("https://www.npmjs.com/package/{name}"),
        "homepage": resp.get("homepage").and_then(|v| v.as_str()).unwrap_or(""),
    })
    .to_string())
}

fn resolve_pypi(name: &str, client: &Client) -> Result<String, String> {
    let url = format!("https://pypi.org/pypi/{name}/json");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    let info = resp
        .get("info")
        .ok_or(format!("Package '{name}' not found on PyPI"))?;
    Ok(json!({
        "name": name,
        "language": "python",
        "version": info.get("version").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "description": info.get("summary").and_then(|v| v.as_str()).unwrap_or(""),
        "docs_url": info.get("project_urls").and_then(|u| u.get("Documentation")).and_then(|v| v.as_str())
            .unwrap_or(&format!("https://pypi.org/project/{name}/")),
        "homepage": info.get("home_page").and_then(|v| v.as_str()).unwrap_or(""),
    }).to_string())
}

fn resolve_go(name: &str, client: &Client) -> Result<String, String> {
    // Go module proxy
    let url = format!("https://proxy.golang.org/{}/@latest", name);
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "name": name,
        "language": "go",
        "version": resp.get("Version").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "docs_url": format!("https://pkg.go.dev/{name}"),
    })
    .to_string())
}

fn resolve_rubygems(name: &str, client: &Client) -> Result<String, String> {
    let url = format!("https://rubygems.org/api/v1/gems/{name}.json");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "name": name,
        "language": "ruby",
        "version": resp.get("version").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "description": resp.get("info").and_then(|v| v.as_str()).unwrap_or(""),
        "docs_url": resp.get("documentation_uri").and_then(|v| v.as_str())
            .unwrap_or(&format!("https://rubydoc.info/gems/{name}")),
    })
    .to_string())
}

fn resolve_packagist(name: &str, client: &Client) -> Result<String, String> {
    // Packagist requires vendor/package format; try as-is first
    let url = format!("https://repo.packagist.org/p2/{name}.json");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    let packages = resp
        .get("packages")
        .and_then(|p| p.get(name))
        .and_then(|v| v.as_array())
        .ok_or(format!(
            "Package '{name}' not found on Packagist (use vendor/package format)"
        ))?;
    let latest = packages.first().ok_or("no versions")?;
    Ok(json!({
        "name": name,
        "language": "php",
        "version": latest.get("version").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "description": latest.get("description").and_then(|v| v.as_str()).unwrap_or(""),
        "docs_url": format!("https://packagist.org/packages/{name}"),
    })
    .to_string())
}

fn resolve_maven(name: &str, client: &Client) -> Result<String, String> {
    // Search Maven Central
    let url = format!("https://search.maven.org/solrsearch/select?q={name}&rows=1&wt=json");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    let doc = resp
        .pointer("/response/docs/0")
        .ok_or(format!("Package '{name}' not found on Maven Central"))?;
    let group = doc.get("g").and_then(|v| v.as_str()).unwrap_or("");
    let artifact = doc.get("a").and_then(|v| v.as_str()).unwrap_or(name);
    Ok(json!({
        "name": format!("{group}:{artifact}"),
        "language": "java",
        "version": doc.get("latestVersion").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "docs_url": format!("https://javadoc.io/doc/{group}/{artifact}"),
    })
    .to_string())
}

fn resolve_swift(name: &str, client: &Client) -> Result<String, String> {
    let url = format!("https://swiftpackageindex.com/api/search?query={name}");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    let results = resp
        .get("results")
        .and_then(|v| v.as_array())
        .ok_or(format!("No results for '{name}' on Swift Package Index"))?;
    let first = results.first().ok_or("no results")?;
    let package = first.get("package").unwrap_or(first);
    Ok(json!({
        "name": name,
        "language": "swift",
        "url": package.get("url").and_then(|v| v.as_str()).unwrap_or(""),
        "docs_url": format!("https://swiftpackageindex.com/search?query={name}"),
    })
    .to_string())
}

fn resolve_nuget(name: &str, client: &Client) -> Result<String, String> {
    let url = format!(
        "https://api.nuget.org/v3/registration5-gz-semver2/{}/index.json",
        name.to_lowercase()
    );
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    let items = resp
        .get("items")
        .and_then(|v| v.as_array())
        .ok_or(format!("Package '{name}' not found on NuGet"))?;
    let latest_page = items.last().ok_or("no versions")?;
    // Get version from the page's upper bound
    let upper = latest_page
        .get("upper")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    Ok(json!({
        "name": name,
        "language": "csharp",
        "version": upper,
        "docs_url": format!("https://www.nuget.org/packages/{name}"),
    })
    .to_string())
}

fn resolve_hex(name: &str, client: &Client) -> Result<String, String> {
    let url = format!("https://hex.pm/api/packages/{name}");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    let releases = resp.get("releases").and_then(|v| v.as_array());
    let version = releases
        .and_then(|r| r.first())
        .and_then(|r| r.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    Ok(json!({
        "name": name,
        "language": "elixir",
        "version": version,
        "description": resp.get("meta").and_then(|m| m.get("description")).and_then(|v| v.as_str()).unwrap_or(""),
        "docs_url": format!("https://hexdocs.pm/{name}"),
    }).to_string())
}

fn resolve_pub_dev(name: &str, client: &Client) -> Result<String, String> {
    let url = format!("https://pub.dev/api/packages/{name}");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    let latest = resp
        .get("latest")
        .ok_or(format!("Package '{name}' not found on pub.dev"))?;
    let pubspec = latest.get("pubspec").unwrap_or(latest);
    Ok(json!({
        "name": name,
        "language": "dart",
        "version": pubspec.get("version").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "description": pubspec.get("description").and_then(|v| v.as_str()).unwrap_or(""),
        "docs_url": format!("https://pub.dev/documentation/{name}/latest/"),
    })
    .to_string())
}

// ---------------------------------------------------------------------------
// get_docs — fetch and cache documentation
// ---------------------------------------------------------------------------

fn get_docs(
    library: &str,
    language: Option<&str>,
    topic: Option<&str>,
    version: Option<&str>,
    db: &Connection,
    client: &Client,
) -> Result<String, String> {
    let lang = language.unwrap_or_else(|| detect_language(library, client));
    let ver = version.unwrap_or("latest");

    // Check cache
    let cache_key = format!("{lang}:{library}:{ver}");
    if let Some(cached) = get_cached(db, &cache_key) {
        let docs = if let Some(t) = topic {
            filter_sections(&cached, t)
        } else {
            truncate_docs(&cached)
        };
        return Ok(format!(
            "[cached] Documentation for {library} ({lang}):\n\n{docs}"
        ));
    }

    // Fetch docs based on language
    let raw_docs = match lang {
        "rust" => fetch_docs_rs(library, ver, client),
        "javascript" | "typescript" | "js" | "ts" => fetch_npm_docs(library, client),
        "python" | "py" => fetch_pypi_docs(library, client),
        "go" | "golang" => fetch_go_docs(library, client),
        "ruby" | "rb" => fetch_rubygems_docs(library, client),
        "php" => fetch_packagist_docs(library, client),
        "java" | "kotlin" | "kt" => fetch_javadoc(library, client),
        "swift" => fetch_swift_docs(library, client),
        "csharp" | "cs" | "dotnet" => fetch_nuget_docs(library, client),
        "elixir" | "ex" => fetch_hex_docs(library, client),
        "dart" | "flutter" => fetch_pub_dev_docs(library, client),
        _ => Err(format!("Unsupported language: {lang}")),
    }?;

    // Cache it
    set_cached(db, &cache_key, &raw_docs);

    let docs = if let Some(t) = topic {
        filter_sections(&raw_docs, t)
    } else {
        truncate_docs(&raw_docs)
    };

    Ok(format!("Documentation for {library} ({lang}):\n\n{docs}"))
}

// ---------------------------------------------------------------------------
// Doc fetchers per registry
// ---------------------------------------------------------------------------

fn fetch_docs_rs(name: &str, version: &str, client: &Client) -> Result<String, String> {
    // docs.rs renders HTML — fetch the crate root page and extract content
    let url = if version == "latest" {
        format!("https://docs.rs/{name}/latest/{name}/", name = name)
    } else {
        format!("https://docs.rs/{name}/{version}/{name}/")
    };
    let html = fetch_html(client, &url)?;
    let doc = Html::parse_document(&html);

    // Extract the main doc content
    let mut content = String::new();

    // Get the crate-level docs (docblock)
    if let Ok(sel) = Selector::parse(".docblock") {
        for el in doc.select(&sel) {
            content.push_str(&extract_text(&el));
            content.push_str("\n\n");
        }
    }

    // Get type/function summaries
    if let Ok(sel) = Selector::parse(".item-table .item-name") {
        for el in doc.select(&sel) {
            content.push_str("- ");
            content.push_str(&extract_text(&el).trim().replace('\n', " "));
            content.push('\n');
        }
    }

    if content.is_empty() {
        // Fallback: fetch README from crates.io
        let readme_url = format!("https://crates.io/api/v1/crates/{name}");
        if let Ok(resp) = client.get(&readme_url).send() {
            if let Ok(data) = resp.json::<Value>() {
                if let Some(readme) = data.pointer("/crate/readme_path") {
                    let _ = readme; // readme is a path, not inline
                }
            }
        }
        // Try GitHub README
        Ok(fetch_github_readme(name, client).unwrap_or_else(|_| {
            format!("No documentation found for crate '{name}'. Visit https://docs.rs/{name}")
        }))
    } else {
        Ok(content)
    }
}

fn fetch_npm_docs(name: &str, client: &Client) -> Result<String, String> {
    // npm registry includes README in the package metadata
    let url = format!("https://registry.npmjs.org/{name}");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    if let Some(readme) = resp.get("readme").and_then(|v| v.as_str()) {
        if !readme.is_empty() && readme != "ERROR: No README data found!" {
            return Ok(readme.to_string());
        }
    }

    // Fallback: try latest version's README
    if let Some(latest) = resp
        .get("dist-tags")
        .and_then(|d| d.get("latest"))
        .and_then(|v| v.as_str())
    {
        let ver_url = format!("https://registry.npmjs.org/{name}/{latest}");
        if let Ok(ver_resp) = client.get(&ver_url).send() {
            if let Ok(ver_data) = ver_resp.json::<Value>() {
                if let Some(readme) = ver_data.get("readme").and_then(|v| v.as_str()) {
                    if !readme.is_empty() {
                        return Ok(readme.to_string());
                    }
                }
            }
        }
    }

    Err(format!("No README found for npm package '{name}'"))
}

fn fetch_pypi_docs(name: &str, client: &Client) -> Result<String, String> {
    let url = format!("https://pypi.org/pypi/{name}/json");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    let info = resp.get("info").ok_or("not found")?;

    // PyPI has description field (usually the full README)
    if let Some(desc) = info.get("description").and_then(|v| v.as_str()) {
        if !desc.is_empty() {
            return Ok(desc.to_string());
        }
    }

    // Fallback to summary
    if let Some(summary) = info.get("summary").and_then(|v| v.as_str()) {
        return Ok(summary.to_string());
    }

    Err(format!("No docs found for Python package '{name}'"))
}

fn fetch_go_docs(name: &str, client: &Client) -> Result<String, String> {
    // Scrape pkg.go.dev
    let url = format!("https://pkg.go.dev/{name}");
    let html = fetch_html(client, &url)?;
    let doc = Html::parse_document(&html);

    let mut content = String::new();

    // Overview section
    if let Ok(sel) = Selector::parse(".Documentation-overview") {
        for el in doc.select(&sel) {
            content.push_str(&extract_text(&el));
            content.push_str("\n\n");
        }
    }

    // Package synopsis
    if let Ok(sel) = Selector::parse(".Documentation-content") {
        for el in doc.select(&sel) {
            let text = extract_text(&el);
            if text.len() > content.len() {
                content = text;
            }
        }
    }

    if content.is_empty() {
        // Try the unit-meta description
        if let Ok(sel) = Selector::parse("[data-test-id='UnitReadme']") {
            for el in doc.select(&sel) {
                content.push_str(&extract_text(&el));
            }
        }
    }

    if content.is_empty() {
        Err(format!("No docs found for Go package '{name}'"))
    } else {
        Ok(content)
    }
}

fn fetch_rubygems_docs(name: &str, client: &Client) -> Result<String, String> {
    // Try rubydoc.info for API docs
    let url = format!("https://www.rubydoc.info/gems/{name}");
    let html = fetch_html(client, &url)?;
    let doc = Html::parse_document(&html);

    let mut content = String::new();
    if let Ok(sel) = Selector::parse("#content") {
        for el in doc.select(&sel) {
            content.push_str(&extract_text(&el));
        }
    }

    if content.is_empty() {
        // Fallback: GitHub README
        fetch_github_readme_for_gem(name, client)
    } else {
        Ok(content)
    }
}

fn fetch_packagist_docs(name: &str, client: &Client) -> Result<String, String> {
    // Packagist shows README on the package page
    let url = format!("https://packagist.org/packages/{name}");
    let html = fetch_html(client, &url)?;
    let doc = Html::parse_document(&html);

    let mut content = String::new();
    if let Ok(sel) = Selector::parse(".package-description, .readme") {
        for el in doc.select(&sel) {
            content.push_str(&extract_text(&el));
            content.push_str("\n\n");
        }
    }

    if content.is_empty() {
        Err(format!("No docs found for PHP package '{name}'"))
    } else {
        Ok(content)
    }
}

fn fetch_javadoc(name: &str, client: &Client) -> Result<String, String> {
    // Search Maven Central first to get group:artifact
    let search_url = format!("https://search.maven.org/solrsearch/select?q={name}&rows=1&wt=json");
    let resp: Value = client
        .get(&search_url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    let doc = resp
        .pointer("/response/docs/0")
        .ok_or(format!("'{name}' not found on Maven Central"))?;
    let group = doc.get("g").and_then(|v| v.as_str()).unwrap_or("");
    let artifact = doc.get("a").and_then(|v| v.as_str()).unwrap_or(name);

    // Fetch javadoc.io overview
    let url = format!("https://javadoc.io/doc/{group}/{artifact}/latest/index.html");
    match fetch_html(client, &url) {
        Ok(html) => {
            let parsed = Html::parse_document(&html);
            let mut content = String::new();
            if let Ok(sel) = Selector::parse(".description, .block") {
                for el in parsed.select(&sel) {
                    content.push_str(&extract_text(&el));
                    content.push_str("\n");
                }
            }
            if content.is_empty() {
                Ok(format!(
                    "Javadoc available at: https://javadoc.io/doc/{group}/{artifact}"
                ))
            } else {
                Ok(content)
            }
        }
        Err(_) => Ok(format!(
            "Javadoc available at: https://javadoc.io/doc/{group}/{artifact}"
        )),
    }
}

fn fetch_swift_docs(name: &str, client: &Client) -> Result<String, String> {
    // Try to find GitHub repo from Swift Package Index
    let url = format!("https://swiftpackageindex.com/api/search?query={name}");
    let resp: Value = client
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    if let Some(results) = resp.get("results").and_then(|v| v.as_array()) {
        if let Some(first) = results.first() {
            if let Some(repo_url) = first.pointer("/package/url").and_then(|v| v.as_str()) {
                // Extract owner/repo from GitHub URL and fetch README
                if let Some(gh) = parse_github_url(repo_url) {
                    return fetch_github_readme_direct(&gh.0, &gh.1, client);
                }
            }
        }
    }

    Err(format!("No docs found for Swift package '{name}'"))
}

fn fetch_nuget_docs(name: &str, client: &Client) -> Result<String, String> {
    // NuGet package page
    let url = format!("https://www.nuget.org/packages/{name}");
    let html = fetch_html(client, &url)?;
    let doc = Html::parse_document(&html);

    let mut content = String::new();

    // Package description
    if let Ok(sel) = Selector::parse(".package-description") {
        for el in doc.select(&sel) {
            content.push_str(&extract_text(&el));
            content.push_str("\n\n");
        }
    }

    // README section
    if let Ok(sel) = Selector::parse("#readme-body") {
        for el in doc.select(&sel) {
            content.push_str(&extract_text(&el));
        }
    }

    if content.is_empty() {
        Err(format!("No docs found for NuGet package '{name}'"))
    } else {
        Ok(content)
    }
}

fn fetch_hex_docs(name: &str, client: &Client) -> Result<String, String> {
    // hexdocs.pm has excellent docs
    let url = format!("https://hexdocs.pm/{name}");
    let html = fetch_html(client, &url)?;
    let doc = Html::parse_document(&html);

    let mut content = String::new();
    if let Ok(sel) = Selector::parse("#content") {
        for el in doc.select(&sel) {
            content.push_str(&extract_text(&el));
        }
    }

    if content.is_empty() {
        // Fallback: hex.pm package page
        let alt_url = format!("https://hex.pm/packages/{name}");
        let alt_html = fetch_html(client, &alt_url)?;
        let alt_doc = Html::parse_document(&alt_html);
        if let Ok(sel) = Selector::parse(".description") {
            for el in alt_doc.select(&sel) {
                content.push_str(&extract_text(&el));
            }
        }
    }

    if content.is_empty() {
        Err(format!("No docs found for Elixir package '{name}'"))
    } else {
        Ok(content)
    }
}

fn fetch_pub_dev_docs(name: &str, client: &Client) -> Result<String, String> {
    // pub.dev renders markdown README
    let url = format!("https://pub.dev/packages/{name}");
    let html = fetch_html(client, &url)?;
    let doc = Html::parse_document(&html);

    let mut content = String::new();

    // README content
    if let Ok(sel) = Selector::parse(".detail-tabs-content .markdown-body, .-pkg-readme") {
        for el in doc.select(&sel) {
            content.push_str(&extract_text(&el));
        }
    }

    if content.is_empty() {
        // Fallback: API endpoint
        let api_url = format!("https://pub.dev/api/packages/{name}");
        if let Ok(resp) = client.get(&api_url).send() {
            if let Ok(data) = resp.json::<Value>() {
                if let Some(desc) = data
                    .pointer("/latest/pubspec/description")
                    .and_then(|v| v.as_str())
                {
                    return Ok(desc.to_string());
                }
            }
        }
        Err(format!("No docs found for Dart package '{name}'"))
    } else {
        Ok(content)
    }
}

// ---------------------------------------------------------------------------
// HTML helpers
// ---------------------------------------------------------------------------

fn fetch_html(client: &Client, url: &str) -> Result<String, String> {
    let resp = client
        .get(url)
        .header("Accept", "text/html")
        .send()
        .map_err(|e| format!("HTTP error: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}: {url}", resp.status()));
    }
    let text = resp.text().map_err(|e| format!("Read error: {e}"))?;
    if text.len() > MAX_DOC_SIZE * 10 {
        // Truncate huge HTML pages before parsing
        Ok(text[..MAX_DOC_SIZE * 10].to_string())
    } else {
        Ok(text)
    }
}

fn extract_text(element: &scraper::ElementRef) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn parse_github_url(url: &str) -> Option<(String, String)> {
    // Parse https://github.com/owner/repo or git@github.com:owner/repo.git
    let url = url.trim_end_matches(".git");
    if let Some(path) = url.strip_prefix("https://github.com/") {
        let parts: Vec<&str> = path.splitn(3, '/').collect();
        if parts.len() >= 2 {
            return Some((parts[0].to_string(), parts[1].to_string()));
        }
    }
    None
}

fn fetch_github_readme(name: &str, client: &Client) -> Result<String, String> {
    // Try common GitHub patterns
    let attempts = [
        format!("https://raw.githubusercontent.com/{name}/{name}/main/README.md"),
        format!("https://raw.githubusercontent.com/{name}/{name}/master/README.md"),
    ];
    for url in &attempts {
        if let Ok(resp) = client.get(url).send() {
            if resp.status().is_success() {
                if let Ok(text) = resp.text() {
                    return Ok(text);
                }
            }
        }
    }
    Err(format!("No GitHub README found for '{name}'"))
}

fn fetch_github_readme_direct(owner: &str, repo: &str, client: &Client) -> Result<String, String> {
    let attempts = [
        format!("https://raw.githubusercontent.com/{owner}/{repo}/main/README.md"),
        format!("https://raw.githubusercontent.com/{owner}/{repo}/master/README.md"),
    ];
    for url in &attempts {
        if let Ok(resp) = client.get(url).send() {
            if resp.status().is_success() {
                if let Ok(text) = resp.text() {
                    return Ok(text);
                }
            }
        }
    }
    Err(format!("No README found for {owner}/{repo}"))
}

fn fetch_github_readme_for_gem(name: &str, client: &Client) -> Result<String, String> {
    // Get gem info to find source code URL
    let url = format!("https://rubygems.org/api/v1/gems/{name}.json");
    if let Ok(resp) = client.get(&url).send() {
        if let Ok(data) = resp.json::<Value>() {
            if let Some(src) = data.get("source_code_uri").and_then(|v| v.as_str()) {
                if let Some((owner, repo)) = parse_github_url(src) {
                    return fetch_github_readme_direct(&owner, &repo, client);
                }
            }
            if let Some(hp) = data.get("homepage_uri").and_then(|v| v.as_str()) {
                if let Some((owner, repo)) = parse_github_url(hp) {
                    return fetch_github_readme_direct(&owner, &repo, client);
                }
            }
        }
    }
    Err(format!("No docs found for Ruby gem '{name}'"))
}

// ---------------------------------------------------------------------------
// Doc filtering and truncation
// ---------------------------------------------------------------------------

fn filter_sections(docs: &str, topic: &str) -> String {
    let topic_lower = topic.to_lowercase();
    let keywords: Vec<&str> = topic_lower.split_whitespace().collect();

    // Split into sections by markdown headers
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut current_header = String::new();
    let mut current_body = String::new();

    for line in docs.lines() {
        if line.starts_with('#') || line.starts_with("##") {
            if !current_header.is_empty() || !current_body.is_empty() {
                sections.push((current_header.clone(), current_body.clone()));
            }
            current_header = line.to_string();
            current_body.clear();
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }
    if !current_header.is_empty() || !current_body.is_empty() {
        sections.push((current_header, current_body));
    }

    // Score and filter sections
    let mut scored: Vec<(usize, &str, &str)> = sections
        .iter()
        .map(|(h, b)| {
            let combined = format!("{} {}", h.to_lowercase(), b.to_lowercase());
            let score: usize = keywords.iter().filter(|k| combined.contains(**k)).count();
            (score, h.as_str(), b.as_str())
        })
        .filter(|(score, _, _)| *score > 0)
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));

    if scored.is_empty() {
        // No sections matched — return truncated full docs
        return truncate_docs(docs);
    }

    let mut result = String::new();
    let mut total_len = 0;
    for (_, header, body) in scored.iter().take(10) {
        if total_len > MAX_DOC_SIZE {
            break;
        }
        if !header.is_empty() {
            result.push_str(header);
            result.push('\n');
        }
        result.push_str(body);
        result.push('\n');
        total_len += header.len() + body.len();
    }

    result
}

fn truncate_docs(docs: &str) -> String {
    if docs.len() <= MAX_DOC_SIZE {
        docs.to_string()
    } else {
        format!(
            "{}\n\n[Truncated — {} bytes total. Use 'topic' parameter to search for specific sections.]",
            &docs[..MAX_DOC_SIZE],
            docs.len()
        )
    }
}

// ---------------------------------------------------------------------------
// SQLite cache
// ---------------------------------------------------------------------------

fn open_cache_db() -> Connection {
    let mut path = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    path.push("flamel");
    path.push("docs_cache.db");
    std::fs::create_dir_all(path.parent().unwrap()).ok();

    let conn = Connection::open(&path).expect("failed to open docs cache DB");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS doc_cache (
            key       TEXT PRIMARY KEY,
            content   TEXT NOT NULL,
            fetched_at INTEGER NOT NULL
        );
        PRAGMA journal_mode=WAL;",
    )
    .expect("failed to create cache table");
    conn
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn get_cached(db: &Connection, key: &str) -> Option<String> {
    let cutoff = now_secs().saturating_sub(CACHE_TTL_SECS) as i64;
    db.query_row(
        "SELECT content FROM doc_cache WHERE key = ?1 AND fetched_at > ?2",
        rusqlite::params![key, cutoff],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

fn set_cached(db: &Connection, key: &str, content: &str) {
    let now = now_secs() as i64;
    db.execute(
        "INSERT OR REPLACE INTO doc_cache (key, content, fetched_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![key, content, now],
    )
    .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(slug: &str, title: &str, description: &str, section: &str, text: &str) -> Value {
        json!({
            "slug": slug,
            "title": title,
            "description": description,
            "section": section,
            "text": text,
        })
    }

    #[test]
    fn product_search_ignores_question_words_and_matches_related_terms() {
        let secrets = document(
            "actions/secrets",
            "Secrets",
            "Store write-only repository and environment credentials.",
            "Actions",
            "Reference secrets in deployment workflows. Values are masked from logs.",
        );
        let runners = document(
            "runner/install",
            "Install Actions Runner",
            "Register a standalone or server-embedded runner.",
            "Actions runners",
            "A runner provides private workflow capacity.",
        );
        let query = "How do Flamel Actions deploy over SSH with repository secrets and self-hosted runners?";

        assert!(score_flamel_document(query, &secrets).is_some());
        assert!(score_flamel_document(query, &runners).is_some());
    }

    #[test]
    fn product_search_finds_artifact_and_release_guidance() {
        let artifacts = document(
            "actions/cache-artifacts",
            "Caches and artifacts",
            "Accelerate jobs and retain useful build output.",
            "Actions",
            "Artifacts preserve outputs from a workflow run and may be uploaded by later jobs.",
        );
        let releases = document(
            "git/releases",
            "Releases",
            "Publish public downloads without exposing private source.",
            "Flamel Git",
            "A release attaches versioned downloads to a repository.",
        );
        let query =
            "How do Flamel Git Actions workflows upload build artifacts and publish a release?";

        assert!(score_flamel_document(query, &artifacts).is_some());
        assert!(score_flamel_document(query, &releases).is_some());
        assert!(
            score_flamel_document(query, &artifacts).unwrap()
                > score_flamel_document("unrelated typography question", &artifacts).unwrap_or(0)
        );
    }
}
