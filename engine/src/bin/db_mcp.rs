//! flamel DB MCP — stdio JSON-RPC server for read-only database exploration.
//! Supports PostgreSQL (via psql), MySQL (via mysql CLI), and SQLite (via rusqlite).
//! No extra Rust database driver crates needed — pg/mysql use the CLI tools.

use flamel::mcp_args::normalize_tool_arguments;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

fn main() {
    flamel::mcp_parent_guard::install();
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

        let response = handle_request(&req);
        if let Some(resp) = response {
            let mut out = stdout.lock();
            let _ = serde_json::to_writer(&mut out, &resp);
            let _ = out.write_all(b"\n");
            let _ = out.flush();
        }
    }
}

fn handle_request(req: &Value) -> Option<Value> {
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
                "serverInfo": {"name": "db_mcp", "version": "0.1.0"},
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
                    "result": {
                        "content": [{"type": "text", "text": format!("Error: {e}")}],
                        "isError": true,
                    }
                }));
            }
        };

        match handle_tool_call(name, &args) {
            Ok(text) => {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": text}],
                    }
                }));
            }
            Err(e) => {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": e}],
                        "isError": true,
                    }
                }));
            }
        }
    }

    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32601, "message": "Method not found"}
    }))
}

fn tools_list() -> Value {
    json!([
        {
            "name": "db",
            "description": "Read-only database exploration. Actions: list_tables (default) — show all tables (params: connection_string, schema?); describe — show table schema (params: connection_string, table); query — run SELECT query (params: connection_string, query). Supports SQLite (sqlite:path, sqlite:///path, or a database file path), PostgreSQL (postgres://...), MySQL (mysql://...). Connection strings can reference env vars with $ prefix (e.g. $DATABASE_URL). Legacy: list_tables, describe_table, run_query.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["list_tables", "describe", "query"], "description": "DB action (default: list_tables)"},
                    "connection_string": {"type": "string", "description": "Database connection string (sqlite:path, sqlite:///path, a .db/.sqlite file path, postgres://..., mysql://..., or $ENV_VAR)"},
                    "table": {"type": "string", "description": "Table name (for describe)"},
                    "query": {"type": "string", "description": "SQL SELECT query (for query)"},
                    "schema": {"type": "string", "description": "Schema filter (for list_tables)"}
                },
                "required": ["connection_string"]
            }
        }
    ])
}

fn handle_tool_call(name: &str, args: &Value) -> Result<String, String> {
    let resolved: String;
    let name = if name == "db" {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list_tables");
        resolved = action.to_string();
        resolved.as_str()
    } else {
        name
    };

    let conn_str = resolve_connection_string(args)?;
    let db_type = detect_db_type(&conn_str)?;

    match name {
        "list_tables" => {
            let schema = args.get("schema").and_then(|v| v.as_str());
            list_tables(&db_type, &conn_str, schema)
        }
        "describe_table" | "describe" => {
            let table = args
                .get("table")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required parameter: table".to_string())?;
            describe_table(&db_type, &conn_str, table)
        }
        "run_query" | "query" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required parameter: query".to_string())?;
            run_query(&db_type, &conn_str, query)
        }
        _ => Err(format!("Unknown tool: {name}")),
    }
}

// ---------------------------------------------------------------------------
// Connection string resolution
// ---------------------------------------------------------------------------

fn resolve_connection_string(args: &Value) -> Result<String, String> {
    if let Some(cs) = args.get("connection_string").and_then(|v| v.as_str()) {
        if !cs.is_empty() {
            return Ok(cs.to_string());
        }
    }
    std::env::var("DATABASE_URL").map_err(|_| {
        "No connection_string provided and DATABASE_URL env var not set. \
         Provide a connection_string parameter, e.g.:\n\
         - sqlite:///path/to/db.sqlite\n\
         - postgres://user:pass@localhost:5432/mydb\n\
         - mysql://user:pass@localhost:3306/mydb"
            .to_string()
    })
}

#[derive(Debug)]
enum DbType {
    Sqlite(String),
    Postgres(String),
    Mysql(String),
}

