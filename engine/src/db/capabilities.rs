use crate::db::Db;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCatalogEntry {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub source: String,
    pub publisher: String,
    pub version: String,
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub remote_url: Option<String>,
    pub auth_kind: String,
    pub permissions: Vec<String>,
    pub toolsets: Vec<String>,
    pub admin_approved: bool,
    pub builtin: bool,
    pub connected: bool,
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub project_path: Option<String>,
    pub latency_ms: Option<i64>,
    pub tool_count: i64,
    pub consecutive_failures: i64,
    pub restart_count: i64,
    pub circuit_state: String,
    pub last_success_at: Option<String>,
    pub pack_id: Option<String>,
    pub pack_version: Option<String>,
    pub pack_previous_version: Option<String>,
    pub pack_state: Option<String>,
    pub pack_signed: bool,
    pub schedule_eligible: bool,
    pub native_adapter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityTool {
    pub capability_id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub annotations: Value,
    pub enabled: bool,
    pub discovered_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityTask {
    pub id: String,
    pub capability_id: String,
    pub connection_id: Option<String>,
    pub user_id: String,
    pub session_id: Option<String>,
    pub tool_name: String,
    pub status: String,
    pub input: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub progress: f64,
    pub cancel_requested: bool,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub updated_at: String,
}

fn string_array(raw: String) -> Vec<String> {
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn list_catalog(
    db: &Db,
    user_id: &str,
    project_path: Option<&str>,
) -> Vec<CapabilityCatalogEntry> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.slug, c.title, c.description, c.category, c.source,
                    c.publisher, c.version, c.transport, c.command, c.args_json,
                    c.remote_url, c.auth_kind, c.permissions_json, c.toolsets_json,
                    c.admin_approved, c.builtin,
                    x.id, x.name, x.enabled, x.status, x.last_error, x.project_path,
                    x.latency_ms, COALESCE(x.tool_count, (
                        SELECT COUNT(*) FROM capability_tools t WHERE t.capability_id=c.id AND t.enabled=1
                    )), x.consecutive_failures, x.restart_count,
                    x.circuit_state, x.last_success_at,
                    p.pack_id, p.active_version, p.previous_version, p.state,
                    p.schedule_eligible,
                    json_extract(c.metadata_json, '$.connectors[0].nativeAdapter')
             FROM capability_catalog c
             LEFT JOIN capability_connections x ON x.capability_id = c.id
                AND x.user_id = ?1
                AND (x.project_path IS NULL OR x.project_path = ?2)
             LEFT JOIN capability_pack_installations p ON p.capability_id = c.id
             ORDER BY c.builtin DESC, c.category, c.title",
        )
        .unwrap();
    stmt.query_map(
        rusqlite::params![user_id, project_path.unwrap_or("")],
        |row| {
            let connection_id: Option<String> = row.get(17)?;
            Ok(CapabilityCatalogEntry {
                id: row.get(0)?,
                slug: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                category: row.get(4)?,
                source: row.get(5)?,
                publisher: row.get(6)?,
                version: row.get(7)?,
                transport: row.get(8)?,
                command: row.get(9)?,
                args: string_array(row.get(10)?),
                remote_url: row.get(11)?,
                auth_kind: row.get(12)?,
                permissions: string_array(row.get(13)?),
                toolsets: string_array(row.get(14)?),
                admin_approved: row.get::<_, i64>(15)? == 1,
                builtin: row.get::<_, i64>(16)? == 1,
                connected: connection_id.is_some(),
                connection_id,
                connection_name: row.get(18)?,
                enabled: row.get::<_, Option<i64>>(19)?.unwrap_or(0) == 1,
                status: row
                    .get::<_, Option<String>>(20)?
                    .unwrap_or_else(|| "available".into()),
                last_error: row.get(21)?,
                project_path: row.get(22)?,
                latency_ms: row.get(23)?,
                tool_count: row.get::<_, Option<i64>>(24)?.unwrap_or(0),
                consecutive_failures: row.get::<_, Option<i64>>(25)?.unwrap_or(0),
                restart_count: row.get::<_, Option<i64>>(26)?.unwrap_or(0),
                circuit_state: row
                    .get::<_, Option<String>>(27)?
                    .unwrap_or_else(|| "closed".into()),
                last_success_at: row.get(28)?,
                pack_id: row.get(29)?,
                pack_version: row.get(30)?,
                pack_previous_version: row.get(31)?,
                pack_state: row.get(32)?,
                pack_signed: row.get::<_, Option<String>>(29)?.is_some(),
                schedule_eligible: row.get::<_, Option<i64>>(33)?.unwrap_or(0) == 1,
                native_adapter: row.get(34)?,
            })
        },
    )
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

pub fn seed_builtin_catalog(db: &Db) {
    let now = chrono::Utc::now().to_rfc3339();
    let conn = db.conn();
    let retired = [
        "builtin-git",
        "builtin-browser",
        "builtin-docs",
        "builtin-db",
        "builtin-memory",
        "builtin-viz",
    ];
    for capability_id in retired {
        let _ = conn.execute(
            "DELETE FROM capability_tools WHERE capability_id=?1",
            [capability_id],
        );
        let _ = conn.execute(
            "DELETE FROM capability_connections WHERE capability_id=?1",
            [capability_id],
        );
        let _ = conn.execute(
            "DELETE FROM capability_catalog WHERE id=?1",
            [capability_id],
        );
    }
    let _ = conn.execute(
        "INSERT INTO capability_catalog (
                id, slug, title, description, category, source, publisher,
                transport, admin_approved, builtin, created_at, updated_at
             ) VALUES ('builtin-flamel', 'flamel', 'Flamel', ?1, 'development',
                'builtin', 'Flamel', 'stdio', 1, 1, ?2, ?2)
             ON CONFLICT(id) DO UPDATE SET title='Flamel', description=excluded.description,
                category='development', updated_at=excluded.updated_at",
        rusqlite::params![
            "Files, code navigation, plans, agents, checks, assets, Git, documentation, databases, browser automation, and project memory",
            now,
        ],
    );
    drop(conn);
    seed_builtin_tools(db);
}

fn seed_builtin_tools(db: &Db) {
    let now = chrono::Utc::now().to_rfc3339();
    let conn = db.conn();
    for capability in crate::mcp_tool_catalog::FLAMEL_TOOL_CAPABILITIES {
        let description = if capability.validation.is_empty() {
            capability.display_name.to_string()
        } else {
            format!(
                "{}: {}",
                capability.display_name,
                capability.validation.join("; ")
            )
        };
        let _ = conn.execute(
            "INSERT INTO capability_tools (capability_id,name,description,input_schema_json,
             annotations_json,enabled,discovered_at) VALUES ('builtin-flamel',?1,?2,'{\"type\":\"object\"}',?3,1,?4)
             ON CONFLICT(capability_id,name) DO UPDATE SET description=excluded.description,
             annotations_json=excluded.annotations_json,discovered_at=excluded.discovered_at",
            rusqlite::params![
                capability.name,
                description,
                crate::mcp_tool_catalog::capability_annotations_value(capability).to_string(),
                now
            ],
        );
    }
    let bundled = [
        (
            "builtin-flamel",
            "git",
            "Canonical Flamel Git operations plus explicit connected GitHub and Gitea operations",
        ),
        (
            "builtin-flamel",
            "resolve_library",
            "Resolve a package or library to version-aware documentation",
        ),
        (
            "builtin-flamel",
            "get_docs",
            "Retrieve version-aware library documentation",
        ),
        (
            "builtin-flamel",
            "search_flamel_docs",
            "Search canonical Flamel product and operations documentation",
        ),
        (
            "builtin-flamel",
            "get_flamel_doc",
            "Read a canonical Flamel documentation page",
        ),
        (
            "builtin-flamel",
            "db",
            "Read-only database schema and query exploration",
        ),
        (
            "builtin-flamel",
            "memory",
            "Retrieve and maintain evidence-backed project memory",
        ),
    ];
    for (capability_id, name, description) in bundled {
        let _ = conn.execute(
            "INSERT INTO capability_tools (capability_id,name,description,input_schema_json,
             annotations_json,enabled,discovered_at) VALUES (?1,?2,?3,'{\"type\":\"object\"}','{}',1,?4)
             ON CONFLICT(capability_id,name) DO UPDATE SET description=excluded.description,
             discovered_at=excluded.discovered_at",
            rusqlite::params![capability_id, name, description, now],
        );
    }
    // `browser` was the legacy one-shot sidecar. The compact persistent browser
    // surface is now represented by browser_session/browser_run/browser_inspect.
    let _ = conn.execute(
        "DELETE FROM capability_tools WHERE capability_id='builtin-flamel' AND name='browser'",
        [],
    );
}

