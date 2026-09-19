use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

pub const BRAIN_SCHEMA_VERSION: u32 = 2;
const MAX_CONTENT_BYTES: usize = 100_000;
const MAX_TAGS: usize = 64;
const MAX_TAG_BYTES: usize = 128;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_ARTIFACTS: usize = 64;
const MAX_CANDIDATES: i64 = 500;
const MAX_RESULTS: usize = 50;
const DEFAULT_RESULT_BYTES: usize = 64 * 1024;
const MAX_RESULT_BYTES: usize = 256 * 1024;

pub struct BrainDb {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct BrainScope {
    pub kind: String,
    pub id: String,
}

impl BrainScope {
    pub fn personal() -> Self {
        Self {
            kind: "personal".into(),
            id: String::new(),
        }
    }

    pub fn project(path: impl Into<String>) -> Self {
        Self {
            kind: "project".into(),
            id: path.into(),
        }
    }

    pub fn private_surface() -> Self {
        Self {
            kind: "private".into(),
            id: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrainRecord {
    pub id: String,
    pub schema_version: u32,
    pub user_id: String,
    pub assistant_id: String,
    pub surface: BrainScope,
    pub revision: u32,
    pub kind: String,
    pub scope: BrainScope,
    pub project_path: String,
    pub content: String,
    pub tags: Vec<String>,
    pub provenance: Value,
    pub artifact_refs: Vec<Value>,
    pub confidence: f64,
    pub sensitivity: String,
    pub retention: String,
    pub pinned: bool,
    pub superseded_by: Option<String>,
    pub dedupe_hash: String,
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub accessed_at: Option<i64>,
    pub access_count: u64,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct BrainWrite {
    pub assistant_id: String,
    pub surface: BrainScope,
    pub revision: u32,
    pub kind: String,
    pub scope: BrainScope,
    pub project_path: String,
    pub content: String,
    pub tags: Vec<String>,
    pub provenance: Value,
    pub artifact_refs: Vec<Value>,
    pub confidence: f64,
    pub sensitivity: String,
    pub retention: String,
    pub pinned: bool,
    pub expires_at: Option<i64>,
}

impl Default for BrainWrite {
    fn default() -> Self {
        Self {
            assistant_id: "default".into(),
            surface: BrainScope::private_surface(),
            revision: 1,
            kind: "fact".into(),
            scope: BrainScope::personal(),
            project_path: String::new(),
            content: String::new(),
            tags: Vec::new(),
            provenance: json!({"source":"explicit memory tool"}),
            artifact_refs: Vec::new(),
            confidence: 1.0,
            sensitivity: "normal".into(),
            retention: "durable".into(),
            pinned: false,
            expires_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainWriteOutcome {
    pub record: BrainRecord,
    pub deduplicated: bool,
}

#[derive(Debug, Clone)]
pub struct BrainQuery {
    pub query: String,
    pub assistant_id: Option<String>,
    pub surface: Option<BrainScope>,
    pub scopes: Vec<BrainScope>,
    pub kinds: Vec<String>,
    pub limit: usize,
    pub max_bytes: usize,
    pub include_superseded: bool,
}

impl Default for BrainQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            assistant_id: None,
            surface: None,
            scopes: Vec::new(),
            kinds: Vec::new(),
            limit: 20,
            max_bytes: DEFAULT_RESULT_BYTES,
            include_superseded: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainHit {
    pub record: BrainRecord,
    pub lexical_score: i64,
    pub semantic_score: i64,
    pub score: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainSearchResult {
    pub schema_version: u32,
    pub retrieval_mode: String,
    pub semantic_status: String,
    pub hits: Vec<BrainHit>,
    pub truncated: bool,
    pub bytes: usize,
}

pub trait BrainSemanticIndex {
    fn score(&self, query: &str, records: &[BrainRecord]) -> Result<HashMap<String, f32>, String>;
}

impl BrainDb {
    pub fn open(path: &str) -> Self {
        Self::try_open(path).expect("Failed to open assistant brain database")
    }

    pub fn try_open(path: &str) -> Result<Self, String> {
        if let Some(parent) = PathBuf::from(path).parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let conn = Connection::open(path).map_err(|error| error.to_string())?;
        run_migrations(&conn)?;
        secure_file(path)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
}

fn run_migrations(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         PRAGMA secure_delete=ON;

         CREATE TABLE IF NOT EXISTS assistant_brain_schema (
             version INTEGER PRIMARY KEY,
             applied_at INTEGER NOT NULL
         );

         CREATE TABLE IF NOT EXISTS assistant_brain_records (
             id TEXT PRIMARY KEY,
             schema_version INTEGER NOT NULL,
             user_id TEXT NOT NULL,
             assistant_id TEXT NOT NULL DEFAULT '',
             surface_kind TEXT NOT NULL DEFAULT 'private',
             surface_id TEXT NOT NULL DEFAULT '',
             revision INTEGER NOT NULL DEFAULT 1,
             record_kind TEXT NOT NULL,
             scope_kind TEXT NOT NULL,
             scope_id TEXT NOT NULL DEFAULT '',
             project_path TEXT NOT NULL DEFAULT '',
             content TEXT NOT NULL,
             tags_json TEXT NOT NULL DEFAULT '[]',
             provenance_json TEXT NOT NULL DEFAULT '{}',
             artifact_refs_json TEXT NOT NULL DEFAULT '[]',
             confidence REAL NOT NULL DEFAULT 1.0,
             sensitivity TEXT NOT NULL DEFAULT 'normal',
             retention TEXT NOT NULL DEFAULT 'durable',
             pinned INTEGER NOT NULL DEFAULT 0,
             superseded_by TEXT,
             dedupe_hash TEXT NOT NULL,
             embedding_json TEXT,
             embedding_provider TEXT,
             embedding_model TEXT,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             accessed_at INTEGER,
             access_count INTEGER NOT NULL DEFAULT 0,
             expires_at INTEGER,
             FOREIGN KEY(superseded_by) REFERENCES assistant_brain_records(id)
         );

         CREATE VIRTUAL TABLE IF NOT EXISTS assistant_brain_fts USING fts5(
             content,
             tags_json,
             scope_id,
             content='assistant_brain_records',
             content_rowid='rowid'
         );

         CREATE TRIGGER IF NOT EXISTS assistant_brain_ai
         AFTER INSERT ON assistant_brain_records BEGIN
             INSERT INTO assistant_brain_fts(rowid, content, tags_json, scope_id)
             VALUES (new.rowid, new.content, new.tags_json, new.scope_id);
         END;

         CREATE TRIGGER IF NOT EXISTS assistant_brain_ad
         AFTER DELETE ON assistant_brain_records BEGIN
             INSERT INTO assistant_brain_fts(
                 assistant_brain_fts, rowid, content, tags_json, scope_id
             ) VALUES ('delete', old.rowid, old.content, old.tags_json, old.scope_id);
         END;

         CREATE TRIGGER IF NOT EXISTS assistant_brain_au
         AFTER UPDATE ON assistant_brain_records BEGIN
             INSERT INTO assistant_brain_fts(
                 assistant_brain_fts, rowid, content, tags_json, scope_id
             ) VALUES ('delete', old.rowid, old.content, old.tags_json, old.scope_id);
             INSERT INTO assistant_brain_fts(rowid, content, tags_json, scope_id)
             VALUES (new.rowid, new.content, new.tags_json, new.scope_id);
         END;

         CREATE INDEX IF NOT EXISTS idx_assistant_brain_owner_scope
             ON assistant_brain_records(user_id, scope_kind, scope_id);
         CREATE INDEX IF NOT EXISTS idx_assistant_brain_owner_kind
             ON assistant_brain_records(user_id, record_kind);
         CREATE INDEX IF NOT EXISTS idx_assistant_brain_expiry
             ON assistant_brain_records(user_id, expires_at);
         CREATE INDEX IF NOT EXISTS idx_assistant_brain_dedupe
             ON assistant_brain_records(user_id, dedupe_hash);

         CREATE TABLE IF NOT EXISTS assistant_brain_turn_influence (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             user_id TEXT NOT NULL,
             assistant_id TEXT NOT NULL,
             turn_id TEXT NOT NULL,
             provider TEXT NOT NULL,
             session_key TEXT NOT NULL,
             record_id TEXT NOT NULL,
             created_at INTEGER NOT NULL,
             FOREIGN KEY(record_id) REFERENCES assistant_brain_records(id) ON DELETE CASCADE,
             UNIQUE(user_id, turn_id, record_id)
         );
         CREATE INDEX IF NOT EXISTS idx_assistant_brain_influence_turn
             ON assistant_brain_turn_influence(user_id, assistant_id, turn_id);

         INSERT OR IGNORE INTO assistant_brain_schema(version, applied_at)
         VALUES (1, unixepoch());",
    )
    .map_err(|error| format!("Failed to initialize assistant brain schema: {error}"))?;

    let v2_applied = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_brain_schema WHERE version=2)",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| error.to_string())?;

    if !v2_applied {
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| format!("Failed to begin assistant brain migration: {error}"))?;
        for (column, definition) in [
            ("assistant_id", "assistant_id TEXT NOT NULL DEFAULT ''"),
            (
                "surface_kind",
                "surface_kind TEXT NOT NULL DEFAULT 'private'",
            ),
            ("surface_id", "surface_id TEXT NOT NULL DEFAULT ''"),
            ("revision", "revision INTEGER NOT NULL DEFAULT 1"),
        ] {
            let exists = transaction
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM pragma_table_info('assistant_brain_records') WHERE name=?1
                     )",
                    [column],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|error| error.to_string())?;
            if !exists {
                transaction
                    .execute(
                        &format!("ALTER TABLE assistant_brain_records ADD COLUMN {definition}"),
                        [],
                    )
                    .map_err(|error| format!("Failed to add brain column {column}: {error}"))?;
            }
        }
        transaction
            .execute_batch(
                "DROP TRIGGER IF EXISTS assistant_brain_ai;
                 DROP TRIGGER IF EXISTS assistant_brain_ad;
                 DROP TRIGGER IF EXISTS assistant_brain_au;
                 DROP TABLE IF EXISTS assistant_brain_fts;
                 CREATE VIRTUAL TABLE assistant_brain_fts USING fts5(
                     content,tags_json,scope_id,
                     content='assistant_brain_records',content_rowid='rowid'
                 );
                 CREATE TRIGGER assistant_brain_ai AFTER INSERT ON assistant_brain_records BEGIN
                     INSERT INTO assistant_brain_fts(rowid,content,tags_json,scope_id)
                     VALUES(new.rowid,new.content,new.tags_json,new.scope_id);
                 END;
                 CREATE TRIGGER assistant_brain_ad AFTER DELETE ON assistant_brain_records BEGIN
                     INSERT INTO assistant_brain_fts(
                         assistant_brain_fts,rowid,content,tags_json,scope_id
                     ) VALUES('delete',old.rowid,old.content,old.tags_json,old.scope_id);
                 END;
                 CREATE TRIGGER assistant_brain_au AFTER UPDATE ON assistant_brain_records BEGIN
                     INSERT INTO assistant_brain_fts(
                         assistant_brain_fts,rowid,content,tags_json,scope_id
                     ) VALUES('delete',old.rowid,old.content,old.tags_json,old.scope_id);
                     INSERT INTO assistant_brain_fts(rowid,content,tags_json,scope_id)
                     VALUES(new.rowid,new.content,new.tags_json,new.scope_id);
                 END;
                 INSERT INTO assistant_brain_fts(assistant_brain_fts) VALUES('rebuild');
                 UPDATE assistant_brain_records
                 SET assistant_id=CASE
                     WHEN json_valid(provenance_json)
                         AND NULLIF(CAST(json_extract(provenance_json, '$.assistant_id') AS TEXT), '') IS NOT NULL
                     THEN CAST(json_extract(provenance_json, '$.assistant_id') AS TEXT)
                     WHEN json_valid(provenance_json)
                         AND json_extract(provenance_json, '$.source')='explicit memory tool'
                     THEN 'memory-mcp'
                     ELSE ''
                 END
                 WHERE assistant_id='';
                 UPDATE assistant_brain_records SET schema_version=2,revision=MAX(revision,1);
                 CREATE INDEX IF NOT EXISTS idx_assistant_brain_owner_assistant_surface
                     ON assistant_brain_records(user_id,assistant_id,surface_kind,surface_id);
                 INSERT INTO assistant_brain_schema(version, applied_at)
                 VALUES(2, unixepoch());",
            )
            .map_err(|error| format!("Failed to migrate assistant brain schema: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Failed to commit assistant brain migration: {error}"))?;
    }
    Ok(())
}

#[cfg(unix)]
fn secure_file(path: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn secure_file(_path: &str) -> Result<(), String> {
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<BrainRecord> {
    let tags: String = row.get(12)?;
    let provenance: String = row.get(13)?;
    let artifacts: String = row.get(14)?;
    Ok(BrainRecord {
        id: row.get(0)?,
        schema_version: row.get::<_, i64>(1)?.max(0) as u32,
        user_id: row.get(2)?,
        assistant_id: row.get(3)?,
        surface: BrainScope {
            kind: row.get(4)?,
            id: row.get(5)?,
        },
        revision: row.get::<_, i64>(6)?.max(1) as u32,
        kind: row.get(7)?,
        scope: BrainScope {
            kind: row.get(8)?,
            id: row.get(9)?,
        },
        project_path: row.get(10)?,
        content: row.get(11)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        provenance: serde_json::from_str(&provenance).unwrap_or_else(|_| json!({})),
        artifact_refs: serde_json::from_str(&artifacts).unwrap_or_default(),
        confidence: row.get(15)?,
        sensitivity: row.get(16)?,
        retention: row.get(17)?,
        pinned: row.get::<_, i64>(18)? != 0,
        superseded_by: row.get(19)?,
        dedupe_hash: row.get(20)?,
        embedding_provider: row.get(21)?,
        embedding_model: row.get(22)?,
        created_at: row.get(23)?,
        updated_at: row.get(24)?,
        accessed_at: row.get(25)?,
        access_count: row.get::<_, i64>(26)?.max(0) as u64,
        expires_at: row.get(27)?,
    })
}

const RECORD_COLUMNS: &str =
    "b.id,b.schema_version,b.user_id,b.assistant_id,b.surface_kind,b.surface_id,b.revision,
    b.record_kind,b.scope_kind,b.scope_id,b.project_path,b.content,b.tags_json,b.provenance_json,
    b.artifact_refs_json,b.confidence,b.sensitivity,b.retention,b.pinned,b.superseded_by,
    b.dedupe_hash,b.embedding_provider,b.embedding_model,b.created_at,b.updated_at,b.accessed_at,
    b.access_count,b.expires_at";

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn dedupe_hash(user_id: &str, input: &BrainWrite) -> String {
    let canonical = json!({
        "user":user_id,
        "assistant":input.assistant_id,
        "surfaceKind":input.surface.kind,
        "surfaceId":input.surface.id,
        "revision":input.revision,
        "kind":input.kind,
        "scopeKind":input.scope.kind,
        "scopeId":input.scope.id,
        "content":normalize_text(&input.content),
    });
    format!("{:x}", Sha256::digest(canonical.to_string().as_bytes()))
}

fn valid_record_kind(value: &str) -> bool {
    matches!(
        value,
        "preference" | "fact" | "decision" | "entity" | "goal" | "episode" | "procedure"
    )
}

fn valid_scope_kind(value: &str) -> bool {
    matches!(
        value,
        "personal" | "project" | "chattle_space" | "chattle_channel" | "conversation" | "job"
    )
}

fn validate_scope(scope: &BrainScope) -> Result<(), String> {
    if !valid_scope_kind(&scope.kind) {
        return Err(format!("unsupported brain scope kind {:?}", scope.kind));
    }
    if scope.kind == "personal" {
        if !scope.id.is_empty() {
            return Err("personal brain scope must use an empty id".into());
        }
    } else if scope.id.trim().is_empty() || scope.id.len() > 4096 {
        return Err("non-personal brain scope requires an id of at most 4096 bytes".into());
    }
    Ok(())
}

fn validate_surface(surface: &BrainScope) -> Result<(), String> {
    if !matches!(surface.kind.as_str(), "private" | "chattle_channel") {
        return Err("brain surface must be private or chattle_channel".into());
    }
    if surface.kind == "private" {
        if !surface.id.is_empty() {
            return Err("private brain surface must use an empty id".into());
        }
    } else if surface.id.trim().is_empty() || surface.id.len() > 4096 {
        return Err("shared brain surface requires an id of at most 4096 bytes".into());
    }
    Ok(())
}

fn validate_json_metadata(name: &str, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(format!("{name} exceeds {MAX_METADATA_BYTES} bytes"));
    }
    if crate::public_payload::validate_public_payload(value).is_err() {
        return Err(format!("{name} contains protected runtime data"));
    }
    Ok(())
}

fn validate_write(input: &BrainWrite) -> Result<(), String> {
    if input.assistant_id.trim().is_empty() || input.assistant_id.len() > 128 {
        return Err("brain assistant_id must contain at most 128 bytes".into());
    }
    validate_surface(&input.surface)?;
    if input.revision == 0 {
        return Err("brain revision must be positive".into());
    }
    if !valid_record_kind(&input.kind) {
        return Err(format!("unsupported brain record kind {:?}", input.kind));
    }
    validate_scope(&input.scope)?;
    if input.content.trim().is_empty() || input.content.len() > MAX_CONTENT_BYTES {
        return Err(format!(
            "brain content must contain between 1 and {MAX_CONTENT_BYTES} bytes"
        ));
    }
    if input.tags.len() > MAX_TAGS
        || input
            .tags
            .iter()
            .any(|tag| tag.trim().is_empty() || tag.len() > MAX_TAG_BYTES)
    {
        return Err(format!(
            "brain tags are limited to {MAX_TAGS} non-empty values of {MAX_TAG_BYTES} bytes"
        ));
    }
    if !input.provenance.is_object() {
        return Err("brain provenance must be an object".into());
    }
    if input.artifact_refs.len() > MAX_ARTIFACTS {
        return Err(format!(
            "brain artifact references are limited to {MAX_ARTIFACTS}"
        ));
    }
    if input
        .artifact_refs
        .iter()
        .any(|reference| !reference.is_object())
    {
        return Err("brain artifact references must be objects".into());
    }
    validate_json_metadata("brain provenance", &input.provenance)?;
    validate_json_metadata("brain artifact references", &json!(input.artifact_refs))?;
    if !input.confidence.is_finite() || !(0.0..=1.0).contains(&input.confidence) {
        return Err("brain confidence must be between 0 and 1".into());
    }
    if !matches!(
        input.sensitivity.as_str(),
        "normal" | "private" | "restricted"
    ) {
        return Err("brain sensitivity must be normal, private, or restricted".into());
    }
    if !matches!(
        input.retention.as_str(),
        "session" | "project" | "durable" | "ttl"
    ) {
        return Err("brain retention must be session, project, durable, or ttl".into());
    }
    if input.retention == "ttl" && input.expires_at.is_none() {
        return Err("ttl brain retention requires expires_at".into());
    }
    if input.expires_at.is_some_and(|expiry| expiry <= 0) {
        return Err("brain expires_at must be a positive Unix timestamp".into());
    }
    let content_value = Value::String(input.content.clone());
    if crate::public_payload::validate_public_payload(&content_value).is_err() {
        return Err("brain content contains protected runtime data".into());
    }
    Ok(())
}

fn get_record_with_conn(
    conn: &Connection,
    id: &str,
    user_id: &str,
) -> Result<Option<BrainRecord>, String> {
    let sql = format!(
        "SELECT {RECORD_COLUMNS} FROM assistant_brain_records b WHERE b.id=?1 AND b.user_id=?2"
    );
    conn.query_row(&sql, rusqlite::params![id, user_id], row_to_record)
        .optional()
        .map_err(|error| error.to_string())
}

pub fn get_brain_record(db: &BrainDb, id: &str, user_id: &str) -> Result<BrainRecord, String> {
    get_record_with_conn(&db.conn(), id, user_id)?
        .ok_or_else(|| "Brain record not found or not owned by user".into())
}

pub fn find_active_brain_records_by_tag(
    db: &BrainDb,
    user_id: &str,
    assistant_id: &str,
    surface: &BrainScope,
    scope: &BrainScope,
    kind: &str,
    tag: &str,
) -> Result<Vec<BrainRecord>, String> {
    if assistant_id.trim().is_empty() {
        return Err("brain assistant_id is required".into());
    }
    validate_surface(surface)?;
    validate_scope(scope)?;
    if !valid_record_kind(kind) {
        return Err("unsupported brain record kind".into());
    }
    if tag.trim().is_empty() || tag.len() > MAX_TAG_BYTES {
        return Err("brain tag lookup is invalid".into());
    }
    let conn = db.conn();
    let sql = format!(
        "SELECT {RECORD_COLUMNS} FROM assistant_brain_records b
         WHERE b.user_id=?1 AND b.assistant_id=?2
         AND b.surface_kind=?3 AND b.surface_id=?4
         AND b.scope_kind=?5 AND b.scope_id=?6 AND b.record_kind=?7
         AND b.superseded_by IS NULL AND (b.expires_at IS NULL OR b.expires_at>unixepoch())
         AND EXISTS(SELECT 1 FROM json_each(b.tags_json) WHERE value=?8)
         ORDER BY b.updated_at DESC LIMIT 20"
    );
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            rusqlite::params![
                user_id,
                assistant_id,
                surface.kind,
                surface.id,
                scope.kind,
                scope.id,
                kind,
                tag
            ],
            row_to_record,
        )
        .map_err(|error| error.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn record_turn_influence(
    db: &BrainDb,
    user_id: &str,
    assistant_id: &str,
    turn_id: &str,
    provider: &str,
    session_key: &str,
    record_ids: &[String],
) -> Result<usize, String> {
    if assistant_id.trim().is_empty()
        || assistant_id.len() > 128
        || turn_id.trim().is_empty()
        || turn_id.len() > 128
        || provider.trim().is_empty()
        || provider.len() > 128
        || session_key.trim().is_empty()
        || session_key.len() > 512
        || record_ids.len() > MAX_RESULTS
    {
        return Err("brain turn influence metadata is invalid".into());
    }
    let mut conn = db.conn();
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    let mut inserted = 0;
    for record_id in record_ids {
        let owned = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM assistant_brain_records WHERE id=?1 AND user_id=?2
                 )",
                rusqlite::params![record_id, user_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| error.to_string())?;
        if !owned {
            return Err("brain turn influence contains a foreign record".into());
        }
        inserted += transaction
            .execute(
                "INSERT OR IGNORE INTO assistant_brain_turn_influence(
                    user_id,assistant_id,turn_id,provider,session_key,record_id,created_at
                 ) VALUES(?1,?2,?3,?4,?5,?6,unixepoch())",
                rusqlite::params![
                    user_id,
                    assistant_id,
                    turn_id,
                    provider,
                    session_key,
                    record_id
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(inserted)
}

pub fn turn_influence_record_ids(
    db: &BrainDb,
    user_id: &str,
    assistant_id: &str,
    turn_id: &str,
) -> Result<Vec<String>, String> {
    let conn = db.conn();
    let mut statement = conn
        .prepare(
            "SELECT record_id FROM assistant_brain_turn_influence
             WHERE user_id=?1 AND assistant_id=?2 AND turn_id=?3
             ORDER BY id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(rusqlite::params![user_id, assistant_id, turn_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| error.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn add_brain_record(
    db: &BrainDb,
    user_id: &str,
    input: &BrainWrite,
) -> Result<BrainWriteOutcome, String> {
    validate_write(input)?;
    let conn = db.conn();
    let hash = dedupe_hash(user_id, input);
    let existing_id = conn
        .query_row(
            "SELECT id FROM assistant_brain_records
             WHERE user_id=?1 AND dedupe_hash=?2 AND superseded_by IS NULL
             AND (expires_at IS NULL OR expires_at > unixepoch())
             ORDER BY updated_at DESC LIMIT 1",
            rusqlite::params![user_id, hash],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(existing_id) = existing_id {
        let record = get_record_with_conn(&conn, &existing_id, user_id)?
            .ok_or_else(|| "deduplicated brain record disappeared".to_owned())?;
        return Ok(BrainWriteOutcome {
            record,
            deduplicated: true,
        });
    }

    let id = uuid::Uuid::new_v4().to_string();
    let timestamp = now();
    conn.execute(
        "INSERT INTO assistant_brain_records(
             id,schema_version,user_id,assistant_id,surface_kind,surface_id,revision,
             record_kind,scope_kind,scope_id,project_path,content,tags_json,provenance_json,
             artifact_refs_json,confidence,sensitivity,retention,pinned,dedupe_hash,
             created_at,updated_at,expires_at
         ) VALUES(
             ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,
             ?20,?21,?22,?23
         )",
        rusqlite::params![
            id,
            BRAIN_SCHEMA_VERSION,
            user_id,
            input.assistant_id,
            input.surface.kind,
            input.surface.id,
            input.revision,
            input.kind,
            input.scope.kind,
            input.scope.id,
            input.project_path,
            input.content.trim(),
            serde_json::to_string(&input.tags).map_err(|error| error.to_string())?,
            input.provenance.to_string(),
            serde_json::to_string(&input.artifact_refs).map_err(|error| error.to_string())?,
            input.confidence,
            input.sensitivity,
            input.retention,
            input.pinned,
            hash,
            timestamp,
            timestamp,
            input.expires_at,
        ],
    )
    .map_err(|error| error.to_string())?;
    let record = get_record_with_conn(&conn, &id, user_id)?
        .ok_or_else(|| "new brain record could not be read".to_owned())?;
    Ok(BrainWriteOutcome {
        record,
        deduplicated: false,
    })
}

pub fn update_brain_record(
    db: &BrainDb,
    id: &str,
    user_id: &str,
    input: &BrainWrite,
) -> Result<BrainRecord, String> {
    validate_write(input)?;
    let conn = db.conn();
    if get_record_with_conn(&conn, id, user_id)?.is_none() {
        return Err("Brain record not found or not owned by user".into());
    }
    let hash = dedupe_hash(user_id, input);
    let duplicate = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM assistant_brain_records
                WHERE user_id=?1 AND dedupe_hash=?2 AND id<>?3 AND superseded_by IS NULL
                AND (expires_at IS NULL OR expires_at > unixepoch())
             )",
            rusqlite::params![user_id, hash, id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| error.to_string())?;
    if duplicate {
        return Err("An active duplicate brain record already exists".into());
    }
    conn.execute(
        "UPDATE assistant_brain_records SET
             assistant_id=?1,surface_kind=?2,surface_id=?3,revision=?4,
             record_kind=?5,scope_kind=?6,scope_id=?7,project_path=?8,content=?9,
             tags_json=?10,provenance_json=?11,artifact_refs_json=?12,confidence=?13,
             sensitivity=?14,retention=?15,pinned=?16,dedupe_hash=?17,expires_at=?18,
             updated_at=?19
         WHERE id=?20 AND user_id=?21",
        rusqlite::params![
            input.assistant_id,
            input.surface.kind,
            input.surface.id,
            input.revision,
            input.kind,
            input.scope.kind,
            input.scope.id,
            input.project_path,
            input.content.trim(),
            serde_json::to_string(&input.tags).map_err(|error| error.to_string())?,
            input.provenance.to_string(),
            serde_json::to_string(&input.artifact_refs).map_err(|error| error.to_string())?,
            input.confidence,
            input.sensitivity,
            input.retention,
            input.pinned,
            hash,
            input.expires_at,
            now(),
            id,
            user_id,
        ],
    )
    .map_err(|error| error.to_string())?;
    get_record_with_conn(&conn, id, user_id)?
        .ok_or_else(|| "updated brain record could not be read".into())
}

pub fn pin_brain_record(db: &BrainDb, id: &str, user_id: &str, pinned: bool) -> Result<(), String> {
    let rows = db
        .conn()
        .execute(
            "UPDATE assistant_brain_records SET pinned=?1,updated_at=unixepoch()
             WHERE id=?2 AND user_id=?3",
            rusqlite::params![pinned, id, user_id],
        )
        .map_err(|error| error.to_string())?;
    if rows == 0 {
        return Err("Brain record not found or not owned by user".into());
    }
    Ok(())
}

pub fn supersede_brain_record(
    db: &BrainDb,
    id: &str,
    replacement_id: &str,
    user_id: &str,
) -> Result<(), String> {
    if id == replacement_id {
        return Err("A brain record cannot supersede itself".into());
    }
    let conn = db.conn();
    let original = get_record_with_conn(&conn, id, user_id)?
        .ok_or_else(|| "Brain record not found or not owned by user".to_owned())?;
    let replacement = get_record_with_conn(&conn, replacement_id, user_id)?
        .ok_or_else(|| "Replacement brain record not found or not owned by user".to_owned())?;
    if original.scope != replacement.scope {
        return Err("Replacement brain record must use the same scope".into());
    }
    if replacement.superseded_by.is_some() {
        return Err("Replacement brain record is already superseded".into());
    }
    conn.execute(
        "UPDATE assistant_brain_records SET superseded_by=?1,updated_at=unixepoch()
         WHERE id=?2 AND user_id=?3",
        rusqlite::params![replacement_id, id, user_id],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn forget_brain_record(db: &BrainDb, id: &str, user_id: &str) -> Result<(), String> {
    let conn = db.conn();
    conn.execute(
        "UPDATE assistant_brain_records SET superseded_by=NULL
         WHERE superseded_by=?1 AND user_id=?2",
        rusqlite::params![id, user_id],
    )
    .map_err(|error| error.to_string())?;
    let rows = conn
        .execute(
            "DELETE FROM assistant_brain_records WHERE id=?1 AND user_id=?2",
            rusqlite::params![id, user_id],
        )
        .map_err(|error| error.to_string())?;
    if rows == 0 {
        return Err("Brain record not found or not owned by user".into());
    }
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn cleanup_expired_brain_records(db: &BrainDb, user_id: &str) -> Result<usize, String> {
    let conn = db.conn();
    let deleted = conn
        .execute(
            "DELETE FROM assistant_brain_records
             WHERE user_id=?1 AND pinned=0 AND expires_at IS NOT NULL AND expires_at<=unixepoch()",
            [user_id],
        )
        .map_err(|error| error.to_string())?;
    if deleted > 0 {
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|error| error.to_string())?;
    }
    Ok(deleted)
}

fn search_tokens(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    query
        .split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() > 1)
        .map(str::to_lowercase)
        .filter(|token| seen.insert(token.clone()))
        .take(32)
        .collect()
}

fn fts_query(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|token| format!("\"{}\"*", token.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn scope_matches(record: &BrainRecord, scopes: &[BrainScope]) -> bool {
    scopes.is_empty() || scopes.iter().any(|scope| scope == &record.scope)
}

fn kind_matches(record: &BrainRecord, kinds: &[String]) -> bool {
    kinds.is_empty() || kinds.iter().any(|kind| kind == &record.kind)
}

fn active_matches(record: &BrainRecord, include_superseded: bool, timestamp: i64) -> bool {
    (include_superseded || record.superseded_by.is_none())
        && record.expires_at.is_none_or(|expiry| expiry > timestamp)
}

fn lexical_score(record: &BrainRecord, query: &str, tokens: &[String]) -> i64 {
    let content = record.content.to_lowercase();
    let tags = record.tags.join(" ").to_lowercase();
    let scope = record.scope.id.to_lowercase();
    let phrase = query.trim().to_lowercase();
    let mut score = 0;
    if !phrase.is_empty() {
        if content.contains(&phrase) {
            score += 80;
        }
        if tags.contains(&phrase) {
            score += 100;
        }
    }
    for token in tokens {
        if content.contains(token) {
            score += 12;
        }
        if tags.contains(token) {
            score += 16;
        }
        if scope.contains(token) {
            score += 4;
        }
    }
    if score > 0 {
        score += (record.confidence.clamp(0.0, 1.0) * 10.0).round() as i64;
        if record.pinned {
            score += 25;
        }
    }
    score
}

fn select_all_candidates(
    conn: &Connection,
    user_id: &str,
    assistant_id: Option<&str>,
    surface: Option<&BrainScope>,
    include_superseded: bool,
) -> Result<Vec<BrainRecord>, String> {
    let superseded = if include_superseded {
        ""
    } else {
        "AND b.superseded_by IS NULL"
    };
    let sql = format!(
        "SELECT {RECORD_COLUMNS} FROM assistant_brain_records b
         WHERE b.user_id=?1
         AND (?2 IS NULL OR b.assistant_id=?2)
         AND (?3 IS NULL OR (b.surface_kind=?3 AND b.surface_id=?4))
         {superseded}
         AND (b.expires_at IS NULL OR b.expires_at>unixepoch())
         ORDER BY b.pinned DESC,b.updated_at DESC LIMIT {MAX_CANDIDATES}"
    );
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            rusqlite::params![
                user_id,
                assistant_id,
                surface.map(|value| value.kind.as_str()),
                surface.map(|value| value.id.as_str())
            ],
            row_to_record,
        )
        .map_err(|error| error.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn select_fts_candidates(
    conn: &Connection,
    user_id: &str,
    assistant_id: Option<&str>,
    surface: Option<&BrainScope>,
    tokens: &[String],
    include_superseded: bool,
) -> Result<Vec<BrainRecord>, String> {
    let superseded = if include_superseded {
        ""
    } else {
        "AND b.superseded_by IS NULL"
    };
    let sql = format!(
        "SELECT {RECORD_COLUMNS}
         FROM assistant_brain_fts
         JOIN assistant_brain_records b ON b.rowid=assistant_brain_fts.rowid
         WHERE assistant_brain_fts MATCH ?1 AND b.user_id=?2
         AND (?3 IS NULL OR b.assistant_id=?3)
         AND (?4 IS NULL OR (b.surface_kind=?4 AND b.surface_id=?5))
         {superseded}
         AND (b.expires_at IS NULL OR b.expires_at>unixepoch())
         ORDER BY bm25(assistant_brain_fts),b.pinned DESC,b.updated_at DESC
         LIMIT {MAX_CANDIDATES}"
    );
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            rusqlite::params![
                fts_query(tokens),
                user_id,
                assistant_id,
                surface.map(|value| value.kind.as_str()),
                surface.map(|value| value.id.as_str())
            ],
            row_to_record,
        )
        .map_err(|error| error.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn search_brain_records(
    db: &BrainDb,
    user_id: &str,
    query: &BrainQuery,
    semantic: Option<&dyn BrainSemanticIndex>,
) -> Result<BrainSearchResult, String> {
    if query
        .assistant_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err("brain assistant_id filter cannot be empty".into());
    }
    if let Some(surface) = &query.surface {
        validate_surface(surface)?;
    }
    let tokens = search_tokens(&query.query);
    if tokens.is_empty() {
        return Ok(BrainSearchResult {
            schema_version: BRAIN_SCHEMA_VERSION,
            retrieval_mode: "lexical".into(),
            semantic_status: "disabled".into(),
            hits: Vec::new(),
            truncated: false,
            bytes: 0,
        });
    }
    for scope in &query.scopes {
        validate_scope(scope)?;
    }
    if query.kinds.iter().any(|kind| !valid_record_kind(kind)) {
        return Err("brain query contains an unsupported record kind".into());
    }

    let conn = db.conn();
    let assistant_id = query.assistant_id.as_deref();
    let surface = query.surface.as_ref();
    let lexical_candidates = select_fts_candidates(
        &conn,
        user_id,
        assistant_id,
        surface,
        &tokens,
        query.include_superseded,
    )
    .or_else(|_| {
        select_all_candidates(
            &conn,
            user_id,
            assistant_id,
            surface,
            query.include_superseded,
        )
    })?;
    let mut candidates = BTreeMap::new();
    for record in lexical_candidates {
        candidates.insert(record.id.clone(), record);
    }
    let all_candidates = select_all_candidates(
        &conn,
        user_id,
        assistant_id,
        surface,
        query.include_superseded,
    )?;
    for record in all_candidates {
        if semantic.is_some() || record.pinned {
            candidates.entry(record.id.clone()).or_insert(record);
        }
    }
    let timestamp = now();
    let mut candidates = candidates
        .into_values()
        .filter(|record| {
            scope_matches(record, &query.scopes)
                && kind_matches(record, &query.kinds)
                && active_matches(record, query.include_superseded, timestamp)
        })
        .collect::<Vec<_>>();

    let (semantic_scores, retrieval_mode, semantic_status) = if let Some(index) = semantic {
        match index.score(&query.query, &candidates) {
            Ok(scores) => (scores, "hybrid".to_owned(), "ready".to_owned()),
            Err(_) => (HashMap::new(), "lexical".to_owned(), "fallback".to_owned()),
        }
    } else {
        (HashMap::new(), "lexical".to_owned(), "disabled".to_owned())
    };

    let mut hits = candidates
        .drain(..)
        .filter_map(|record| {
            let lexical = lexical_score(&record, &query.query, &tokens);
            let semantic = semantic_scores
                .get(&record.id)
                .copied()
                .unwrap_or_default()
                .clamp(0.0, 1.0);
            let semantic_score = (semantic * 100.0).round() as i64;
            let relevance = lexical
                .saturating_mul(2)
                .saturating_add(semantic_score)
                .saturating_add(if record.pinned { 10 } else { 0 });
            let score =
                relevance.saturating_add((record.confidence.clamp(0.0, 1.0) * 10.0).round() as i64);
            (relevance > 0).then_some(BrainHit {
                record,
                lexical_score: lexical,
                semantic_score,
                score,
            })
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| right.record.updated_at.cmp(&left.record.updated_at))
            .then_with(|| left.record.id.cmp(&right.record.id))
    });

    let limit = query.limit.clamp(1, MAX_RESULTS);
    let max_bytes = query.max_bytes.clamp(1024, MAX_RESULT_BYTES);
    let available = hits.len();
    let mut selected = Vec::new();
    let mut bytes: usize = 0;
    for hit in hits.into_iter().take(limit) {
        let size = serde_json::to_vec(&hit)
            .map_err(|error| error.to_string())?
            .len();
        if bytes.saturating_add(size) > max_bytes {
            break;
        }
        bytes += size;
        selected.push(hit);
    }
    let truncated = selected.len() < available.min(limit) || available > limit;
    if !selected.is_empty() {
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        for hit in &selected {
            transaction
                .execute(
                    "UPDATE assistant_brain_records
                     SET accessed_at=unixepoch(),access_count=access_count+1
                     WHERE id=?1 AND user_id=?2",
                    rusqlite::params![hit.record.id, user_id],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
    }

    Ok(BrainSearchResult {
        schema_version: BRAIN_SCHEMA_VERSION,
        retrieval_mode,
        semantic_status,
        hits: selected,
        truncated,
        bytes,
    })
}

pub fn export_brain_records(
    db: &BrainDb,
    user_id: &str,
    assistant_id: Option<&str>,
    surface: Option<&BrainScope>,
    scopes: &[BrainScope],
    include_superseded: bool,
    max_bytes: usize,
) -> Result<(Vec<BrainRecord>, bool, usize), String> {
    if assistant_id.is_some_and(|value| value.trim().is_empty()) {
        return Err("brain assistant_id filter cannot be empty".into());
    }
    if let Some(surface) = surface {
        validate_surface(surface)?;
    }
    for scope in scopes {
        validate_scope(scope)?;
    }
    let conn = db.conn();
    let records = select_all_candidates(&conn, user_id, assistant_id, surface, include_superseded)?;
    let budget = max_bytes.clamp(1024, MAX_RESULT_BYTES);
    let mut output = Vec::new();
    let mut bytes: usize = 0;
    let mut truncated = false;
    for record in records
        .into_iter()
        .filter(|record| scope_matches(record, scopes))
    {
        let size = serde_json::to_vec(&record)
            .map_err(|error| error.to_string())?
            .len();
        if bytes.saturating_add(size) > budget {
            truncated = true;
            break;
        }
        bytes += size;
        output.push(record);
    }
    Ok((output, truncated, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> (BrainDb, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "flamel-assistant-brain-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        (BrainDb::open(path.to_string_lossy().as_ref()), path)
    }

    fn write(content: &str, scope: BrainScope) -> BrainWrite {
        BrainWrite {
            content: content.into(),
            scope,
            provenance: json!({"source":"test","session_id":"session-1"}),
            artifact_refs: vec![json!({"kind":"file","id":"src/main.rs"})],
            ..BrainWrite::default()
        }
    }

    fn cleanup(db: BrainDb, path: PathBuf) {
        drop(db);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn fresh_and_legacy_databases_receive_versioned_brain_schema() {
        let path = std::env::temp_dir().join(format!(
            "flamel-assistant-brain-upgrade-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        {
            let legacy = Connection::open(&path).unwrap();
            legacy
                .execute_batch(
                    "CREATE TABLE memories(
                        id TEXT PRIMARY KEY,user_id TEXT NOT NULL,project_path TEXT NOT NULL,
                        content TEXT NOT NULL,tags TEXT NOT NULL,created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    CREATE TABLE assistant_brain_records (
                        id TEXT PRIMARY KEY,schema_version INTEGER NOT NULL,user_id TEXT NOT NULL,
                        record_kind TEXT NOT NULL,scope_kind TEXT NOT NULL,scope_id TEXT NOT NULL DEFAULT '',
                        project_path TEXT NOT NULL DEFAULT '',content TEXT NOT NULL,
                        tags_json TEXT NOT NULL DEFAULT '[]',provenance_json TEXT NOT NULL DEFAULT '{}',
                        artifact_refs_json TEXT NOT NULL DEFAULT '[]',confidence REAL NOT NULL DEFAULT 1.0,
                        sensitivity TEXT NOT NULL DEFAULT 'normal',retention TEXT NOT NULL DEFAULT 'durable',
                        pinned INTEGER NOT NULL DEFAULT 0,superseded_by TEXT,dedupe_hash TEXT NOT NULL,
                        embedding_json TEXT,embedding_provider TEXT,embedding_model TEXT,
                        created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,accessed_at INTEGER,
                        access_count INTEGER NOT NULL DEFAULT 0,expires_at INTEGER
                    );
                    INSERT INTO assistant_brain_records(
                        id,schema_version,user_id,record_kind,scope_kind,scope_id,content,
                        provenance_json,dedupe_hash,created_at,updated_at
                    ) VALUES(
                        'legacy-record',1,'owner','fact','personal','','legacy fact',
                        '{\"assistant_id\":\"assistant-legacy\",\"source\":\"test\"}',
                        'legacy-hash',1,1
                    );
                    INSERT INTO assistant_brain_records(
                        id,schema_version,user_id,record_kind,scope_kind,scope_id,content,
                        provenance_json,dedupe_hash,created_at,updated_at
                    ) VALUES(
                        'legacy-tool-record',1,'owner','fact','personal','','legacy tool fact',
                        '{\"source\":\"explicit memory tool\"}',
                        'legacy-tool-hash',1,1
                    );",
                )
                .unwrap();
        }
        let db = BrainDb::open(path.to_string_lossy().as_ref());
        let version: i64 = db
            .conn()
            .query_row(
                "SELECT MAX(version) FROM assistant_brain_schema",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, BRAIN_SCHEMA_VERSION as i64);
        let migrated = get_brain_record(&db, "legacy-record", "owner").unwrap();
        assert_eq!(migrated.schema_version, BRAIN_SCHEMA_VERSION);
        assert_eq!(migrated.assistant_id, "assistant-legacy");
        assert_eq!(migrated.surface, BrainScope::private_surface());
        assert_eq!(migrated.revision, 1);
        assert_eq!(
            get_brain_record(&db, "legacy-tool-record", "owner")
                .unwrap()
                .assistant_id,
            "memory-mcp"
        );
        let legacy_table: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memories'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy_table, 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        cleanup(db, path);
    }

    #[test]
    fn records_are_isolated_by_owner_assistant_surface_and_exact_scope() {
        let (db, path) = database();
        let mut input = write(
            "channel launch decision",
            BrainScope {
                kind: "chattle_channel".into(),
                id: "channel-a".into(),
            },
        );
        input.assistant_id = "assistant-a".into();
        input.surface = BrainScope::private_surface();
        add_brain_record(&db, "owner", &input).unwrap();
        let query = BrainQuery {
            query: "launch".into(),
            assistant_id: Some("assistant-a".into()),
            surface: Some(BrainScope::private_surface()),
            scopes: vec![BrainScope {
                kind: "chattle_channel".into(),
                id: "channel-a".into(),
            }],
            ..BrainQuery::default()
        };
        assert_eq!(
            search_brain_records(&db, "owner", &query, None)
                .unwrap()
                .hits
                .len(),
            1
        );
        assert!(
            search_brain_records(&db, "other", &query, None)
                .unwrap()
                .hits
                .is_empty()
        );
        let wrong_assistant = BrainQuery {
            assistant_id: Some("assistant-b".into()),
            ..query.clone()
        };
        assert!(
            search_brain_records(&db, "owner", &wrong_assistant, None)
                .unwrap()
                .hits
                .is_empty()
        );
        let wrong_surface = BrainQuery {
            surface: Some(BrainScope {
                kind: "chattle_channel".into(),
                id: "channel-a".into(),
            }),
            ..query.clone()
        };
        assert!(
            search_brain_records(&db, "owner", &wrong_surface, None)
                .unwrap()
                .hits
                .is_empty()
        );
        let wrong_scope = BrainQuery {
            scopes: vec![BrainScope {
                kind: "chattle_channel".into(),
                id: "channel-b".into(),
            }],
            ..query
        };
        assert!(
            search_brain_records(&db, "owner", &wrong_scope, None)
                .unwrap()
                .hits
                .is_empty()
        );
        cleanup(db, path);
    }

    #[test]
    fn dedupe_expiry_pinning_and_supersession_are_deterministic() {
        let (db, path) = database();
        let first = add_brain_record(
            &db,
            "owner",
            &write("Use the native Flamel runtime", BrainScope::personal()),
        )
        .unwrap();
        let duplicate = add_brain_record(
            &db,
            "owner",
            &write("  use the native  flamel runtime ", BrainScope::personal()),
        )
        .unwrap();
        assert!(duplicate.deduplicated);
        assert_eq!(duplicate.record.id, first.record.id);

        let mut expired = write("temporary episode", BrainScope::personal());
        expired.retention = "ttl".into();
        expired.expires_at = Some(now() - 1);
        let expired = add_brain_record(&db, "owner", &expired).unwrap();
        pin_brain_record(&db, &expired.record.id, "owner", true).unwrap();
        assert_eq!(cleanup_expired_brain_records(&db, "owner").unwrap(), 0);
        pin_brain_record(&db, &expired.record.id, "owner", false).unwrap();
        assert_eq!(cleanup_expired_brain_records(&db, "owner").unwrap(), 1);

        let replacement = add_brain_record(
            &db,
            "owner",
            &write(
                "Use the revised native runtime contract",
                BrainScope::personal(),
            ),
        )
        .unwrap();
        supersede_brain_record(&db, &first.record.id, &replacement.record.id, "owner").unwrap();
        let result = search_brain_records(
            &db,
            "owner",
            &BrainQuery {
                query: "runtime".into(),
                ..BrainQuery::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].record.id, replacement.record.id);
        cleanup(db, path);
    }

    struct BoostSemantic {
        id: String,
    }

    impl BrainSemanticIndex for BoostSemantic {
        fn score(
            &self,
            _query: &str,
            records: &[BrainRecord],
        ) -> Result<HashMap<String, f32>, String> {
            Ok(records
                .iter()
                .map(|record| {
                    (
                        record.id.clone(),
                        if record.id == self.id { 1.0 } else { 0.0 },
                    )
                })
                .collect())
        }
    }

    struct FailingSemantic;

    impl BrainSemanticIndex for FailingSemantic {
        fn score(
            &self,
            _query: &str,
            _records: &[BrainRecord],
        ) -> Result<HashMap<String, f32>, String> {
            Err("provider unavailable".into())
        }
    }

    #[test]
    fn fts_and_semantic_reranking_form_a_bounded_hybrid_retriever() {
        let (db, path) = database();
        let first = add_brain_record(
            &db,
            "owner",
            &write("runtime architecture", BrainScope::personal()),
        )
        .unwrap();
        let boosted = add_brain_record(
            &db,
            "owner",
            &write("runtime operations", BrainScope::personal()),
        )
        .unwrap();
        let indexed: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM assistant_brain_fts WHERE assistant_brain_fts MATCH 'runtime'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexed, 2);
        let result = search_brain_records(
            &db,
            "owner",
            &BrainQuery {
                query: "runtime".into(),
                max_bytes: 4096,
                ..BrainQuery::default()
            },
            Some(&BoostSemantic {
                id: boosted.record.id.clone(),
            }),
        )
        .unwrap();
        assert_eq!(result.retrieval_mode, "hybrid");
        assert_eq!(result.semantic_status, "ready");
        assert_eq!(result.hits[0].record.id, boosted.record.id);
        assert_ne!(result.hits[0].record.id, first.record.id);
        assert!(result.bytes <= 4096);
        cleanup(db, path);
    }

    #[test]
    fn retrieval_is_bounded_and_semantic_failure_falls_back_deterministically() {
        let (db, path) = database();
        for index in 0..8 {
            add_brain_record(
                &db,
                "owner",
                &write(
                    &format!("native runtime recovery fact number {index}"),
                    BrainScope::personal(),
                ),
            )
            .unwrap();
        }
        let query = BrainQuery {
            query: "native runtime recovery".into(),
            limit: 3,
            max_bytes: 4096,
            ..BrainQuery::default()
        };
        let first = search_brain_records(&db, "owner", &query, Some(&FailingSemantic)).unwrap();
        let second = search_brain_records(&db, "owner", &query, Some(&FailingSemantic)).unwrap();
        assert_eq!(first.retrieval_mode, "lexical");
        assert_eq!(first.semantic_status, "fallback");
        assert_eq!(first.hits.len(), 3);
        assert!(first.bytes <= 4096);
        assert!(first.truncated);
        assert_eq!(
            first
                .hits
                .iter()
                .map(|hit| &hit.record.id)
                .collect::<Vec<_>>(),
            second
                .hits
                .iter()
                .map(|hit| &hit.record.id)
                .collect::<Vec<_>>()
        );
        cleanup(db, path);
    }

    #[test]
    fn malformed_provenance_is_rejected_and_user_values_are_persisted() {
        let (db, path) = database();
        let mut malformed = write("ordinary fact", BrainScope::personal());
        malformed.provenance = json!(["not", "an", "object"]);
        assert!(add_brain_record(&db, "owner", &malformed).is_err());

        let mut supplied = write("ordinary fact", BrainScope::personal());
        supplied.provenance = json!({"bot_token":"keep-this-token"});
        let stored = add_brain_record(&db, "owner", &supplied).unwrap();
        assert_eq!(stored.record.provenance["bot_token"], "keep-this-token");

        let mut content = write(
            "Authorization: Bearer keep-this-value",
            BrainScope::personal(),
        );
        content.provenance = json!({"source":"test"});
        let stored = add_brain_record(&db, "owner", &content).unwrap();
        assert_eq!(
            stored.record.content,
            "Authorization: Bearer keep-this-value"
        );
        cleanup(db, path);
    }

    #[test]
    fn edit_forget_export_and_access_accounting_are_owner_safe() {
        let (db, path) = database();
        let created = add_brain_record(
            &db,
            "owner",
            &write("initial project decision", BrainScope::project("/project")),
        )
        .unwrap();
        assert!(
            update_brain_record(
                &db,
                &created.record.id,
                "other",
                &write("foreign edit", BrainScope::project("/project")),
            )
            .is_err()
        );
        let edited = update_brain_record(
            &db,
            &created.record.id,
            "owner",
            &write("accepted project decision", BrainScope::project("/project")),
        )
        .unwrap();
        assert_eq!(edited.content, "accepted project decision");

        let result = search_brain_records(
            &db,
            "owner",
            &BrainQuery {
                query: "accepted decision".into(),
                scopes: vec![BrainScope::project("/project")],
                ..BrainQuery::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(
            get_brain_record(&db, &created.record.id, "owner")
                .unwrap()
                .access_count,
            1
        );

        let (exported, truncated, _) = export_brain_records(
            &db,
            "owner",
            None,
            None,
            &[BrainScope::project("/project")],
            false,
            16 * 1024,
        )
        .unwrap();
        assert_eq!(exported.len(), 1);
        assert!(!truncated);
        assert!(forget_brain_record(&db, &created.record.id, "other").is_err());
        forget_brain_record(&db, &created.record.id, "owner").unwrap();
        assert!(get_brain_record(&db, &created.record.id, "owner").is_err());
        cleanup(db, path);
    }
}