fn detect_db_type(conn_str: &str) -> Result<DbType, String> {
    if conn_str.starts_with("postgres://") || conn_str.starts_with("postgresql://") {
        Ok(DbType::Postgres(conn_str.to_string()))
    } else if conn_str.starts_with("mysql://") {
        Ok(DbType::Mysql(conn_str.to_string()))
    } else if let Some(raw_path) = conn_str.strip_prefix("sqlite:") {
        let path = if let Some(path) = raw_path.strip_prefix("///") {
            format!("/{path}")
        } else if let Some(path) = raw_path.strip_prefix("//") {
            path.to_string()
        } else {
            raw_path.to_string()
        };
        Ok(DbType::Sqlite(path))
    } else if conn_str.ends_with(".db")
        || conn_str.ends_with(".sqlite")
        || conn_str.ends_with(".sqlite3")
    {
        Ok(DbType::Sqlite(conn_str.to_string()))
    } else {
        Err(format!(
            "Cannot detect database type from: {conn_str}\n\
                     Expected prefix: postgres://, mysql://, sqlite: or a path ending in .db/.sqlite"
        ))
    }
}

// ---------------------------------------------------------------------------
// SQLite (native via rusqlite — already a dependency)
// ---------------------------------------------------------------------------

fn sqlite_open(path: &str) -> Result<rusqlite::Connection, String> {
    rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Failed to open SQLite database '{path}': {e}"))
}

fn sqlite_list_tables(path: &str) -> Result<String, String> {
    let conn = sqlite_open(path)?;
    let mut stmt = conn.prepare(
        "SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY type, name"
    ).map_err(|e| format!("Query error: {e}"))?;

    let mut rows: Vec<String> = Vec::new();
    rows.push(format!("{:<40} {:<10} {}", "Name", "Type", "Rows"));
    rows.push("-".repeat(60));

    let mapped = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let typ: String = row.get(1)?;
            Ok((name, typ))
        })
        .map_err(|e| format!("Query error: {e}"))?;

    for r in mapped {
        let (name, typ) = r.map_err(|e| format!("Row error: {e}"))?;
        let count = if typ == "table" {
            conn.query_row(
                &format!("SELECT COUNT(*) FROM \"{}\"", name.replace('"', "\"\"")),
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c.to_string())
            .unwrap_or_else(|_| "?".to_string())
        } else {
            "-".to_string()
        };
        rows.push(format!("{:<40} {:<10} {}", name, typ, count));
    }

    Ok(rows.join("\n"))
}

fn sqlite_describe_table(path: &str, table: &str) -> Result<String, String> {
    let conn = sqlite_open(path)?;
    let safe_table = table.replace('\'', "''");

    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info('{safe_table}')"))
        .map_err(|e| format!("Query error: {e}"))?;

    let mut rows: Vec<String> = Vec::new();
    rows.push(format!("Table: {table}\n"));
    rows.push(format!(
        "{:<5} {:<30} {:<20} {:<8} {:<20} {}",
        "CID", "Name", "Type", "NotNull", "Default", "PK"
    ));
    rows.push("-".repeat(95));

    let mapped = stmt
        .query_map([], |row| {
            let cid: i32 = row.get(0)?;
            let name: String = row.get(1)?;
            let typ: String = row.get(2)?;
            let notnull: bool = row.get(3)?;
            let dflt: Option<String> = row.get(4)?;
            let pk: i32 = row.get(5)?;
            Ok((cid, name, typ, notnull, dflt, pk))
        })
        .map_err(|e| format!("Query error: {e}"))?;

    for r in mapped {
        let (cid, name, typ, notnull, dflt, pk) = r.map_err(|e| format!("Row error: {e}"))?;
        let nn = if notnull { "YES" } else { "NO" };
        let d = dflt.unwrap_or_else(|| "NULL".to_string());
        let p = if pk > 0 {
            format!("PK{pk}")
        } else {
            String::new()
        };
        rows.push(format!(
            "{:<5} {:<30} {:<20} {:<8} {:<20} {}",
            cid, name, typ, nn, d, p
        ));
    }

    // Foreign keys
    let mut fk_stmt = conn
        .prepare(&format!("PRAGMA foreign_key_list('{safe_table}')"))
        .map_err(|e| format!("FK query error: {e}"))?;
    let fks: Vec<String> = fk_stmt
        .query_map([], |row| {
            let from: String = row.get(3)?;
            let to_table: String = row.get(2)?;
            let to_col: String = row.get(4)?;
            Ok(format!("  {from} -> {to_table}.{to_col}"))
        })
        .map_err(|e| format!("FK error: {e}"))?
        .filter_map(|r| r.ok())
        .collect();

    if !fks.is_empty() {
        rows.push(String::new());
        rows.push("Foreign Keys:".to_string());
        rows.extend(fks);
    }

    // Indexes
    let mut idx_stmt = conn
        .prepare(&format!("PRAGMA index_list('{safe_table}')"))
        .map_err(|e| format!("Index query error: {e}"))?;
    let idxs: Vec<String> = idx_stmt
        .query_map([], |row| {
            let name: String = row.get(1)?;
            let unique: bool = row.get(2)?;
            Ok(format!(
                "  {} {}",
                name,
                if unique { "(unique)" } else { "" }
            ))
        })
        .map_err(|e| format!("Index error: {e}"))?
        .filter_map(|r| r.ok())
        .collect();

    if !idxs.is_empty() {
        rows.push(String::new());
        rows.push("Indexes:".to_string());
        rows.extend(idxs);
    }

    Ok(rows.join("\n"))
}