pub fn seed_marketplace(db: &Db) {
    let now = chrono::Utc::now().to_rfc3339();
    let entries = [
        (
            "sourcegraph",
            "Sourcegraph",
            "Code search and code graph intelligence across repositories",
            "code-intelligence",
            "Sourcegraph",
            "https://sourcegraph.com",
            "",
            "token",
            json!(["repository:read"]),
        ),
        (
            "sentry",
            "Sentry",
            "Inspect production errors, traces, releases, and issue context",
            "observability",
            "Sentry",
            "https://sentry.io",
            "https://mcp.sentry.dev/mcp",
            "oauth2",
            json!(["issues:read", "projects:read"]),
        ),
        (
            "grafana",
            "Grafana",
            "Query dashboards, metrics, logs, alerts, and incident context",
            "observability",
            "Grafana Labs",
            "https://grafana.com",
            "https://mcp.grafana.com/mcp",
            "oauth2",
            json!(["observability:read"]),
        ),
        (
            "github",
            "GitHub",
            "Repositories, issues, pull requests, reviews, and Actions",
            "source-control",
            "GitHub",
            "https://github.com",
            "https://api.githubcopilot.com/mcp/",
            "oauth2",
            json!(["repositories:read", "issues:write", "pull_requests:write"]),
        ),
        (
            "gitlab",
            "GitLab",
            "Projects, merge requests, issues, pipelines, and deployments",
            "source-control",
            "GitLab",
            "https://gitlab.com",
            "",
            "token",
            json!(["projects:read", "merge_requests:write"]),
        ),
        (
            "aws",
            "AWS",
            "Discover and operate approved AWS services through scoped credentials",
            "cloud",
            "Amazon Web Services",
            "https://aws.amazon.com",
            "",
            "token",
            json!(["cloud:read"]),
        ),
        (
            "kubernetes",
            "Kubernetes",
            "Inspect clusters, workloads, events, manifests, and rollout health",
            "infrastructure",
            "Flamel curated",
            "https://kubernetes.io",
            "",
            "token",
            json!(["cluster:read"]),
        ),
        (
            "terraform",
            "Terraform",
            "Inspect modules, providers, plans, state metadata, and policy checks",
            "infrastructure",
            "HashiCorp",
            "https://terraform.io",
            "",
            "token",
            json!(["infrastructure:read"]),
        ),
    ];
    let conn = db.conn();
    for (slug, title, description, category, publisher, homepage, remote_url, auth, permissions) in
        entries
    {
        let _ = conn.execute(
            "INSERT OR IGNORE INTO capability_marketplace (id,slug,title,description,category,publisher,
             official,homepage_url,transport,remote_url,auth_kind,permissions_json,status,metadata_json,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,1,?7,'streamable_http',NULLIF(?8,''),?9,?10,'available','{}',?11,?11)
             ON CONFLICT(slug) DO UPDATE SET title=excluded.title,description=excluded.description,
             category=excluded.category,publisher=excluded.publisher,homepage_url=excluded.homepage_url,
             remote_url=excluded.remote_url,auth_kind=excluded.auth_kind,permissions_json=excluded.permissions_json,
             updated_at=excluded.updated_at",
            rusqlite::params![format!("market-{slug}"),slug,title,description,category,publisher,homepage,remote_url,auth,permissions.to_string(),now],
        );
    }
}

pub fn list_marketplace(db: &Db) -> Vec<Value> {
    let mut entries = {
        let conn = db.conn();
        let mut stmt=match conn.prepare("SELECT id,slug,title,description,category,publisher,official,homepage_url,
            transport,remote_url,auth_kind,permissions_json,status,metadata_json FROM capability_marketplace ORDER BY category,title") {Ok(s)=>s,Err(_)=>return vec![]};
        stmt.query_map([],|r|{
            let permissions:String=r.get(11)?; let metadata:String=r.get(13)?;
            Ok(json!({"id":r.get::<_,String>(0)?,"slug":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,
                "description":r.get::<_,String>(3)?,"category":r.get::<_,String>(4)?,"publisher":r.get::<_,String>(5)?,
                "official":r.get::<_,i64>(6)?==1,"homepageUrl":r.get::<_,Option<String>>(7)?,"transport":r.get::<_,String>(8)?,
                "remoteUrl":r.get::<_,Option<String>>(9)?,"authKind":r.get::<_,String>(10)?,
                "permissions":serde_json::from_str::<Value>(&permissions).unwrap_or_else(|_|json!([])),
                "status":r.get::<_,String>(12)?,"metadata":serde_json::from_str::<Value>(&metadata).unwrap_or_else(|_|json!({}))}))
        }).map(|rows|rows.filter_map(Result::ok).collect::<Vec<_>>()).unwrap_or_default()
    };
    if let Ok(metadata) = crate::db::runtime_catalog::active_runtime_marketplace_metadata(db) {
        for entry in &mut entries {
            let Some(slug) = entry.get("slug").and_then(Value::as_str) else {
                continue;
            };
            let Some(overlay) = metadata.iter().find(|overlay| overlay.slug == slug) else {
                continue;
            };
            entry["title"] = json!(overlay.title);
            entry["description"] = json!(overlay.description);
            entry["category"] = json!(overlay.category);
            entry["publisher"] = json!(overlay.publisher);
            entry["status"] = json!(overlay.status);
        }
        entries.sort_by(|left, right| {
            left.get("category")
                .and_then(Value::as_str)
                .cmp(&right.get("category").and_then(Value::as_str))
                .then_with(|| {
                    left.get("title")
                        .and_then(Value::as_str)
                        .cmp(&right.get("title").and_then(Value::as_str))
                })
        });
    }
    entries
}

pub fn install_marketplace(db: &Db, marketplace_id: &str) -> Result<String, String> {
    let entry = list_marketplace(db)
        .into_iter()
        .find(|v| v.get("id").and_then(Value::as_str) == Some(marketplace_id))
        .ok_or("Marketplace entry not found")?;
    let remote = entry.get("remoteUrl").and_then(Value::as_str);
    if remote.is_none() {
        return Err("This integration requires a server-admin supplied local command".into());
    }
    let slug = entry.get("slug").and_then(Value::as_str).unwrap_or("");
    let id = format!("registry-{slug}");
    let now = chrono::Utc::now().to_rfc3339();
    db.conn().execute(
        "INSERT INTO capability_catalog (id,slug,title,description,category,source,publisher,version,
         transport,remote_url,auth_kind,permissions_json,toolsets_json,admin_approved,builtin,metadata_json,created_at,updated_at)
         VALUES (?1,?2,?3,?4,?5,'registry',?6,'pinned',?7,?8,?9,?10,'[]',1,0,?11,?12,?12)
         ON CONFLICT(slug) DO UPDATE SET title=excluded.title,description=excluded.description,
         publisher=excluded.publisher,remote_url=excluded.remote_url,permissions_json=excluded.permissions_json,
         admin_approved=1,updated_at=excluded.updated_at",
        rusqlite::params![id,slug,entry["title"].as_str(),entry["description"].as_str(),entry["category"].as_str(),
            entry["publisher"].as_str(),entry["transport"].as_str(),remote,entry["authKind"].as_str(),
            entry["permissions"].to_string(),json!({"marketplaceId":marketplace_id,"official":entry["official"]}).to_string(),now]
    ).map_err(|e|e.to_string())?;
    Ok(id)
}

pub fn import_marketplace_manifest(db: &Db, body: &Value) -> Result<Value, String> {
    let entries = body
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("Manifest entries are required")?;
    if entries.is_empty() || entries.len() > 200 {
        return Err("Manifest must contain 1-200 entries".into());
    }
    let registry = body
        .get("registry")
        .and_then(Value::as_str)
        .unwrap_or("private")
        .trim();
    if registry.is_empty()
        || !registry
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err(
            "Registry name must contain only letters, numbers, dashes, or underscores".into(),
        );
    }
    if body
        .get("manifestUrl")
        .and_then(Value::as_str)
        .is_some_and(|url| !url.starts_with("https://"))
    {
        return Err("Private registry manifest URLs must use HTTPS".into());
    }
    let now = chrono::Utc::now().to_rfc3339();
    let mut imported = Vec::new();
    for entry in entries {
        let slug = entry
            .get("slug")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let title = entry
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let transport = entry
            .get("transport")
            .and_then(Value::as_str)
            .unwrap_or("streamable_http");
        let remote = entry.get("remoteUrl").and_then(Value::as_str);
        if slug.is_empty()
            || title.is_empty()
            || !slug
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        {
            return Err("Every manifest entry needs a valid slug and title".into());
        }
        if !matches!(transport, "streamable_http" | "sse") {
            return Err("Private registry entries must use an HTTP transport".into());
        }
        if remote.is_some_and(|url| !url.starts_with("https://")) {
            return Err("Private registry MCP URLs must use HTTPS".into());
        }
        if entry
            .get("checksum")
            .and_then(Value::as_str)
            .is_some_and(|checksum| {
                checksum.len() != 64 || !checksum.chars().all(|ch| ch.is_ascii_hexdigit())
            })
        {
            return Err("Checksums must be 64-character SHA-256 hex values".into());
        }
        let id = format!("private-{registry}-{slug}");
        let existing: Option<(String, i64)> = db
            .conn()
            .query_row(
                "SELECT id,official FROM capability_marketplace WHERE slug=?1",
                [slug],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        if existing
            .as_ref()
            .is_some_and(|(existing_id, official)| *official == 1 || existing_id != &id)
        {
            return Err(format!(
                "Marketplace slug '{slug}' is already owned by another publisher"
            ));
        }
        let metadata = json!({"registry":registry,"version":entry.get("version"),"checksum":entry.get("checksum"),"schemaValidated":true});
        db.conn().execute("INSERT INTO capability_marketplace (id,slug,title,description,category,publisher,official,homepage_url,manifest_url,transport,remote_url,auth_kind,permissions_json,checksum,status,metadata_json,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,0,?7,?8,?9,?10,?11,?12,?13,'available',?14,?15,?15) ON CONFLICT(slug) DO UPDATE SET title=excluded.title,description=excluded.description,category=excluded.category,publisher=excluded.publisher,homepage_url=excluded.homepage_url,manifest_url=excluded.manifest_url,transport=excluded.transport,remote_url=excluded.remote_url,auth_kind=excluded.auth_kind,permissions_json=excluded.permissions_json,checksum=excluded.checksum,metadata_json=excluded.metadata_json,updated_at=excluded.updated_at",
            rusqlite::params![id,slug,title,entry.get("description").and_then(Value::as_str).unwrap_or(""),entry.get("category").and_then(Value::as_str).unwrap_or("developer"),entry.get("publisher").and_then(Value::as_str).unwrap_or(registry),entry.get("homepageUrl").and_then(Value::as_str),body.get("manifestUrl").and_then(Value::as_str),transport,remote,entry.get("authKind").and_then(Value::as_str).unwrap_or("token"),entry.get("permissions").cloned().unwrap_or_else(||json!([])).to_string(),entry.get("checksum").and_then(Value::as_str),metadata.to_string(),now]).map_err(|error|error.to_string())?;
        imported.push(slug.to_string());
    }
    let count = imported.len();
    Ok(json!({"registry":registry,"imported":imported,"count":count}))
}

pub fn seed_skills(db: &Db) {
    crate::db::skill_catalog::seed_builtin_versions(db);
}

pub fn search_skills(db: &Db, query: &str) -> Vec<Value> {
    let query = query.to_ascii_lowercase();
    let conn = db.conn();
    let mut stmt=match conn.prepare("SELECT id,slug,title,description,category,instructions,triggers_json FROM capability_skills WHERE enabled=1 ORDER BY title"){Ok(s)=>s,Err(_)=>return vec![]};
    stmt.query_map([],|r|{let triggers:String=r.get(6)?;Ok(json!({"id":r.get::<_,String>(0)?,"slug":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"description":r.get::<_,String>(3)?,"category":r.get::<_,String>(4)?,"instructions":r.get::<_,String>(5)?,"triggers":serde_json::from_str::<Value>(&triggers).unwrap_or_else(|_|json!([]))}))})
        .map(|rows|rows.filter_map(Result::ok).filter(|v|query.is_empty()||format!("{} {} {} {}",v["title"],v["description"],v["category"],v["triggers"]).to_ascii_lowercase().contains(&query)).take(10).collect()).unwrap_or_default()
}

pub fn create_catalog_entry(db: &Db, body: &Value) -> Result<CapabilityCatalogEntry, String> {
    let slug = body
        .get("slug")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let title = body
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(slug)
        .trim();
    let transport = body
        .get("transport")
        .and_then(Value::as_str)
        .unwrap_or("stdio");
    if slug.is_empty() || title.is_empty() {
        return Err("Name and title are required".into());
    }
    if !matches!(transport, "stdio" | "streamable_http" | "sse") {
        return Err("Transport must be stdio, streamable_http, or sse".into());
    }
    let command = body.get("command").and_then(Value::as_str);
    let remote_url = body.get("remoteUrl").and_then(Value::as_str);
    if transport == "stdio" && command.unwrap_or("").trim().is_empty() {
        return Err("A command is required for local integrations".into());
    }
    if transport != "stdio" && remote_url.unwrap_or("").trim().is_empty() {
        return Err("A remote URL is required for remote integrations".into());
    }
    if transport != "stdio" {
        let parsed = reqwest::Url::parse(remote_url.unwrap_or(""))
            .map_err(|_| "Remote URL is invalid".to_string())?;
        let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
        if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
            return Err("Remote integrations require HTTPS except on loopback".into());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let args = body.get("args").cloned().unwrap_or_else(|| json!([]));
    let permissions = body
        .get("permissions")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let toolsets = body.get("toolsets").cloned().unwrap_or_else(|| json!([]));
    db.conn().execute(
        "INSERT INTO capability_catalog (
            id, slug, title, description, category, source, publisher, version,
            transport, command, args_json, remote_url, auth_kind, permissions_json,
            toolsets_json, admin_approved, builtin, metadata_json, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'admin', ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 1, 0, '{}', ?15, ?15)",
        rusqlite::params![
            id, slug, title,
            body.get("description").and_then(Value::as_str).unwrap_or(""),
            body.get("category").and_then(Value::as_str).unwrap_or("developer"),
            body.get("publisher").and_then(Value::as_str).unwrap_or("Custom"),
            body.get("version").and_then(Value::as_str).unwrap_or(""),
            transport, command, args.to_string(), remote_url,
            body.get("authKind").and_then(Value::as_str).unwrap_or("none"),
            permissions.to_string(), toolsets.to_string(), now,
        ],
    ).map_err(|e| if e.to_string().contains("UNIQUE") { "An integration with that name already exists".into() } else { e.to_string() })?;
    Ok(list_catalog(db, "", None)
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or(CapabilityCatalogEntry {
            id,
            slug: slug.into(),
            title: title.into(),
            description: String::new(),
            category: "developer".into(),
            source: "admin".into(),
            publisher: "Custom".into(),
            version: String::new(),
            transport: transport.into(),
            command: command.map(str::to_string),
            args: string_array(args.to_string()),
            remote_url: remote_url.map(str::to_string),
            auth_kind: "none".into(),
            permissions: vec![],
            toolsets: vec![],
            admin_approved: true,
            builtin: false,
            connected: false,
            connection_id: None,
            connection_name: None,
            enabled: false,
            status: "available".into(),
            last_error: None,
            project_path: None,
            latency_ms: None,
            tool_count: 0,
            consecutive_failures: 0,
            restart_count: 0,
            circuit_state: "closed".into(),
            last_success_at: None,
            pack_id: None,
            pack_version: None,
            pack_previous_version: None,
            pack_state: None,
            pack_signed: false,
            schedule_eligible: false,
            native_adapter: None,
        }))
}