fn sqlite_run_query(path: &str, query: &str) -> Result<String, String> {
    validate_readonly_query(query)?;
    let conn = sqlite_open(path)?;
    let mut stmt = conn.prepare(query).map_err(|e| format!("SQL error: {e}"))?;

    let col_count = stmt.column_count();
    let col_names: Vec<String> = (0..col_count)
        .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
        .collect();

    let mut result_rows: Vec<Vec<String>> = Vec::new();
    let mut qrows = stmt.query([]).map_err(|e| format!("Query error: {e}"))?;

    let max_rows = 100;
    let mut count = 0;
    while let Some(row) = qrows.next().map_err(|e| format!("Row error: {e}"))? {
        if count >= max_rows {
            break;
        }
        let mut vals: Vec<String> = Vec::new();
        for i in 0..col_count {
            let val: String = match row.get_ref(i) {
                Ok(rusqlite::types::ValueRef::Null) => "NULL".to_string(),
                Ok(rusqlite::types::ValueRef::Integer(n)) => n.to_string(),
                Ok(rusqlite::types::ValueRef::Real(f)) => f.to_string(),
                Ok(rusqlite::types::ValueRef::Text(s)) => {
                    let s = String::from_utf8_lossy(s);
                    if s.chars().count() > 200 {
                        format!("{}...", s.chars().take(200).collect::<String>())
                    } else {
                        s.to_string()
                    }
                }
                Ok(rusqlite::types::ValueRef::Blob(b)) => format!("<blob {} bytes>", b.len()),
                Err(_) => "?".to_string(),
            };
            vals.push(val);
        }
        result_rows.push(vals);
        count += 1;
    }

    Ok(format_table(&col_names, &result_rows, count >= max_rows))
}

// ---------------------------------------------------------------------------
// PostgreSQL (via psql CLI — no extra deps)
// ---------------------------------------------------------------------------