pub fn connect(
    db: &Db,
    capability_id: &str,
    user_id: &str,
    project_path: Option<&str>,
) -> Result<(), String> {
    let approved: Option<i64> = db
        .conn()
        .query_row(
            "SELECT admin_approved FROM capability_catalog WHERE id = ?1",
            [capability_id],
            |row| row.get(0),
        )
        .ok();
    if approved != Some(1) {
        return Err("Integration is not approved by an administrator".into());
    }
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO capability_connections (
            id, capability_id, user_id, project_path, name, enabled, config_json,
            status, created_at, updated_at
         )
         SELECT ?1, id, ?2, ?3, slug, 1,
                json_object('grantedScopes', json(permissions_json)),
                'configured', ?4, ?4
         FROM capability_catalog WHERE id = ?5
         ON CONFLICT(capability_id, user_id, IFNULL(project_path, ''))
         DO UPDATE SET enabled = 1, config_json=excluded.config_json,
                       status = 'configured', updated_at = excluded.updated_at",
            rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                user_id,
                project_path,
                now,
                capability_id
            ],
        )
        .map_err(|e| e.to_string())?;
    let _ = db.conn().execute(
        "UPDATE mcp_configs SET enabled = 1 WHERE id = (
            SELECT json_extract(metadata_json, '$.legacyMcpConfigId')
            FROM capability_catalog WHERE id = ?1 AND source = 'legacy'
        )",
        [capability_id],
    );
    Ok(())
}

pub fn connection_has_exact_scopes(
    db: &Db,
    capability_id: &str,
    user_id: &str,
    project_path: Option<&str>,
) -> bool {
    let values: Option<(String, String)> = db
        .conn()
        .query_row(
            "SELECT c.permissions_json,x.config_json
             FROM capability_catalog c
             JOIN capability_connections x ON x.capability_id=c.id
             WHERE c.id=?1 AND x.user_id=?2
               AND IFNULL(x.project_path,'')=IFNULL(?3,'')
               AND x.enabled=1",
            rusqlite::params![capability_id, user_id, project_path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let Some((required, config)) = values else {
        return false;
    };
    let mut required = serde_json::from_str::<Vec<String>>(&required).unwrap_or_default();
    let mut granted = serde_json::from_str::<Value>(&config)
        .ok()
        .and_then(|value| value.get("grantedScopes").cloned())
        .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
        .unwrap_or_default();
    required.sort();
    required.dedup();
    granted.sort();
    granted.dedup();
    required == granted
}

pub fn validate_credential_kind(
    db: &Db,
    capability_id: &str,
    credential_kind: &str,
) -> Result<(), String> {
    let entry: Option<(String, String, String)> = db
        .conn()
        .query_row(
            "SELECT source,auth_kind,metadata_json FROM capability_catalog WHERE id=?1",
            [capability_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();
    let Some((source, auth_kind, metadata)) = entry else {
        return Err("Capability not found".into());
    };
    if source != "first_party_pack" {
        return Ok(());
    }
    if auth_kind == "none" {
        return Err("This signed pack does not accept credentials".into());
    }
    let allowed = serde_json::from_str::<Value>(&metadata)
        .ok()
        .and_then(|value| value.pointer("/auth/credentialKinds").cloned())
        .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
        .unwrap_or_default();
    if !allowed.iter().any(|allowed| allowed == credential_kind) {
        return Err("Credential kind is not declared by the signed pack".into());
    }
    Ok(())
}

pub fn disconnect(db: &Db, capability_id: &str, user_id: &str, project_path: Option<&str>) {
    let _ = db.conn().execute(
        "DELETE FROM capability_connections WHERE capability_id = ?1 AND user_id = ?2 AND IFNULL(project_path, '') = IFNULL(?3, '')",
        rusqlite::params![capability_id, user_id, project_path],
    );
    let _ = db.conn().execute(
        "DELETE FROM capability_credentials WHERE capability_id=?1 AND user_id=?2",
        rusqlite::params![capability_id, user_id],
    );
    let _ = db.conn().execute(
        "UPDATE capability_pack_installations SET schedules_enabled=0,updated_at=?1
         WHERE capability_id=?2",
        rusqlite::params![chrono::Utc::now().to_rfc3339(), capability_id],
    );
    let _ = db.conn().execute(
        "UPDATE mcp_configs SET enabled = 0 WHERE id = (
            SELECT json_extract(metadata_json, '$.legacyMcpConfigId')
            FROM capability_catalog WHERE id = ?1 AND source = 'legacy'
        )",
        [capability_id],
    );
}

pub fn set_connection_status(
    db: &Db,
    capability_id: &str,
    user_id: &str,
    project_path: Option<&str>,
    status: &str,
    error: Option<&str>,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let _ = db.conn().execute(
        "UPDATE capability_connections SET status = ?1, last_error = ?2,
         health_checked_at = ?3, last_success_at = CASE WHEN ?1 = 'ready' THEN ?3 ELSE last_success_at END,
         updated_at = ?3 WHERE capability_id = ?4 AND user_id = ?5
         AND IFNULL(project_path, '') = IFNULL(?6, '')",
        rusqlite::params![status, error, now, capability_id, user_id, project_path],
    );
}

pub fn record_health(
    db: &Db,
    capability_id: &str,
    user_id: &str,
    project_path: Option<&str>,
    success: bool,
    latency_ms: i64,
    tool_count: i64,
    process_id: Option<i64>,
    error: Option<&str>,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let connection_id: Option<String> = db
        .conn()
        .query_row(
            "SELECT id FROM capability_connections WHERE capability_id = ?1 AND user_id = ?2
         AND IFNULL(project_path, '') = IFNULL(?3, '')",
            rusqlite::params![capability_id, user_id, project_path],
            |row| row.get(0),
        )
        .ok();
    let Some(connection_id) = connection_id else {
        return;
    };
    let failures: i64 = db
        .conn()
        .query_row(
            "SELECT consecutive_failures FROM capability_connections WHERE id = ?1",
            [&connection_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let next_failures = if success { 0 } else { failures + 1 };
    let circuit = if next_failures >= 3 { "open" } else { "closed" };
    let status = if success {
        "ready"
    } else if circuit == "open" {
        "unavailable"
    } else {
        "failed"
    };
    let _ = db.conn().execute(
        "UPDATE capability_connections SET status = ?1, last_error = ?2, health_checked_at = ?3,
         last_success_at = CASE WHEN ?4 THEN ?3 ELSE last_success_at END,
         latency_ms = ?5, tool_count = ?6, consecutive_failures = ?7, circuit_state = ?8,
         updated_at = ?3 WHERE id = ?9",
        rusqlite::params![
            status,
            error,
            now,
            success,
            latency_ms,
            tool_count,
            next_failures,
            circuit,
            connection_id
        ],
    );
    let _ = db.conn().execute(
        "INSERT INTO capability_health (capability_id, connection_id, process_id, tool_count,
         consecutive_failures, circuit_state, latency_ms, initialized_at, last_success_at,
         last_failure_at, last_error, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CASE WHEN ?8 THEN ?9 END,
         CASE WHEN ?8 THEN ?9 END, CASE WHEN ?8 THEN NULL ELSE ?9 END, ?10, ?9)
         ON CONFLICT(capability_id, connection_id) DO UPDATE SET process_id=excluded.process_id,
         tool_count=excluded.tool_count, consecutive_failures=excluded.consecutive_failures,
         circuit_state=excluded.circuit_state, latency_ms=excluded.latency_ms,
         initialized_at=CASE WHEN ?8 THEN COALESCE(capability_health.initialized_at, ?9) ELSE capability_health.initialized_at END,
         last_success_at=CASE WHEN ?8 THEN ?9 ELSE capability_health.last_success_at END,
         last_failure_at=CASE WHEN ?8 THEN capability_health.last_failure_at ELSE ?9 END,
         last_error=?10, updated_at=?9",
        rusqlite::params![capability_id, connection_id, process_id, tool_count, next_failures,
            circuit, latency_ms, success, now, error],
    );
}

pub fn reset_circuit(db: &Db, capability_id: &str, user_id: &str, project_path: Option<&str>) {
    let now = chrono::Utc::now().to_rfc3339();
    let _ = db.conn().execute(
        "UPDATE capability_connections SET circuit_state='closed', consecutive_failures=0,
         status='configured', last_error=NULL, updated_at=?1
         WHERE capability_id=?2 AND user_id=?3 AND IFNULL(project_path,'')=IFNULL(?4,'')",
        rusqlite::params![now, capability_id, user_id, project_path],
    );
}

pub fn record_restart(db: &Db, capability_id: &str, user_id: &str, project_path: Option<&str>) {
    let now = chrono::Utc::now().to_rfc3339();
    let _ = db.conn().execute(
        "UPDATE capability_connections SET restart_count=restart_count+1, updated_at=?1
         WHERE capability_id=?2 AND user_id=?3 AND IFNULL(project_path,'')=IFNULL(?4,'')",
        rusqlite::params![now, capability_id, user_id, project_path],
    );
    let _ = db.conn().execute(
        "UPDATE capability_health SET restart_count=restart_count+1, updated_at=?1
         WHERE capability_id=?2 AND connection_id=(
             SELECT id FROM capability_connections WHERE capability_id=?2 AND user_id=?3
             AND IFNULL(project_path,'')=IFNULL(?4,'')
         )",
        rusqlite::params![now, capability_id, user_id, project_path],
    );
}

pub fn upsert_discovered_tools(db: &Db, capability_id: &str, tools: &[Value]) {
    let now = chrono::Utc::now().to_rfc3339();
    let conn = db.conn();
    let _ = conn.execute(
        "DELETE FROM capability_tools WHERE capability_id=?1",
        [capability_id],
    );
    for tool in tools {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            continue;
        };
        let input = tool
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let output = tool.get("outputSchema").cloned();
        let annotations = tool
            .get("annotations")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let _ = conn.execute(
            "INSERT INTO capability_tools (capability_id,name,description,input_schema_json,
             output_schema_json,annotations_json,enabled,discovered_at)
             VALUES (?1,?2,?3,?4,?5,?6,1,?7)",
            rusqlite::params![
                capability_id,
                name,
                tool.get("description")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                input.to_string(),
                output.map(|v| v.to_string()),
                annotations.to_string(),
                now
            ],
        );
    }
}

pub fn upsert_discovered_surfaces(
    db: &Db,
    capability_id: &str,
    resources: &[Value],
    prompts: &[Value],
) {
    let now = chrono::Utc::now().to_rfc3339();
    let conn = db.conn();
    let _ = conn.execute(
        "DELETE FROM capability_surfaces WHERE capability_id=?1",
        [capability_id],
    );
    for (kind, entries) in [("resource", resources), ("prompt", prompts)] {
        for entry in entries {
            let name = entry
                .get("name")
                .or_else(|| entry.get("uri"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let _ = conn.execute(
                "INSERT INTO capability_surfaces (capability_id,kind,name,description,definition_json,discovered_at) VALUES (?1,?2,?3,?4,?5,?6)",
                rusqlite::params![capability_id,kind,name,entry.get("description").and_then(Value::as_str).unwrap_or(""),entry.to_string(),now],
            );
        }
    }
}

pub fn list_surfaces(db: &Db, capability_id: &str) -> Vec<Value> {
    let conn = db.conn();
    let mut stmt=match conn.prepare("SELECT kind,name,description,definition_json,discovered_at FROM capability_surfaces WHERE capability_id=?1 ORDER BY kind,name"){Ok(stmt)=>stmt,Err(_)=>return vec![]};
    stmt.query_map([capability_id],|row|{
        let definition:String=row.get(3)?;
        Ok(json!({"kind":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"description":row.get::<_,String>(2)?,"definition":serde_json::from_str::<Value>(&definition).unwrap_or_else(|_|json!({})),"discoveredAt":row.get::<_,String>(4)?}))
    }).map(|rows|rows.filter_map(Result::ok).collect()).unwrap_or_default()
}

pub fn list_tools(db: &Db, capability_id: &str, enabled_only: bool) -> Vec<CapabilityTool> {
    let conn = db.conn();
    let sql = if enabled_only {
        "SELECT capability_id,name,description,input_schema_json,output_schema_json,
         annotations_json,enabled,discovered_at FROM capability_tools
         WHERE capability_id=?1 AND enabled=1 ORDER BY name"
    } else {
        "SELECT capability_id,name,description,input_schema_json,output_schema_json,
         annotations_json,enabled,discovered_at FROM capability_tools
         WHERE capability_id=?1 ORDER BY name"
    };
    let mut stmt = match conn.prepare(sql) {
        Ok(stmt) => stmt,
        Err(_) => return vec![],
    };
    stmt.query_map([capability_id], |row| {
        let input: String = row.get(3)?;
        let output: Option<String> = row.get(4)?;
        let annotations: String = row.get(5)?;
        Ok(CapabilityTool {
            capability_id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            input_schema: serde_json::from_str(&input).unwrap_or_else(|_| json!({})),
            output_schema: output.and_then(|value| serde_json::from_str(&value).ok()),
            annotations: serde_json::from_str(&annotations).unwrap_or_else(|_| json!({})),
            enabled: row.get::<_, i64>(6)? == 1,
            discovered_at: row.get(7)?,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

fn validate_schema_value(
    schema: &Value,
    value: &Value,
    path: &str,
    depth: usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err("Capability tool input schema nesting exceeds 16 levels".into());
    }
    if let Some(choices) = schema.get("enum").and_then(Value::as_array)
        && !choices.contains(value)
    {
        return Err(format!("{path} is not one of the allowed values"));
    }
    let Some(kind) = schema.get("type").and_then(Value::as_str) else {
        return Ok(());
    };
    match kind {
        "object" => {
            let object = value
                .as_object()
                .ok_or_else(|| format!("{path} must be an object"))?;
            if let Some(required) = schema.get("required").and_then(Value::as_array) {
                for field in required.iter().filter_map(Value::as_str) {
                    if !object.contains_key(field) {
                        return Err(format!("{path}.{field} is required"));
                    }
                }
            }
            let properties = schema.get("properties").and_then(Value::as_object);
            if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
                for field in object.keys() {
                    if !properties.is_some_and(|properties| properties.contains_key(field)) {
                        return Err(format!(
                            "{path}.{field} is not allowed by the signed schema"
                        ));
                    }
                }
            }
            if let Some(properties) = properties {
                for (field, child) in object {
                    if let Some(child_schema) = properties.get(field) {
                        validate_schema_value(
                            child_schema,
                            child,
                            &format!("{path}.{field}"),
                            depth + 1,
                        )?;
                    }
                }
            }
        }
        "array" => {
            let values = value
                .as_array()
                .ok_or_else(|| format!("{path} must be an array"))?;
            if let Some(items) = schema.get("items") {
                for (index, item) in values.iter().enumerate() {
                    validate_schema_value(items, item, &format!("{path}[{index}]"), depth + 1)?;
                }
            }
        }
        "string" if !value.is_string() => return Err(format!("{path} must be a string")),
        "integer" if !value.as_i64().is_some() && !value.as_u64().is_some() => {
            return Err(format!("{path} must be an integer"));
        }
        "number" if !value.is_number() => return Err(format!("{path} must be a number")),
        "boolean" if !value.is_boolean() => return Err(format!("{path} must be a boolean")),
        "null" if !value.is_null() => return Err(format!("{path} must be null")),
        "string" | "integer" | "number" | "boolean" | "null" => {}
        _ => return Err(format!("{path} uses an unsupported JSON schema type")),
    }
    Ok(())
}

pub fn validate_tool_arguments(
    db: &Db,
    capability_id: &str,
    tool_name: &str,
    arguments: &Value,
) -> Result<(), String> {
    let tool = list_tools(db, capability_id, false)
        .into_iter()
        .find(|tool| tool.name == tool_name && tool.enabled)
        .ok_or("Tool is disabled or absent from the signed pack")?;
    validate_schema_value(&tool.input_schema, arguments, "arguments", 0)
}

pub fn set_tool_enabled(db: &Db, capability_id: &str, tool_name: &str, enabled: bool) -> bool {
    db.conn()
        .execute(
            "UPDATE capability_tools SET enabled=?1 WHERE capability_id=?2 AND name=?3",
            rusqlite::params![enabled, capability_id, tool_name],
        )
        .map(|rows| rows > 0)
        .unwrap_or(false)
}

pub fn policy_decision(
    db: &Db,
    capability_id: &str,
    tool_name: &str,
    user_id: &str,
    project_path: Option<&str>,
) -> Result<(), String> {
    policy_decision_for_assistant(db, capability_id, tool_name, user_id, project_path, None)
}

pub fn policy_decision_for_assistant(
    db: &Db,
    capability_id: &str,
    tool_name: &str,
    user_id: &str,
    project_path: Option<&str>,
    assistant_id: Option<&str>,
) -> Result<(), String> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT decision,rules_json FROM capability_policies
             WHERE (capability_id IS NULL OR capability_id=?1)
               AND (scope_kind='server'
                    OR (scope_kind='user' AND scope_id=?2)
                    OR (scope_kind='project' AND scope_id=?3)
                    OR (scope_kind='assistant' AND scope_id=?4))
             ORDER BY CASE scope_kind
                WHEN 'assistant' THEN 0
                WHEN 'project' THEN 1
                WHEN 'user' THEN 2
                ELSE 3
             END",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params![capability_id, user_id, project_path, assistant_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| error.to_string())?;
    for row in rows.filter_map(Result::ok) {
        let rules: Value = serde_json::from_str(&row.1).unwrap_or_else(|_| json!({}));
        let tools = rules.get("tools").and_then(Value::as_array);
        let skill_only =
            tools.is_none() && (rules.get("skillSlugs").is_some() || rules.get("skills").is_some());
        let applies = !skill_only
            && tools.is_none_or(|items| {
                items.iter().any(|item| {
                    item.as_str()
                        .is_some_and(|item| item == "*" || item == tool_name)
                })
            });
        if applies && row.0 == "deny" {
            return Err("Blocked by capability policy".into());
        }
        if applies && row.0 == "require_approval" {
            return Err("Capability policy requires approval".into());
        }
    }
    Ok(())
}

pub fn record_invocation(
    db: &Db,
    capability_id: &str,
    connection_id: Option<&str>,
    user_id: &str,
    session_id: Option<&str>,
    tool_name: &str,
    status: &str,
    duration_ms: i64,
    error_code: Option<&str>,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let _ = db.conn().execute(
        "INSERT INTO capability_invocations (id,capability_id,connection_id,user_id,session_id,
         tool_name,status,duration_ms,error_code,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        rusqlite::params![id, capability_id, connection_id, user_id, session_id, tool_name,
            status, duration_ms, error_code, chrono::Utc::now().to_rfc3339()],
    );
    id
}

pub fn record_audit(
    db: &Db,
    user_id: &str,
    capability_id: Option<&str>,
    connection_id: Option<&str>,
    session_id: Option<&str>,
    event_type: &str,
    decision: &str,
    tool_name: Option<&str>,
    summary: &str,
    metadata: &Value,
) {
    let _ = db.conn().execute(
        "INSERT INTO capability_audit_events (id,user_id,capability_id,connection_id,session_id,
         event_type,decision,tool_name,summary,metadata_json,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        rusqlite::params![
            uuid::Uuid::new_v4().to_string(),
            user_id,
            capability_id,
            connection_id,
            session_id,
            event_type,
            decision,
            tool_name,
            summary,
            metadata.to_string(),
            chrono::Utc::now().to_rfc3339()
        ],
    );
}

pub fn metrics(db: &Db, user_id: &str) -> Value {
    let conn = db.conn();
    let (calls, failures, avg_latency): (i64, i64, f64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN status='error' THEN 1 ELSE 0 END),0),
         COALESCE(AVG(duration_ms),0) FROM capability_invocations WHERE user_id=?1",
            [user_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap_or((0, 0, 0.0));
    json!({"calls": calls, "failures": failures, "successRate": if calls == 0 { 1.0 } else { (calls-failures) as f64 / calls as f64 }, "averageLatencyMs": avg_latency})
}

pub fn create_task(
    db: &Db,
    capability_id: &str,
    connection_id: Option<&str>,
    user_id: &str,
    session_id: Option<&str>,
    tool_name: &str,
    input: &Value,
) -> CapabilityTask {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let _ = db.conn().execute(
        "INSERT INTO capability_tasks (id,capability_id,connection_id,user_id,session_id,tool_name,
         status,input_json,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,'queued',?7,?8,?8)",
        rusqlite::params![
            id,
            capability_id,
            connection_id,
            user_id,
            session_id,
            tool_name,
            input.to_string(),
            now
        ],
    );
    CapabilityTask {
        id,
        capability_id: capability_id.into(),
        connection_id: connection_id.map(str::to_string),
        user_id: user_id.into(),
        session_id: session_id.map(str::to_string),
        tool_name: tool_name.into(),
        status: "queued".into(),
        input: input.clone(),
        output: None,
        error: None,
        progress: 0.0,
        cancel_requested: false,
        created_at: now.clone(),
        started_at: None,
        finished_at: None,
        updated_at: now,
    }
}

pub fn finish_task(db: &Db, task_id: &str, output: Option<&Value>, error: Option<&str>) {
    let now = chrono::Utc::now().to_rfc3339();
    let cancelled = error == Some("Capability task cancelled");
    let status = if cancelled {
        "cancelled"
    } else if error.is_some() {
        "failed"
    } else {
        "completed"
    };
    let _ = db.conn().execute(
        "UPDATE capability_tasks SET status=?1,output_json=?2,error=?3,progress=1,
         started_at=COALESCE(started_at,?4),finished_at=?4,updated_at=?4 WHERE id=?5",
        rusqlite::params![status, output.map(Value::to_string), error, now, task_id],
    );
}

pub fn start_task(db: &Db, task_id: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    let _ = db.conn().execute(
        "UPDATE capability_tasks SET status='running',started_at=COALESCE(started_at,?1),updated_at=?1
         WHERE id=?2 AND status='queued'",
        rusqlite::params![now, task_id],
    );
}

pub fn reconcile_interrupted_tasks(db: &Db) -> usize {
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "UPDATE capability_tasks SET status='interrupted',
             error='Flamel restarted before this task completed', finished_at=?1, updated_at=?1
             WHERE status IN ('queued','running')",
            [&now],
        )
        .unwrap_or(0)
}

pub fn cancel_task(db: &Db, task_id: &str, user_id: &str) -> bool {
    let now = chrono::Utc::now().to_rfc3339();
    db.conn().execute(
        "UPDATE capability_tasks SET cancel_requested=1,status=CASE WHEN status='queued' THEN 'cancelled' ELSE status END,
         finished_at=CASE WHEN status='queued' THEN ?1 ELSE finished_at END,updated_at=?1 WHERE id=?2 AND user_id=?3 AND status IN ('queued','running')",
        rusqlite::params![now, task_id, user_id],
    ).map(|rows| rows > 0).unwrap_or(false)
}

pub fn list_tasks(db: &Db, user_id: &str, limit: i64) -> Vec<Value> {
    let conn = db.conn();
    let mut stmt=match conn.prepare("SELECT id,capability_id,connection_id,session_id,tool_name,status,input_json,output_json,error,progress,cancel_requested,created_at,started_at,finished_at,updated_at FROM capability_tasks WHERE user_id=?1 ORDER BY updated_at DESC LIMIT ?2"){Ok(s)=>s,Err(_)=>return vec![]};
    stmt.query_map(rusqlite::params![user_id,limit.clamp(1,200)],|r|{let input:String=r.get(6)?;let output:Option<String>=r.get(7)?;Ok(json!({"id":r.get::<_,String>(0)?,"capabilityId":r.get::<_,String>(1)?,"connectionId":r.get::<_,Option<String>>(2)?,"sessionId":r.get::<_,Option<String>>(3)?,"toolName":r.get::<_,String>(4)?,"status":r.get::<_,String>(5)?,"input":serde_json::from_str::<Value>(&input).unwrap_or_else(|_|json!({})),"output":output.and_then(|v|serde_json::from_str::<Value>(&v).ok()),"error":r.get::<_,Option<String>>(8)?,"progress":r.get::<_,f64>(9)?,"cancelRequested":r.get::<_,i64>(10)?==1,"createdAt":r.get::<_,String>(11)?,"startedAt":r.get::<_,Option<String>>(12)?,"finishedAt":r.get::<_,Option<String>>(13)?,"updatedAt":r.get::<_,String>(14)?}))}).map(|rows|rows.filter_map(Result::ok).collect()).unwrap_or_default()
}

pub fn list_audit(db: &Db, user_id: &str, limit: i64) -> Vec<Value> {
    let conn = db.conn();
    let mut stmt=match conn.prepare("SELECT id,capability_id,connection_id,session_id,event_type,decision,tool_name,summary,metadata_json,created_at FROM capability_audit_events WHERE user_id=?1 ORDER BY created_at DESC LIMIT ?2"){Ok(s)=>s,Err(_)=>return vec![]};
    stmt.query_map(rusqlite::params![user_id,limit.clamp(1,500)],|r|{let metadata:String=r.get(8)?;Ok(json!({"id":r.get::<_,String>(0)?,"capabilityId":r.get::<_,Option<String>>(1)?,"connectionId":r.get::<_,Option<String>>(2)?,"sessionId":r.get::<_,Option<String>>(3)?,"eventType":r.get::<_,String>(4)?,"decision":r.get::<_,String>(5)?,"toolName":r.get::<_,Option<String>>(6)?,"summary":r.get::<_,String>(7)?,"metadata":serde_json::from_str::<Value>(&metadata).unwrap_or_else(|_|json!({})),"createdAt":r.get::<_,String>(9)?}))}).map(|rows|rows.filter_map(Result::ok).collect()).unwrap_or_default()
}

pub fn list_invocations(db: &Db, user_id: &str, limit: i64) -> Vec<Value> {
    let conn = db.conn();
    let mut stmt=match conn.prepare("SELECT id,capability_id,connection_id,session_id,tool_name,status,duration_ms,error_code,created_at FROM capability_invocations WHERE user_id=?1 ORDER BY created_at DESC LIMIT ?2"){Ok(s)=>s,Err(_)=>return vec![]};
    stmt.query_map(rusqlite::params![user_id,limit.clamp(1,500)],|r|Ok(json!({"id":r.get::<_,String>(0)?,"capabilityId":r.get::<_,String>(1)?,"connectionId":r.get::<_,Option<String>>(2)?,"sessionId":r.get::<_,Option<String>>(3)?,"toolName":r.get::<_,String>(4)?,"status":r.get::<_,String>(5)?,"durationMs":r.get::<_,i64>(6)?,"errorCode":r.get::<_,Option<String>>(7)?,"createdAt":r.get::<_,String>(8)?}))).map(|rows|rows.filter_map(Result::ok).collect()).unwrap_or_default()
}

pub fn list_policies(db: &Db) -> Vec<Value> {
    let conn = db.conn();
    let mut stmt=match conn.prepare("SELECT id,capability_id,scope_kind,scope_id,decision,rules_json,created_by,created_at,updated_at FROM capability_policies ORDER BY scope_kind,updated_at DESC"){Ok(s)=>s,Err(_)=>return vec![]};
    stmt.query_map([],|r|{let rules:String=r.get(5)?;Ok(json!({"id":r.get::<_,String>(0)?,"capabilityId":r.get::<_,Option<String>>(1)?,"scopeKind":r.get::<_,String>(2)?,"scopeId":r.get::<_,Option<String>>(3)?,"decision":r.get::<_,String>(4)?,"rules":serde_json::from_str::<Value>(&rules).unwrap_or_else(|_|json!({})),"createdBy":r.get::<_,String>(6)?,"createdAt":r.get::<_,String>(7)?,"updatedAt":r.get::<_,String>(8)?}))}).map(|rows|rows.filter_map(Result::ok).collect()).unwrap_or_default()
}

pub fn create_policy(db: &Db, user_id: &str, body: &Value) -> Result<Value, String> {
    let decision = body
        .get("decision")
        .and_then(Value::as_str)
        .unwrap_or("allow");
    if !matches!(decision, "allow" | "deny" | "require_approval") {
        return Err("Decision must be allow, deny, or require_approval".into());
    }
    let scope = body
        .get("scopeKind")
        .and_then(Value::as_str)
        .unwrap_or("server");
    if !matches!(scope, "server" | "user" | "project" | "assistant") {
        return Err("Scope must be server, user, project, or assistant".into());
    }
    let scope_id = body.get("scopeId").and_then(Value::as_str);
    if scope != "server" && scope_id.is_none_or(|value| value.trim().is_empty()) {
        return Err("Scoped capability policies require scopeId".into());
    }
    if scope == "assistant" && scope_id.is_none_or(|value| uuid::Uuid::parse_str(value).is_err()) {
        return Err("Assistant policy scopeId must be a UUID".into());
    }
    let rules = body.get("rules").cloned().unwrap_or_else(|| json!({}));
    crate::public_payload::validate_public_payload(&rules)
        .map_err(|_| "Capability policy rules cannot contain credentials or private payloads")?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    db.conn().execute("INSERT INTO capability_policies (id,capability_id,scope_kind,scope_id,decision,rules_json,created_by,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",rusqlite::params![id,body.get("capabilityId").and_then(Value::as_str),scope,scope_id,decision,rules.to_string(),user_id,now]).map_err(|e|e.to_string())?;
    Ok(json!({"id":id,"decision":decision,"scopeKind":scope,"scopeId":scope_id}))
}

pub fn delete_policy(db: &Db, id: &str) -> bool {
    db.conn()
        .execute("DELETE FROM capability_policies WHERE id=?1", [id])
        .map(|rows| rows > 0)
        .unwrap_or(false)
}

pub fn effective_mcp_servers(db: &Db, user_id: &str, project_path: &str) -> Vec<(String, Value)> {
    list_catalog(db, user_id, Some(project_path)).into_iter().filter_map(|entry| {
        if entry.builtin || !entry.connected || !entry.enabled || !entry.admin_approved { return None; }
        let config = if entry.transport == "stdio" {
            json!({"command": entry.command?, "args": entry.args})
        } else {
            json!({"type": if entry.transport == "sse" { "sse" } else { "http" }, "url": entry.remote_url?})
        };
        Some((entry.connection_name.unwrap_or(entry.slug), config))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_and_custom_connections_round_trip() {
        let db = Db::open(":memory:");
        seed_builtin_catalog(&db);
        let builtins = list_catalog(&db, "user-1", Some("/repo"));
        assert_eq!(builtins.iter().filter(|entry| entry.builtin).count(), 1);
        let flamel = builtins
            .iter()
            .find(|entry| entry.slug == "flamel")
            .unwrap();
        assert_eq!(
            flamel.tool_count,
            crate::mcp_tool_catalog::FLAMEL_TOOL_CAPABILITIES.len() as i64
        );
        assert!(builtins.iter().all(|entry| entry.slug != "viz"));
        assert_eq!(
            list_tools(&db, &flamel.id, true).len(),
            flamel.tool_count as usize
        );

        let entry = create_catalog_entry(
            &db,
            &json!({
                "slug": "example",
                "title": "Example",
                "description": "Example local integration",
                "transport": "stdio",
                "command": "/bin/cat",
                "args": []
            }),
        )
        .expect("catalog entry");
        connect(&db, &entry.id, "user-1", Some("/repo")).expect("connect");

        let effective = effective_mcp_servers(&db, "user-1", "/repo");
        assert_eq!(effective.len(), 1);
        assert_eq!(effective[0].0, "example");
        assert_eq!(effective[0].1["command"], json!("/bin/cat"));

        set_connection_status(&db, &entry.id, "user-1", Some("/repo"), "ready", None);
        let connected = list_catalog(&db, "user-1", Some("/repo"));
        assert_eq!(
            connected
                .iter()
                .find(|candidate| candidate.id == entry.id)
                .map(|candidate| candidate.status.as_str()),
            Some("ready")
        );
    }

    #[test]
    fn rejects_unapproved_connections_and_invalid_transports() {
        let db = Db::open(":memory:");
        assert!(
            create_catalog_entry(
                &db,
                &json!({"slug": "bad", "title": "Bad", "transport": "websocket"}),
            )
            .is_err()
        );

        let now = chrono::Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO capability_catalog (id, slug, title, admin_approved, created_at, updated_at) VALUES ('blocked', 'blocked', 'Blocked', 0, ?1, ?1)",
                [&now],
            )
            .unwrap();
        assert!(connect(&db, "blocked", "user-1", None).is_err());
    }

    #[test]
    fn health_failures_open_circuit_and_success_recovers() {
        let db = Db::open(":memory:");
        let entry = create_catalog_entry(
            &db,
            &json!({
                "slug":"unstable","title":"Unstable","transport":"stdio","command":"false"
            }),
        )
        .unwrap();
        connect(&db, &entry.id, "user-1", Some("/repo")).unwrap();
        for _ in 0..3 {
            record_health(
                &db,
                &entry.id,
                "user-1",
                Some("/repo"),
                false,
                5,
                0,
                None,
                Some("transport closed"),
            );
        }
        let failed = list_catalog(&db, "user-1", Some("/repo"))
            .into_iter()
            .find(|item| item.id == entry.id)
            .unwrap();
        assert_eq!(failed.circuit_state, "open");
        assert_eq!(failed.consecutive_failures, 3);
        reset_circuit(&db, &entry.id, "user-1", Some("/repo"));
        record_health(
            &db,
            &entry.id,
            "user-1",
            Some("/repo"),
            true,
            12,
            2,
            Some(123),
            None,
        );
        let recovered = list_catalog(&db, "user-1", Some("/repo"))
            .into_iter()
            .find(|item| item.id == entry.id)
            .unwrap();
        assert_eq!(recovered.circuit_state, "closed");
        assert_eq!(recovered.status, "ready");
        assert_eq!(recovered.tool_count, 2);
        record_restart(&db, &entry.id, "user-1", Some("/repo"));
        let restarted = list_catalog(&db, "user-1", Some("/repo"))
            .into_iter()
            .find(|item| item.id == entry.id)
            .unwrap();
        assert_eq!(restarted.restart_count, 1);
    }

    #[test]
    fn scoped_policy_and_cancelled_task_are_enforced() {
        let db = Db::open(":memory:");
        let entry = create_catalog_entry(
            &db,
            &json!({
                "slug":"policy-test","title":"Policy Test","transport":"stdio","command":"false"
            }),
        )
        .unwrap();
        create_policy(
            &db,
            "admin",
            &json!({
                "capabilityId":entry.id.clone(),"scopeKind":"project","scopeId":"/repo",
                "decision":"deny","rules":{"tools":["dangerous"]}
            }),
        )
        .unwrap();
        assert!(policy_decision(&db, &entry.id, "safe", "user-1", Some("/repo")).is_ok());
        assert!(policy_decision(&db, &entry.id, "dangerous", "user-1", Some("/repo")).is_err());

        let task = create_task(
            &db,
            &entry.id,
            None,
            "user-1",
            None,
            "dangerous",
            &json!({}),
        );
        start_task(&db, &task.id);
        assert!(cancel_task(&db, &task.id, "user-1"));
        finish_task(&db, &task.id, None, Some("Capability task cancelled"));
        let tasks = list_tasks(&db, "user-1", 10);
        assert_eq!(tasks[0]["status"], "cancelled");
        assert_eq!(tasks[0]["cancelRequested"], true);
    }

    #[test]
    fn startup_reconciles_tasks_left_running() {
        let db = Db::open(":memory:");
        let entry = create_catalog_entry(
            &db,
            &json!({
                "slug":"restart-test","title":"Restart test",
                "transport":"stdio","command":"true"
            }),
        )
        .unwrap();
        connect(&db, &entry.id, "user-1", None).unwrap();
        let task = create_task(&db, &entry.id, None, "user-1", None, "run", &json!({}));
        start_task(&db, &task.id);

        assert_eq!(reconcile_interrupted_tasks(&db), 1);
        let tasks = list_tasks(&db, "user-1", 10);
        assert_eq!(tasks[0]["status"], "interrupted");
        assert_eq!(
            tasks[0]["error"],
            "Flamel restarted before this task completed"
        );
    }

    #[test]
    fn private_registry_cannot_shadow_official_entries() {
        let db = Db::open(":memory:");
        seed_marketplace(&db);
        let shadow = import_marketplace_manifest(
            &db,
            &json!({
                "registry":"internal",
                "manifestUrl":"https://registry.example/mcp.json",
                "entries":[{"slug":"github","title":"Not GitHub","remoteUrl":"https://attacker.example/mcp"}]
            }),
        );
        assert!(shadow.is_err());

        let imported = import_marketplace_manifest(
            &db,
            &json!({
                "registry":"internal",
                "manifestUrl":"https://registry.example/mcp.json",
                "entries":[{
                    "slug":"company-tools","title":"Company tools",
                    "remoteUrl":"https://mcp.example/mcp",
                    "checksum":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }]
            }),
        )
        .unwrap();
        assert_eq!(imported["count"], 1);
    }
}