fn pg_run_command(conn_str: &str, sql: &str) -> Result<String, String> {
    let output = flamel::mcp_parent_guard::command("psql")
        .arg("--no-psqlrc")
        .arg("-c")
        .arg(sql)
        .arg("--pset=border=2")
        .arg("--pset=format=aligned")
        .arg("--set=ON_ERROR_STOP=1")
        .env("PGDATABASE", conn_str)
        .env("PGCONNECT_TIMEOUT", "5")
        .output()
        .map_err(|e| format!("Failed to run psql (is it installed?): {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("psql error: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn pg_list_tables(conn_str: &str, schema: Option<&str>) -> Result<String, String> {
    let schema = schema.unwrap_or("public");
    let sql = format!(
        "SELECT t.table_name, t.table_type, \
         COALESCE((SELECT reltuples::bigint FROM pg_class c JOIN pg_namespace n ON c.relnamespace = n.oid \
         WHERE c.relname = t.table_name AND n.nspname = '{schema}'), -1) AS approx_rows \
         FROM information_schema.tables t \
         WHERE t.table_schema = '{schema}' \
         ORDER BY t.table_type, t.table_name;"
    );
    pg_run_command(conn_str, &sql)
}

fn pg_describe_table(conn_str: &str, table: &str) -> Result<String, String> {
    let safe = table.replace('\'', "''");
    let sql = format!(
        "SELECT c.column_name, c.data_type, c.is_nullable, c.column_default, \
         CASE WHEN tc.constraint_type = 'PRIMARY KEY' THEN 'PK' ELSE '' END AS pk \
         FROM information_schema.columns c \
         LEFT JOIN information_schema.key_column_usage kcu \
           ON c.column_name = kcu.column_name AND c.table_name = kcu.table_name \
         LEFT JOIN information_schema.table_constraints tc \
           ON kcu.constraint_name = tc.constraint_name AND tc.constraint_type = 'PRIMARY KEY' \
         WHERE c.table_name = '{safe}' \
         ORDER BY c.ordinal_position;"
    );
    let mut result = format!("Table: {table}\n\n");
    result.push_str(&pg_run_command(conn_str, &sql)?);

    let fk_sql = format!(
        "SELECT kcu.column_name, ccu.table_name AS foreign_table, ccu.column_name AS foreign_column \
         FROM information_schema.table_constraints tc \
         JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name \
         JOIN information_schema.constraint_column_usage ccu ON tc.constraint_name = ccu.constraint_name \
         WHERE tc.table_name = '{safe}' AND tc.constraint_type = 'FOREIGN KEY';"
    );
    if let Ok(fks) = pg_run_command(conn_str, &fk_sql) {
        if !fks.trim().is_empty() && !fks.contains("(0 rows)") {
            result.push_str("\nForeign Keys:\n");
            result.push_str(&fks);
        }
    }

    let idx_sql = format!(
        "SELECT indexname, indexdef FROM pg_indexes WHERE tablename = '{safe}' ORDER BY indexname;"
    );
    if let Ok(idxs) = pg_run_command(conn_str, &idx_sql) {
        if !idxs.trim().is_empty() && !idxs.contains("(0 rows)") {
            result.push_str("\nIndexes:\n");
            result.push_str(&idxs);
        }
    }

    Ok(result)
}

fn pg_run_query(conn_str: &str, query: &str) -> Result<String, String> {
    validate_readonly_query(query)?;
    let sql = format!("BEGIN READ ONLY; {query}; ROLLBACK;");
    pg_run_command(conn_str, &sql)
}

// ---------------------------------------------------------------------------
// MySQL (via mysql CLI — no extra deps)
// ---------------------------------------------------------------------------

fn parse_mysql_url(conn_str: &str) -> Result<(String, String, String, String, String), String> {
    let rest = conn_str
        .strip_prefix("mysql://")
        .ok_or("Invalid MySQL URL")?;
    let (userinfo, hostpath) = rest.split_once('@').ok_or("Missing @ in MySQL URL")?;
    let (user, pass) = userinfo.split_once(':').unwrap_or((userinfo, ""));
    let (hostport, db) = hostpath
        .split_once('/')
        .ok_or("Missing /dbname in MySQL URL")?;
    let (host, port) = hostport.split_once(':').unwrap_or((hostport, "3306"));
    Ok((
        user.to_string(),
        pass.to_string(),
        host.to_string(),
        port.to_string(),
        db.to_string(),
    ))
}

fn mysql_run_command(conn_str: &str, sql: &str) -> Result<String, String> {
    let (user, pass, host, port, db) = parse_mysql_url(conn_str)?;
    let mut cmd = flamel::mcp_parent_guard::command("mysql");
    cmd.arg("-h")
        .arg(&host)
        .arg("-P")
        .arg(&port)
        .arg("-u")
        .arg(&user)
        .arg(&db)
        .arg("-e")
        .arg(sql)
        .arg("--table")
        .env("MYSQL_CONNECT_TIMEOUT", "5");

    if !pass.is_empty() {
        // Keep credentials out of the process argument list.
        cmd.env("MYSQL_PWD", pass);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run mysql client (is it installed?): {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("mysql error: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn mysql_list_tables(conn_str: &str, schema: Option<&str>) -> Result<String, String> {
    let (_, _, _, _, db) = parse_mysql_url(conn_str)?;
    let db_name = schema.unwrap_or(&db);
    let sql = format!(
        "SELECT TABLE_NAME, TABLE_TYPE, TABLE_ROWS \
         FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = '{}' \
         ORDER BY TABLE_TYPE, TABLE_NAME",
        db_name.replace('\'', "''")
    );
    mysql_run_command(conn_str, &sql)
}

fn mysql_describe_table(conn_str: &str, table: &str) -> Result<String, String> {
    let safe = table.replace('\'', "''");
    let mut result = format!("Table: {table}\n\n");
    result.push_str(&mysql_run_command(conn_str, &format!("DESCRIBE `{safe}`"))?);

    let (_, _, _, _, db) = parse_mysql_url(conn_str)?;
    let fk_sql = format!(
        "SELECT COLUMN_NAME, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
         FROM information_schema.KEY_COLUMN_USAGE \
         WHERE TABLE_SCHEMA = '{}' AND TABLE_NAME = '{safe}' AND REFERENCED_TABLE_NAME IS NOT NULL",
        db.replace('\'', "''")
    );
    if let Ok(fks) = mysql_run_command(conn_str, &fk_sql) {
        if !fks.trim().is_empty() {
            result.push_str("\nForeign Keys:\n");
            result.push_str(&fks);
        }
    }

    if let Ok(idx) = mysql_run_command(conn_str, &format!("SHOW INDEX FROM `{safe}`")) {
        if !idx.trim().is_empty() {
            result.push_str("\nIndexes:\n");
            result.push_str(&idx);
        }
    }

    Ok(result)
}

fn mysql_run_query(conn_str: &str, query: &str) -> Result<String, String> {
    validate_readonly_query(query)?;
    mysql_run_command(
        conn_str,
        &format!("START TRANSACTION READ ONLY; {query}; ROLLBACK;"),
    )
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn list_tables(db: &DbType, conn_str: &str, schema: Option<&str>) -> Result<String, String> {
    match db {
        DbType::Sqlite(path) => sqlite_list_tables(path),
        DbType::Postgres(_) => pg_list_tables(conn_str, schema),
        DbType::Mysql(_) => mysql_list_tables(conn_str, schema),
    }
}

fn describe_table(db: &DbType, conn_str: &str, table: &str) -> Result<String, String> {
    match db {
        DbType::Sqlite(path) => sqlite_describe_table(path, table),
        DbType::Postgres(_) => pg_describe_table(conn_str, table),
        DbType::Mysql(_) => mysql_describe_table(conn_str, table),
    }
}

fn run_query(db: &DbType, conn_str: &str, query: &str) -> Result<String, String> {
    match db {
        DbType::Sqlite(path) => sqlite_run_query(path, query),
        DbType::Postgres(_) => pg_run_query(conn_str, query),
        DbType::Mysql(_) => mysql_run_query(conn_str, query),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn validate_readonly_query(query: &str) -> Result<(), String> {
    let upper = query.trim().to_uppercase();
    let allowed_starts = ["SELECT", "EXPLAIN", "PRAGMA", "SHOW", "DESCRIBE", "WITH"];
    if !allowed_starts.iter().any(|s| upper.starts_with(s)) {
        return Err(format!(
            "Only read-only queries are allowed (SELECT, EXPLAIN, PRAGMA, SHOW, DESCRIBE). \
             Got: {}...",
            query.chars().take(50).collect::<String>()
        ));
    }

    let dangerous = [
        "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE", "TRUNCATE", "GRANT", "REVOKE",
        "ATTACH", "DETACH", "VACUUM", "REINDEX", "ANALYZE", "CALL", "EXECUTE", "COPY", "LOAD",
        "LOCK", "UNLOCK", "SET",
    ];
    for kw in dangerous {
        if let Some(idx) = upper.find(kw) {
            let before_ok = idx == 0 || !upper.as_bytes()[idx - 1].is_ascii_alphanumeric();
            let after_idx = idx + kw.len();
            let after_ok =
                after_idx >= upper.len() || !upper.as_bytes()[after_idx].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return Err(format!(
                    "Mutation keyword '{kw}' detected. Only read-only queries are allowed."
                ));
            }
        }
    }

    if upper.starts_with("PRAGMA") {
        let pragma = upper
            .trim_start_matches("PRAGMA")
            .trim_start()
            .split(['(', '=', ';', ' ', '\t', '\r', '\n'])
            .next()
            .unwrap_or_default()
            .rsplit('.')
            .next()
            .unwrap_or_default();
        const READ_ONLY_PRAGMAS: &[&str] = &[
            "TABLE_INFO",
            "TABLE_XINFO",
            "FOREIGN_KEY_LIST",
            "INDEX_LIST",
            "INDEX_INFO",
            "INDEX_XINFO",
            "DATABASE_LIST",
            "COLLATION_LIST",
            "COMPILE_OPTIONS",
        ];
        if !READ_ONLY_PRAGMAS.contains(&pragma) {
            return Err(format!(
                "PRAGMA {pragma} is not in the read-only metadata allowlist."
            ));
        }
    }

    Ok(())
}

fn format_table(headers: &[String], rows: &[Vec<String>], truncated: bool) -> String {
    if rows.is_empty() {
        return format!("{}\n(0 rows)", headers.join(" | "));
    }

    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, val) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(val.len().min(60));
            }
        }
    }

    let mut out = String::new();

    let header_line: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            format!(
                "{:<width$}",
                h,
                width = widths.get(i).copied().unwrap_or(10)
            )
        })
        .collect();
    out.push_str(&header_line.join(" | "));
    out.push('\n');

    let sep: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    out.push_str(&sep.join("-+-"));
    out.push('\n');

    let mut total_len = 0;
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let w = widths.get(i).copied().unwrap_or(10);
                let display = if v.chars().count() > 60 {
                    format!("{}...", v.chars().take(57).collect::<String>())
                } else {
                    v.clone()
                };
                format!("{:<width$}", display, width = w)
            })
            .collect();
        let formatted = line.join(" | ");
        total_len += formatted.len();
        out.push_str(&formatted);
        out.push('\n');

        if total_len > 200_000 {
            out.push_str("... (output truncated at 200KB)\n");
            break;
        }
    }

    let row_count = rows.len();
    if truncated {
        out.push_str(&format!("({row_count} rows, truncated at limit)"));
    } else {
        out.push_str(&format!("({row_count} rows)"));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_connection_strings_accept_documented_forms() {
        for (connection, expected) in [
            ("sqlite:relative.db", "relative.db"),
            ("sqlite:/tmp/test.db", "/tmp/test.db"),
            ("sqlite:///tmp/test.db", "/tmp/test.db"),
            ("sqlite://relative.db", "relative.db"),
            ("/tmp/test.sqlite", "/tmp/test.sqlite"),
        ] {
            match detect_db_type(connection).expect("SQLite connection should parse") {
                DbType::Sqlite(path) => assert_eq!(path, expected),
                other => panic!("expected SQLite for {connection}, got {other:?}"),
            }
        }
    }

    #[test]
    fn readonly_validation_blocks_mutations_and_unsafe_pragmas() {
        for query in [
            "DELETE FROM users",
            "WITH rows AS (SELECT 1) UPDATE users SET admin = 1",
            "ATTACH DATABASE '/tmp/other.db' AS other",
            "PRAGMA writable_schema = ON",
            "PRAGMA journal_mode = WAL",
            "SELECT 1; VACUUM",
        ] {
            assert!(
                validate_readonly_query(query).is_err(),
                "unsafe query was accepted: {query}"
            );
        }
    }

    #[test]
    fn readonly_validation_accepts_metadata_and_select_queries() {
        for query in [
            "SELECT * FROM users LIMIT 10",
            "WITH rows AS (SELECT 1 AS id) SELECT * FROM rows",
            "PRAGMA table_info('users')",
            "PRAGMA main.index_list('users')",
            "SHOW TABLES",
            "DESCRIBE users",
        ] {
            assert!(
                validate_readonly_query(query).is_ok(),
                "safe query was rejected: {query}"
            );
        }
    }
}
