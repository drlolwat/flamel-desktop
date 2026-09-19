use crate::db::Db;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rusqlite::{OptionalExtension, Transaction};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const PACK_SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_CONNECTORS: usize = 8;
const MAX_TOOLS: usize = 200;
const MAX_EVENTS: usize = 100;
const MAX_SKILLS: usize = 32;
const FLAMEL_PACK_SIGNER_KEY_ID: &str = "flamel-release-v1";
// This is the Ed25519 public key already trusted by Flamel Desktop's updater.
// The minisign envelope contains two algorithm bytes, an eight-byte key id,
// and the raw 32-byte public key used here for detached pack signatures.
const FLAMEL_RELEASE_MINISIGN_KEY_B64: &str =
    "RWRxd7w4rSELwFXg41TXARjA3Gus0qLGsBlJaCeGnBjuI5jWde2vQUy0";
const FIRST_PARTY_NATIVE_ADAPTERS: &[&str] = crate::developer_packs::NATIVE_ADAPTER_IDS;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackEnvelope {
    pub manifest: CapabilityPackManifest,
    pub signature: CapabilityPackSignature,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackSignature {
    pub algorithm: String,
    pub key_id: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackManifest {
    pub schema_version: u32,
    pub id: String,
    pub slug: String,
    pub version: Version,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub category: String,
    pub publisher: String,
    #[serde(default)]
    pub homepage_url: Option<String>,
    pub connectors: Vec<CapabilityPackConnector>,
    #[serde(default)]
    pub tools: Vec<CapabilityPackTool>,
    #[serde(default)]
    pub events: Vec<CapabilityPackEvent>,
    pub auth: CapabilityPackAuth,
    #[serde(default)]
    pub skills: Vec<CapabilityPackSkill>,
    pub ui: CapabilityPackUi,
    pub approval: CapabilityPackApproval,
    #[serde(default)]
    pub schedule_eligible: bool,
    pub health_check: CapabilityPackHealthCheck,
    pub uninstall: CapabilityPackUninstall,
    #[serde(default, skip_serializing_if = "CapabilityPackContract::is_empty")]
    pub contract: CapabilityPackContract,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackContract {
    #[serde(default)]
    pub required_connections: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub supported_surfaces: Vec<String>,
    #[serde(default)]
    pub minimum_flamel_version: Option<Version>,
    #[serde(default)]
    pub provenance_url: Option<String>,
    #[serde(default)]
    pub evaluation_fixtures: Vec<CapabilityPackEvaluationFixture>,
}

impl CapabilityPackContract {
    fn is_empty(&self) -> bool {
        self.required_connections.is_empty()
            && self.required_capabilities.is_empty()
            && self.permissions.is_empty()
            && self.supported_surfaces.is_empty()
            && self.minimum_flamel_version.is_none()
            && self.provenance_url.is_none()
            && self.evaluation_fixtures.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackEvaluationFixture {
    pub name: String,
    pub tool_name: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub expected_output_schema: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackConnector {
    pub id: String,
    pub transport: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub remote_url: Option<String>,
    #[serde(default)]
    pub native_adapter: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub annotations: Value,
    #[serde(default = "default_true")]
    pub enabled_by_default: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackEvent {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub schema: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackAuth {
    pub kind: String,
    #[serde(default)]
    pub minimum_scopes: Vec<String>,
    #[serde(default)]
    pub credential_kinds: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackSkill {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub category: String,
    pub instructions: String,
    #[serde(default)]
    pub triggers: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackUi {
    pub icon: String,
    pub accent: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackApproval {
    pub decision: String,
    #[serde(default)]
    pub tools: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackHealthCheck {
    pub tool_name: String,
    pub interval_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPackUninstall {
    pub revoke_credentials: bool,
    pub disable_schedules: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityPackInstallation {
    pub pack_id: String,
    pub capability_id: Option<String>,
    pub active_version: Option<String>,
    pub previous_version: Option<String>,
    pub state: String,
    pub schedule_eligible: bool,
    pub schedules_enabled: bool,
    pub installed_by: String,
    pub installed_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
struct VerifiedPack {
    canonical: Vec<u8>,
    sha256: String,
}

fn default_true() -> bool {
    true
}

fn bounded_text(value: &str, max: usize, field: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() || value.len() > max {
        return Err(format!("{field} must contain between 1 and {max} bytes"));
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn valid_scope(value: &str) -> bool {
    valid_identifier(value)
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
}

fn validate_object_schema(value: &Value, field: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{field} must be a JSON object schema"))?;
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return Err(format!("{field} must declare type=object"));
    }
    Ok(())
}

fn native_adapter_allowed(adapter: &str) -> bool {
    FIRST_PARTY_NATIVE_ADAPTERS.contains(&adapter)
}

fn primary_connector(manifest: &CapabilityPackManifest) -> &CapabilityPackConnector {
    manifest
        .connectors
        .iter()
        .find(|connector| connector.primary)
        .unwrap_or(&manifest.connectors[0])
}

pub fn validate_manifest(manifest: &CapabilityPackManifest) -> Result<(), String> {
    let encoded = serde_json::to_vec(manifest).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_MANIFEST_BYTES {
        return Err("Capability pack manifest exceeds 256 KiB".into());
    }
    crate::public_payload::validate_public_payload(
        &serde_json::to_value(manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|_| "Capability pack manifests cannot contain credentials or private payloads")?;
    if manifest.schema_version != PACK_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported capability pack schema version {}",
            manifest.schema_version
        ));
    }
    if !valid_identifier(&manifest.id) || !manifest.id.starts_with("flamel.") {
        return Err("Pack id must be a Flamel-owned identifier beginning with 'flamel.'".into());
    }
    if !valid_identifier(&manifest.slug) {
        return Err("Pack slug contains unsupported characters".into());
    }
    if manifest.publisher != "Flamel" {
        return Err("Executable capability packs must be published by Flamel".into());
    }
    bounded_text(&manifest.title, 160, "Pack title")?;
    bounded_text(&manifest.category, 80, "Pack category")?;
    if manifest.description.len() > 4_000 {
        return Err("Pack description exceeds 4000 bytes".into());
    }
    if manifest.connectors.is_empty() || manifest.connectors.len() > MAX_CONNECTORS {
        return Err(format!(
            "Capability packs require 1-{MAX_CONNECTORS} connectors"
        ));
    }
    if manifest
        .connectors
        .iter()
        .filter(|entry| entry.primary)
        .count()
        > 1
    {
        return Err("Only one pack connector may be primary".into());
    }
    let mut connector_ids = HashSet::new();
    for connector in &manifest.connectors {
        if !valid_identifier(&connector.id) || !connector_ids.insert(&connector.id) {
            return Err("Pack connector ids must be valid and unique".into());
        }
        match connector.transport.as_str() {
            "streamable_http" | "sse" => {
                let remote = connector
                    .remote_url
                    .as_deref()
                    .ok_or("Remote pack connectors require remoteUrl")?;
                let parsed =
                    reqwest::Url::parse(remote).map_err(|_| "Pack connector URL is invalid")?;
                if parsed.scheme() != "https" {
                    return Err("Pack connector URLs must use HTTPS".into());
                }
                if connector.native_adapter.is_some() {
                    return Err("Remote pack connectors cannot name a native adapter".into());
                }
            }
            "native" => {
                let adapter = connector
                    .native_adapter
                    .as_deref()
                    .ok_or("Native pack connectors require a reviewed adapter id")?;
                if !native_adapter_allowed(adapter) {
                    return Err(format!(
                        "Native adapter '{adapter}' is not compiled into this Flamel release"
                    ));
                }
                if connector.remote_url.is_some() {
                    return Err("Native pack connectors cannot contain a remote URL".into());
                }
            }
            _ => {
                return Err(
                    "Pack connectors may use only reviewed native, streamable_http, or sse transports"
                        .into(),
                );
            }
        }
    }

    if manifest.tools.is_empty() || manifest.tools.len() > MAX_TOOLS {
        return Err(format!(
            "Capability packs require 1-{MAX_TOOLS} typed tools"
        ));
    }
    let mut tool_names = HashSet::new();
    for tool in &manifest.tools {
        if !valid_identifier(&tool.name) || !tool_names.insert(&tool.name) {
            return Err("Pack tool names must be valid and unique".into());
        }
        validate_object_schema(&tool.input_schema, "Tool inputSchema")?;
        if let Some(output) = &tool.output_schema {
            validate_object_schema(output, "Tool outputSchema")?;
        }
        if !tool.annotations.is_object() {
            return Err("Tool annotations must be a JSON object".into());
        }
    }
    if manifest.events.len() > MAX_EVENTS {
        return Err(format!(
            "Capability packs may define at most {MAX_EVENTS} events"
        ));
    }
    let mut event_names = HashSet::new();
    for event in &manifest.events {
        if !valid_identifier(&event.name) || !event_names.insert(&event.name) {
            return Err("Pack event names must be valid and unique".into());
        }
        validate_object_schema(&event.schema, "Event schema")?;
    }
    if !matches!(manifest.auth.kind.as_str(), "none" | "oauth2" | "token") {
        return Err("Pack auth kind must be none, oauth2, or token".into());
    }
    let mut scopes = HashSet::new();
    for scope in &manifest.auth.minimum_scopes {
        if !valid_scope(scope) || !scopes.insert(scope) {
            return Err("Minimum scopes must be valid and unique".into());
        }
    }
    if manifest.auth.minimum_scopes.len() > 64 || manifest.auth.credential_kinds.len() > 16 {
        return Err("Pack auth requirements exceed supported bounds".into());
    }
    if manifest.skills.len() > MAX_SKILLS {
        return Err(format!(
            "Capability packs may define at most {MAX_SKILLS} skills"
        ));
    }
    let mut skill_slugs = HashSet::new();
    for skill in &manifest.skills {
        if !valid_identifier(&skill.slug) || !skill_slugs.insert(&skill.slug) {
            return Err("Pack skill slugs must be valid and unique".into());
        }
        bounded_text(&skill.title, 160, "Skill title")?;
        bounded_text(&skill.category, 80, "Skill category")?;
        bounded_text(&skill.instructions, 32_000, "Skill instructions")?;
        if skill.triggers.len() > 64 {
            return Err("A pack skill may define at most 64 triggers".into());
        }
    }
    bounded_text(&manifest.ui.icon, 128, "Pack icon")?;
    bounded_text(&manifest.ui.accent, 64, "Pack accent")?;
    if !matches!(
        manifest.approval.decision.as_str(),
        "allow" | "require_approval" | "deny"
    ) {
        return Err("Pack approval decision must be allow, require_approval, or deny".into());
    }
    if manifest
        .approval
        .tools
        .iter()
        .any(|tool| !tool_names.contains(tool))
    {
        return Err("Pack approval policy references an unknown tool".into());
    }
    if !tool_names.contains(&manifest.health_check.tool_name) {
        return Err("Pack health check references an unknown tool".into());
    }
    if !(30..=86_400).contains(&manifest.health_check.interval_seconds) {
        return Err("Pack health interval must be between 30 and 86400 seconds".into());
    }
    if !manifest.uninstall.revoke_credentials || !manifest.uninstall.disable_schedules {
        return Err("Pack uninstall policy must revoke credentials and disable schedules".into());
    }
    if manifest
        .homepage_url
        .as_deref()
        .is_some_and(|value| !value.starts_with("https://"))
    {
        return Err("Pack homepage URLs must use HTTPS".into());
    }
    if manifest
        .contract
        .provenance_url
        .as_deref()
        .is_some_and(|value| !value.starts_with("https://"))
    {
        return Err("Pack provenance URLs must use HTTPS".into());
    }
    if manifest
        .contract
        .required_connections
        .iter()
        .any(|required| !connector_ids.contains(required))
    {
        return Err("Pack contract references an unknown required connection".into());
    }
    if manifest
        .contract
        .required_capabilities
        .iter()
        .any(|required| !valid_identifier(required) || required == &manifest.id)
    {
        return Err("Pack required capabilities must be valid external identifiers".into());
    }
    let mut permissions = HashSet::new();
    if manifest.contract.permissions.iter().any(|permission| {
        !valid_identifier(permission)
            || !permissions.insert(permission)
            || !tool_names.contains(permission)
    }) {
        return Err("Pack permissions must be unique signed tool names".into());
    }
    let mut surfaces = HashSet::new();
    if manifest.contract.supported_surfaces.iter().any(|surface| {
        !matches!(
            surface.as_str(),
            "assistant" | "automation" | "mcp" | "chat"
        ) || !surfaces.insert(surface)
    }) {
        return Err("Pack supported surfaces contain an unknown or duplicate surface".into());
    }
    if let Some(minimum) = &manifest.contract.minimum_flamel_version {
        let running = Version::parse(env!("CARGO_PKG_VERSION"))
            .map_err(|_| "Running Flamel version is malformed")?;
        if minimum > &running {
            return Err(format!(
                "Pack requires Flamel {minimum} or newer; this server is {running}"
            ));
        }
    }
    if manifest.contract.evaluation_fixtures.len() > 32 {
        return Err("Pack contract may define at most 32 evaluation fixtures".into());
    }
    let mut fixture_names = HashSet::new();
    for fixture in &manifest.contract.evaluation_fixtures {
        if !valid_identifier(&fixture.name) || !fixture_names.insert(&fixture.name) {
            return Err("Pack evaluation fixture names must be valid and unique".into());
        }
        if !tool_names.contains(&fixture.tool_name) || !fixture.input.is_object() {
            return Err(
                "Pack evaluation fixtures must target a signed tool with object input".into(),
            );
        }
        if let Some(schema) = &fixture.expected_output_schema {
            validate_object_schema(schema, "Evaluation expectedOutputSchema")?;
        }
    }
    Ok(())
}

fn verify_envelope_with_key(
    envelope: &CapabilityPackEnvelope,
    key: &[u8; 32],
) -> Result<VerifiedPack, String> {
    validate_manifest(&envelope.manifest)?;
    if envelope.signature.algorithm != "ed25519" {
        return Err("Capability pack signatures must use ed25519".into());
    }
    let canonical = serde_json::to_vec(&envelope.manifest).map_err(|error| error.to_string())?;
    let raw_signature = STANDARD
        .decode(envelope.signature.value.trim())
        .map_err(|_| "Capability pack signature is not valid base64")?;
    let signature = Signature::from_slice(&raw_signature)
        .map_err(|_| "Capability pack signature must contain 64 bytes")?;
    let verifying_key = VerifyingKey::from_bytes(key).map_err(|_| "Trusted pack key is invalid")?;
    verifying_key
        .verify(&canonical, &signature)
        .map_err(|_| "Capability pack signature verification failed")?;
    let sha256 = hex::encode(Sha256::digest(&canonical));
    Ok(VerifiedPack { canonical, sha256 })
}

fn trusted_release_key(key_id: &str) -> Result<[u8; 32], String> {
    if key_id != FLAMEL_PACK_SIGNER_KEY_ID {
        return Err(format!("Capability pack signer '{key_id}' is not trusted"));
    }
    let envelope = STANDARD
        .decode(FLAMEL_RELEASE_MINISIGN_KEY_B64)
        .map_err(|_| "Bundled Flamel release key is malformed")?;
    let raw = envelope
        .get(envelope.len().saturating_sub(32)..)
        .ok_or("Bundled Flamel release key is incomplete")?;
    raw.try_into()
        .map_err(|_| "Bundled Flamel release key has the wrong length".into())
}

fn pack_installation_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CapabilityPackInstallation> {
    Ok(CapabilityPackInstallation {
        pack_id: row.get(0)?,
        capability_id: row.get(1)?,
        active_version: row.get(2)?,
        previous_version: row.get(3)?,
        state: row.get(4)?,
        schedule_eligible: row.get::<_, i64>(5)? == 1,
        schedules_enabled: row.get::<_, i64>(6)? == 1,
        installed_by: row.get(7)?,
        installed_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

pub fn list_pack_installations(db: &Db) -> Vec<CapabilityPackInstallation> {
    let conn = db.conn();
    let mut statement = match conn.prepare(
        "SELECT pack_id,capability_id,active_version,previous_version,state,
                schedule_eligible,schedules_enabled,installed_by,installed_at,updated_at
         FROM capability_pack_installations ORDER BY pack_id",
    ) {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };
    statement
        .query_map([], pack_installation_from_row)
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

pub fn pack_installation_for_capability(
    db: &Db,
    capability_id: &str,
) -> Option<CapabilityPackInstallation> {
    db.conn()
        .query_row(
            "SELECT pack_id,capability_id,active_version,previous_version,state,
                    schedule_eligible,schedules_enabled,installed_by,installed_at,updated_at
             FROM capability_pack_installations WHERE capability_id=?1",
            [capability_id],
            pack_installation_from_row,
        )
        .optional()
        .ok()
        .flatten()
}

fn existing_tool_states(
    transaction: &Transaction<'_>,
    capability_id: &str,
) -> Result<HashMap<String, bool>, String> {
    let mut statement = transaction
        .prepare("SELECT name,enabled FROM capability_tools WHERE capability_id=?1")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([capability_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? == 1))
        })
        .map_err(|error| error.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn apply_manifest_projection(
    transaction: &Transaction<'_>,
    manifest: &CapabilityPackManifest,
    verified: &VerifiedPack,
    signature: &CapabilityPackSignature,
) -> Result<String, String> {
    let capability_id = format!("pack-{}", manifest.slug);
    let existing: Option<(String, String)> = transaction
        .query_row(
            "SELECT id,source FROM capability_catalog WHERE slug=?1",
            [&manifest.slug],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if existing
        .as_ref()
        .is_some_and(|(id, source)| id != &capability_id || source != "first_party_pack")
    {
        return Err(format!(
            "Pack slug '{}' is already owned by another catalog source",
            manifest.slug
        ));
    }

    let connector = primary_connector(manifest);
    let now = chrono::Utc::now().to_rfc3339();
    let permissions =
        serde_json::to_string(&manifest.auth.minimum_scopes).map_err(|error| error.to_string())?;
    let toolsets = serde_json::to_string(
        &manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
    )
    .map_err(|error| error.to_string())?;
    let metadata = json!({
        "packId":manifest.id,
        "packVersion":manifest.version,
        "packState":"installed",
        "signed":true,
        "signerKeyId":signature.key_id,
        "manifestSha256":verified.sha256,
        "connectors":manifest.connectors,
        "events":manifest.events,
        "auth":manifest.auth,
        "ui":manifest.ui,
        "approval":manifest.approval,
        "scheduleEligible":manifest.schedule_eligible,
        "healthCheck":manifest.health_check,
        "uninstall":manifest.uninstall,
    });
    transaction
        .execute(
            "INSERT INTO capability_catalog (
                id,slug,title,description,category,source,publisher,version,transport,
                command,args_json,remote_url,auth_kind,permissions_json,toolsets_json,
                admin_approved,builtin,metadata_json,created_at,updated_at
             ) VALUES (?1,?2,?3,?4,?5,'first_party_pack',?6,?7,?8,NULL,'[]',?9,?10,?11,?12,1,0,?13,?14,?14)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title,description=excluded.description,
                category=excluded.category,publisher=excluded.publisher,version=excluded.version,
                transport=excluded.transport,command=NULL,args_json='[]',remote_url=excluded.remote_url,
                auth_kind=excluded.auth_kind,permissions_json=excluded.permissions_json,
                toolsets_json=excluded.toolsets_json,admin_approved=1,metadata_json=excluded.metadata_json,
                updated_at=excluded.updated_at",
            rusqlite::params![
                capability_id,
                manifest.slug,
                manifest.title,
                manifest.description,
                manifest.category,
                manifest.publisher,
                manifest.version.to_string(),
                connector.transport,
                connector.remote_url,
                manifest.auth.kind,
                permissions,
                toolsets,
                metadata.to_string(),
                now,
            ],
        )
        .map_err(|error| error.to_string())?;

    let prior_tool_states = existing_tool_states(transaction, &capability_id)?;
    transaction
        .execute(
            "DELETE FROM capability_tools WHERE capability_id=?1",
            [&capability_id],
        )
        .map_err(|error| error.to_string())?;
    for tool in &manifest.tools {
        let enabled = prior_tool_states
            .get(&tool.name)
            .copied()
            .unwrap_or(tool.enabled_by_default);
        transaction
            .execute(
                "INSERT INTO capability_tools (
                    capability_id,name,description,input_schema_json,output_schema_json,
                    annotations_json,enabled,discovered_at
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                rusqlite::params![
                    capability_id,
                    tool.name,
                    tool.description,
                    tool.input_schema.to_string(),
                    tool.output_schema.as_ref().map(Value::to_string),
                    tool.annotations.to_string(),
                    enabled,
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    transaction
        .execute(
            "UPDATE capability_skills SET enabled=0 WHERE pack_id=?1",
            [&manifest.id],
        )
        .map_err(|error| error.to_string())?;
    for skill in &manifest.skills {
        let id = format!("pack-skill-{}-{}", manifest.slug, skill.slug);
        let slug = format!("{}.{}", manifest.slug, skill.slug);
        let version = manifest.version.to_string();
        let hash = crate::db::skill_catalog::insert_pack_skill_version(
            transaction,
            &id,
            &manifest.id,
            &version,
            &capability_id,
            &skill.title,
            &skill.description,
            &skill.category,
            &skill.instructions,
            &skill.triggers,
        )?;
        transaction
            .execute(
                "INSERT INTO capability_skills (
                    id,slug,title,description,category,instructions,triggers_json,
                    enabled,builtin,pack_id,pack_version,current_version,content_hash,
                    created_at,updated_at
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,1,0,?8,?9,?9,?10,?11,?11)
                 ON CONFLICT(slug) DO UPDATE SET title=excluded.title,
                    description=excluded.description,category=excluded.category,
                    instructions=excluded.instructions,triggers_json=excluded.triggers_json,
                    enabled=1,pack_id=excluded.pack_id,pack_version=excluded.pack_version,
                    current_version=excluded.current_version,content_hash=excluded.content_hash,
                    updated_at=excluded.updated_at",
                rusqlite::params![
                    id,
                    slug,
                    skill.title,
                    skill.description,
                    skill.category,
                    skill.instructions,
                    serde_json::to_string(&skill.triggers).map_err(|error| error.to_string())?,
                    manifest.id,
                    version,
                    hash,
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    let policy_id = format!("pack-policy-{}", manifest.id);
    transaction
        .execute("DELETE FROM capability_policies WHERE id=?1", [&policy_id])
        .map_err(|error| error.to_string())?;
    let rules = if manifest.approval.tools.is_empty() {
        json!({"source":"capability_pack","packId":manifest.id})
    } else {
        json!({
            "source":"capability_pack",
            "packId":manifest.id,
            "tools":manifest.approval.tools,
        })
    };
    transaction
        .execute(
            "INSERT INTO capability_policies (
                id,capability_id,scope_kind,scope_id,decision,rules_json,
                created_by,created_at,updated_at
             ) VALUES (?1,?2,'server',NULL,?3,?4,'flamel-pack',?5,?5)",
            rusqlite::params![
                policy_id,
                capability_id,
                manifest.approval.decision,
                rules.to_string(),
                now
            ],
        )
        .map_err(|error| error.to_string())?;

    transaction
        .execute(
            "INSERT INTO capability_marketplace (
                id,slug,title,description,category,publisher,official,homepage_url,
                manifest_url,transport,remote_url,auth_kind,permissions_json,checksum,
                status,metadata_json,created_at,updated_at
             ) VALUES (?1,?2,?3,?4,?5,'Flamel',1,?6,NULL,?7,?8,?9,?10,?11,
                       'installed',?12,?13,?13)
             ON CONFLICT(slug) DO UPDATE SET title=excluded.title,
                description=excluded.description,category=excluded.category,
                publisher='Flamel',official=1,homepage_url=excluded.homepage_url,
                transport=excluded.transport,remote_url=excluded.remote_url,
                auth_kind=excluded.auth_kind,permissions_json=excluded.permissions_json,
                checksum=excluded.checksum,status='installed',
                metadata_json=excluded.metadata_json,updated_at=excluded.updated_at",
            rusqlite::params![
                format!("pack-market-{}", manifest.slug),
                manifest.slug,
                manifest.title,
                manifest.description,
                manifest.category,
                manifest.homepage_url,
                connector.transport,
                connector.remote_url,
                manifest.auth.kind,
                permissions,
                verified.sha256,
                metadata.to_string(),
                now,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(capability_id)
}

fn declared_permissions(manifest: &CapabilityPackManifest) -> HashSet<String> {
    manifest
        .contract
        .permissions
        .iter()
        .cloned()
        .chain(
            manifest
                .auth
                .minimum_scopes
                .iter()
                .map(|scope| format!("scope:{scope}")),
        )
        .chain(
            manifest
                .connectors
                .iter()
                .map(|connector| format!("connection:{}", connector.id)),
        )
        .collect()
}

fn preview_with_verified(
    db: &Db,
    envelope: &CapabilityPackEnvelope,
    verified: &VerifiedPack,
) -> Result<Value, String> {
    let previous = db
        .conn()
        .query_row(
            "SELECT versions.manifest_json
             FROM capability_pack_installations installations
             JOIN capability_pack_versions versions
               ON versions.pack_id=installations.pack_id
              AND versions.version=installations.active_version
             WHERE installations.pack_id=?1 AND installations.state='installed'",
            [&envelope.manifest.id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .and_then(|value| serde_json::from_str::<CapabilityPackManifest>(&value).ok());

    let next_permissions = declared_permissions(&envelope.manifest);
    let previous_permissions = previous
        .as_ref()
        .map(declared_permissions)
        .unwrap_or_default();
    let mut added_permissions = next_permissions
        .difference(&previous_permissions)
        .cloned()
        .collect::<Vec<_>>();
    let mut removed_permissions = previous_permissions
        .difference(&next_permissions)
        .cloned()
        .collect::<Vec<_>>();
    added_permissions.sort();
    removed_permissions.sort();

    Ok(json!({
        "schemaVersion": PACK_SCHEMA_VERSION,
        "packId": envelope.manifest.id,
        "version": envelope.manifest.version,
        "previousVersion": previous.map(|manifest| manifest.version),
        "manifestSha256": verified.sha256,
        "signature": {
            "verified": true,
            "algorithm": envelope.signature.algorithm,
            "keyId": envelope.signature.key_id,
        },
        "permissionDiff": {
            "added": added_permissions,
            "removed": removed_permissions,
            "escalates": !added_permissions.is_empty(),
        },
        "requiredCapabilities": envelope.manifest.contract.required_capabilities,
        "supportedSurfaces": envelope.manifest.contract.supported_surfaces,
        "evaluationFixtures": envelope.manifest.contract.evaluation_fixtures.len(),
        "approvalRequired": true,
        "installs": false,
    }))
}

pub fn preview_capability_pack(
    db: &Db,
    envelope: &CapabilityPackEnvelope,
) -> Result<Value, String> {
    let key = trusted_release_key(&envelope.signature.key_id)?;
    let verified = verify_envelope_with_key(envelope, &key)?;
    preview_with_verified(db, envelope, &verified)
}

#[cfg(test)]
fn preview_capability_pack_with_key(
    db: &Db,
    envelope: &CapabilityPackEnvelope,
    key: &[u8; 32],
) -> Result<Value, String> {
    let verified = verify_envelope_with_key(envelope, key)?;
    preview_with_verified(db, envelope, &verified)
}

fn install_with_verified(
    db: &Db,
    envelope: &CapabilityPackEnvelope,
    verified: &VerifiedPack,
    installed_by: &str,
) -> Result<CapabilityPackInstallation, String> {
    let mut conn = db.conn();
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    for required in &envelope.manifest.contract.required_capabilities {
        let available = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM capability_catalog
                    WHERE id=?1 AND admin_approved=1
                )",
                [required],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())?
            == 1;
        if !available {
            return Err(format!(
                "Required capability '{required}' is unavailable or not administrator-approved"
            ));
        }
    }
    let current: Option<(Option<String>, Option<String>, String)> = transaction
        .query_row(
            "SELECT active_version,previous_version,state
             FROM capability_pack_installations WHERE pack_id=?1",
            [&envelope.manifest.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some((Some(active), _, state)) = &current {
        let active_version =
            Version::parse(active).map_err(|_| "Stored active pack version is malformed")?;
        if state == "installed" && envelope.manifest.version < active_version {
            return Err("Pack downgrades require the explicit rollback action".into());
        }
        if state == "installed" && envelope.manifest.version == active_version {
            let stored_digest: Option<String> = transaction
                .query_row(
                    "SELECT manifest_sha256 FROM capability_pack_versions
                     WHERE pack_id=?1 AND version=?2",
                    rusqlite::params![envelope.manifest.id, active],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if stored_digest.as_deref() == Some(&verified.sha256) {
                transaction.commit().map_err(|error| error.to_string())?;
                drop(conn);
                return list_pack_installations(db)
                    .into_iter()
                    .find(|item| item.pack_id == envelope.manifest.id)
                    .ok_or("Installed pack state disappeared".into());
            }
            return Err(
                "An installed pack version cannot be replaced with different content".into(),
            );
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    transaction
        .execute(
            "INSERT INTO capability_pack_versions (
                pack_id,version,manifest_json,manifest_sha256,signature_b64,
                signer_key_id,verified_at,created_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?7)
             ON CONFLICT(pack_id,version) DO NOTHING",
            rusqlite::params![
                envelope.manifest.id,
                envelope.manifest.version.to_string(),
                String::from_utf8_lossy(&verified.canonical),
                verified.sha256,
                envelope.signature.value,
                envelope.signature.key_id,
                now,
            ],
        )
        .map_err(|error| error.to_string())?;
    let capability_id = apply_manifest_projection(
        &transaction,
        &envelope.manifest,
        verified,
        &envelope.signature,
    )?;
    let previous = current
        .as_ref()
        .and_then(|(active, _, state)| (state == "installed").then(|| active.clone()).flatten());
    transaction
        .execute(
            "INSERT INTO capability_pack_installations (
                pack_id,capability_id,active_version,previous_version,state,
                schedule_eligible,schedules_enabled,installed_by,installed_at,updated_at
             ) VALUES (?1,?2,?3,?4,'installed',?5,0,?6,?7,?7)
             ON CONFLICT(pack_id) DO UPDATE SET capability_id=excluded.capability_id,
                active_version=excluded.active_version,previous_version=excluded.previous_version,
                state='installed',schedule_eligible=excluded.schedule_eligible,
                schedules_enabled=0,installed_by=excluded.installed_by,
                installed_at=CASE WHEN capability_pack_installations.state='uninstalled'
                                  THEN excluded.installed_at
                                  ELSE capability_pack_installations.installed_at END,
                updated_at=excluded.updated_at",
            rusqlite::params![
                envelope.manifest.id,
                capability_id,
                envelope.manifest.version.to_string(),
                previous,
                envelope.manifest.schedule_eligible,
                installed_by,
                now,
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    drop(conn);
    list_pack_installations(db)
        .into_iter()
        .find(|item| item.pack_id == envelope.manifest.id)
        .ok_or("Installed pack state disappeared".into())
}

pub fn install_capability_pack(
    db: &Db,
    envelope: &CapabilityPackEnvelope,
    installed_by: &str,
) -> Result<CapabilityPackInstallation, String> {
    let key = trusted_release_key(&envelope.signature.key_id)?;
    let verified = verify_envelope_with_key(envelope, &key)?;
    install_with_verified(db, envelope, &verified, installed_by)
}

#[cfg(test)]
fn install_capability_pack_with_key(
    db: &Db,
    envelope: &CapabilityPackEnvelope,
    installed_by: &str,
    key: &[u8; 32],
) -> Result<CapabilityPackInstallation, String> {
    let verified = verify_envelope_with_key(envelope, key)?;
    install_with_verified(db, envelope, &verified, installed_by)
}

fn load_stored_envelope(
    db: &Db,
    pack_id: &str,
    version: &str,
) -> Result<CapabilityPackEnvelope, String> {
    let (manifest, signature, signer): (String, String, String) = db
        .conn()
        .query_row(
            "SELECT manifest_json,signature_b64,signer_key_id
             FROM capability_pack_versions WHERE pack_id=?1 AND version=?2",
            rusqlite::params![pack_id, version],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| "Stored pack version not found".to_string())?;
    Ok(CapabilityPackEnvelope {
        manifest: serde_json::from_str(&manifest)
            .map_err(|_| "Stored pack manifest is malformed")?,
        signature: CapabilityPackSignature {
            algorithm: "ed25519".into(),
            key_id: signer,
            value: signature,
        },
    })
}

fn rollback_with_key(
    db: &Db,
    pack_id: &str,
    actor: &str,
    key: &[u8; 32],
) -> Result<CapabilityPackInstallation, String> {
    let current = list_pack_installations(db)
        .into_iter()
        .find(|item| item.pack_id == pack_id && item.state == "installed")
        .ok_or("Installed capability pack not found")?;
    let active = current
        .active_version
        .as_deref()
        .ok_or("Pack has no active version")?;
    let previous = current
        .previous_version
        .as_deref()
        .ok_or("Pack has no rollback version")?;
    let envelope = load_stored_envelope(db, pack_id, previous)?;
    let verified = verify_envelope_with_key(&envelope, key)?;

    let mut conn = db.conn();
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    let capability_id = apply_manifest_projection(
        &transaction,
        &envelope.manifest,
        &verified,
        &envelope.signature,
    )?;
    let now = chrono::Utc::now().to_rfc3339();
    transaction
        .execute(
            "UPDATE capability_pack_installations SET capability_id=?1,
                active_version=?2,previous_version=?3,schedule_eligible=?4,
                schedules_enabled=0,installed_by=?5,updated_at=?6
             WHERE pack_id=?7 AND state='installed'",
            rusqlite::params![
                capability_id,
                previous,
                active,
                envelope.manifest.schedule_eligible,
                actor,
                now,
                pack_id,
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    drop(conn);
    list_pack_installations(db)
        .into_iter()
        .find(|item| item.pack_id == pack_id)
        .ok_or("Rolled back pack state disappeared".into())
}

pub fn rollback_capability_pack(
    db: &Db,
    pack_id: &str,
    actor: &str,
) -> Result<CapabilityPackInstallation, String> {
    let current = list_pack_installations(db)
        .into_iter()
        .find(|item| item.pack_id == pack_id && item.state == "installed")
        .ok_or("Installed capability pack not found")?;
    let previous = current
        .previous_version
        .as_deref()
        .ok_or("Pack has no rollback version")?;
    let envelope = load_stored_envelope(db, pack_id, previous)?;
    let key = trusted_release_key(&envelope.signature.key_id)?;
    rollback_with_key(db, pack_id, actor, &key)
}

#[cfg(test)]
fn rollback_capability_pack_with_key(
    db: &Db,
    pack_id: &str,
    actor: &str,
    key: &[u8; 32],
) -> Result<CapabilityPackInstallation, String> {
    rollback_with_key(db, pack_id, actor, key)
}

pub fn uninstall_capability_pack(
    db: &Db,
    pack_id: &str,
    actor: &str,
) -> Result<CapabilityPackInstallation, String> {
    let current = list_pack_installations(db)
        .into_iter()
        .find(|item| item.pack_id == pack_id && item.state == "installed")
        .ok_or("Installed capability pack not found")?;
    let capability_id = current
        .capability_id
        .as_deref()
        .ok_or("Installed pack has no catalog capability")?;
    let slug: String = db
        .conn()
        .query_row(
            "SELECT slug FROM capability_catalog WHERE id=?1",
            [capability_id],
            |row| row.get(0),
        )
        .map_err(|_| "Installed pack catalog entry not found".to_string())?;

    let mut conn = db.conn();
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE capability_connections SET enabled=0,status='disabled',
                credential_ref=NULL,updated_at=?1 WHERE capability_id=?2",
            rusqlite::params![chrono::Utc::now().to_rfc3339(), capability_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM capability_credentials WHERE capability_id=?1",
            [capability_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM capability_catalog WHERE id=?1",
            [capability_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE capability_skill_installations SET enabled=0
             WHERE skill_id IN (SELECT id FROM capability_skills WHERE pack_id=?1)",
            [pack_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute("DELETE FROM capability_skills WHERE pack_id=?1", [pack_id])
        .map_err(|error| error.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    transaction
        .execute(
            "UPDATE capability_pack_installations SET capability_id=NULL,
                previous_version=active_version,active_version=NULL,state='uninstalled',
                schedules_enabled=0,installed_by=?1,updated_at=?2 WHERE pack_id=?3",
            rusqlite::params![actor, now, pack_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE capability_marketplace SET status='available',
                metadata_json=json_set(metadata_json,'$.packState','uninstalled'),
                updated_at=?1 WHERE slug=?2",
            rusqlite::params![now, slug],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    drop(conn);
    list_pack_installations(db)
        .into_iter()
        .find(|item| item.pack_id == pack_id)
        .ok_or("Uninstalled pack state disappeared".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use uuid::Uuid;

    fn database() -> (std::path::PathBuf, Db) {
        let root = std::env::temp_dir().join(format!("flamel-capability-pack-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("flamel.db");
        let db = Db::open(path.to_string_lossy().as_ref());
        (root, db)
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn manifest(version: &str) -> CapabilityPackManifest {
        CapabilityPackManifest {
            schema_version: PACK_SCHEMA_VERSION,
            id: "flamel.test-pack".into(),
            slug: "test-pack".into(),
            version: Version::parse(version).unwrap(),
            title: "Test Pack".into(),
            description: "A signed first-party test pack".into(),
            category: "testing".into(),
            publisher: "Flamel".into(),
            homepage_url: Some("https://flamel.dev".into()),
            connectors: vec![CapabilityPackConnector {
                id: "primary".into(),
                transport: "streamable_http".into(),
                primary: true,
                remote_url: Some("https://connector.flamel.dev/mcp".into()),
                native_adapter: None,
            }],
            tools: vec![CapabilityPackTool {
                name: "test.read".into(),
                description: "Read test state".into(),
                input_schema: json!({"type":"object","properties":{"id":{"type":"string"}}}),
                output_schema: Some(
                    json!({"type":"object","properties":{"ok":{"type":"boolean"}}}),
                ),
                annotations: json!({"readOnlyHint":true}),
                enabled_by_default: true,
            }],
            events: vec![CapabilityPackEvent {
                name: "test.changed".into(),
                description: "Test state changed".into(),
                schema: json!({"type":"object","properties":{"id":{"type":"string"}}}),
            }],
            auth: CapabilityPackAuth {
                kind: "token".into(),
                minimum_scopes: vec!["test.read".into()],
                credential_kinds: vec!["token".into()],
            },
            skills: vec![CapabilityPackSkill {
                slug: "inspect".into(),
                title: "Inspect test state".into(),
                description: "Use the test reader".into(),
                category: "testing".into(),
                instructions: "Call test.read with the exact requested id.".into(),
                triggers: vec!["inspect test".into()],
            }],
            ui: CapabilityPackUi {
                icon: "plug".into(),
                accent: "violet".into(),
                summary: "Signed test capability".into(),
            },
            approval: CapabilityPackApproval {
                decision: "allow".into(),
                tools: vec!["test.read".into()],
            },
            schedule_eligible: true,
            health_check: CapabilityPackHealthCheck {
                tool_name: "test.read".into(),
                interval_seconds: 60,
            },
            uninstall: CapabilityPackUninstall {
                revoke_credentials: true,
                disable_schedules: true,
            },
            contract: CapabilityPackContract {
                required_connections: vec!["primary".into()],
                required_capabilities: Vec::new(),
                permissions: vec!["test.read".into()],
                supported_surfaces: vec!["assistant".into(), "automation".into(), "mcp".into()],
                minimum_flamel_version: Some(Version::parse(env!("CARGO_PKG_VERSION")).unwrap()),
                provenance_url: Some("https://flamel.dev/security/capability-packs".into()),
                evaluation_fixtures: vec![CapabilityPackEvaluationFixture {
                    name: "health".into(),
                    tool_name: "test.read".into(),
                    input: json!({}),
                    expected_output_schema: Some(
                        json!({"type":"object","properties":{"ok":{"type":"boolean"}}}),
                    ),
                }],
            },
        }
    }

    fn envelope(version: &str) -> CapabilityPackEnvelope {
        let manifest = manifest(version);
        let key = signing_key();
        let canonical = serde_json::to_vec(&manifest).unwrap();
        let signature = key.sign(&canonical);
        CapabilityPackEnvelope {
            manifest,
            signature: CapabilityPackSignature {
                algorithm: "ed25519".into(),
                key_id: "test-key".into(),
                value: STANDARD.encode(signature.to_bytes()),
            },
        }
    }

    #[test]
    fn compiled_developer_pack_installs_as_signed_native_and_gates_mutations() {
        let (root, db) = database();
        let key = signing_key();
        let verification_key = key.verifying_key().to_bytes();
        let manifest = crate::developer_packs::release_manifests()
            .into_iter()
            .find(|manifest| manifest.slug == "linear")
            .unwrap();
        let canonical = serde_json::to_vec(&manifest).unwrap();
        let envelope = CapabilityPackEnvelope {
            manifest,
            signature: CapabilityPackSignature {
                algorithm: "ed25519".into(),
                key_id: "test-key".into(),
                value: STANDARD.encode(key.sign(&canonical).to_bytes()),
            },
        };

        let installation =
            install_capability_pack_with_key(&db, &envelope, "admin", &verification_key).unwrap();
        let capability_id = installation.capability_id.unwrap();
        crate::db::capabilities::connect(&db, &capability_id, "tenant-a", None).unwrap();
        let entry = crate::db::capabilities::list_catalog(&db, "tenant-a", None)
            .into_iter()
            .find(|entry| entry.id == capability_id)
            .unwrap();
        assert_eq!(entry.transport, "native");
        assert_eq!(entry.native_adapter.as_deref(), Some("flamel.linear.v1"));
        assert!(entry.pack_signed);
        assert!(crate::db::capabilities::connection_has_exact_scopes(
            &db,
            &capability_id,
            "tenant-a",
            None,
        ));
        assert!(
            crate::db::capabilities::policy_decision(
                &db,
                &capability_id,
                "linear.issues.query",
                "tenant-a",
                None,
            )
            .is_ok()
        );
        assert!(
            crate::db::capabilities::policy_decision(
                &db,
                &capability_id,
                "linear.issue.update",
                "tenant-a",
                None,
            )
            .unwrap_err()
            .contains("requires approval")
        );
        assert!(
            crate::db::capabilities::list_tools(&db, &capability_id, false)
                .into_iter()
                .find(|tool| tool.name == "linear.issue.update")
                .is_some_and(|tool| !tool.enabled)
        );
        assert!(
            crate::db::capabilities::list_catalog(&db, "tenant-b", None)
                .into_iter()
                .find(|entry| entry.id == capability_id)
                .is_some_and(|entry| !entry.connected)
        );

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn signature_schema_and_supply_chain_validation_fail_closed() {
        let (root, db) = database();
        let key = signing_key().verifying_key().to_bytes();
        let release_key = trusted_release_key(FLAMEL_PACK_SIGNER_KEY_ID).unwrap();
        assert!(VerifyingKey::from_bytes(&release_key).is_ok());
        assert!(trusted_release_key("untrusted-key").is_err());
        let valid = envelope("1.0.0");
        assert!(install_capability_pack_with_key(&db, &valid, "admin", &key).is_ok());

        let mut tampered = envelope("1.1.0");
        tampered.manifest.title = "Tampered after signing".into();
        assert!(
            install_capability_pack_with_key(&db, &tampered, "admin", &key)
                .unwrap_err()
                .contains("verification failed")
        );

        let mut arbitrary_runtime = manifest("1.1.0");
        arbitrary_runtime.connectors[0].transport = "stdio".into();
        arbitrary_runtime.connectors[0].remote_url = None;
        let canonical = serde_json::to_vec(&arbitrary_runtime).unwrap();
        let invalid = CapabilityPackEnvelope {
            manifest: arbitrary_runtime,
            signature: CapabilityPackSignature {
                algorithm: "ed25519".into(),
                key_id: "test-key".into(),
                value: STANDARD.encode(signing_key().sign(&canonical).to_bytes()),
            },
        };
        assert!(
            install_capability_pack_with_key(&db, &invalid, "admin", &key)
                .unwrap_err()
                .contains("only reviewed")
        );

        let unknown: Result<CapabilityPackEnvelope, _> = serde_json::from_value(json!({
            "manifest":serde_json::to_value(manifest("2.0.0")).unwrap(),
            "signature":{"algorithm":"ed25519","keyId":"test-key","value":"x"},
            "openclawPlugin":"npm:untrusted"
        }));
        assert!(unknown.is_err());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preview_reports_permission_escalation_and_missing_dependencies_without_mutation() {
        let (root, db) = database();
        let key = signing_key().verifying_key().to_bytes();
        let first = envelope("1.0.0");
        let preview = preview_capability_pack_with_key(&db, &first, &key).unwrap();
        assert_eq!(preview["installs"], false);
        assert_eq!(preview["signature"]["verified"], true);
        assert_eq!(preview["permissionDiff"]["escalates"], true);
        assert!(list_pack_installations(&db).is_empty());

        install_capability_pack_with_key(&db, &first, "admin", &key).unwrap();
        let mut expanded = manifest("2.0.0");
        expanded.tools.push(CapabilityPackTool {
            name: "test.write".into(),
            description: "Write test state".into(),
            input_schema: json!({"type":"object"}),
            output_schema: Some(json!({"type":"object"})),
            annotations: json!({"destructiveHint":true}),
            enabled_by_default: false,
        });
        expanded.contract.permissions.push("test.write".into());
        let canonical = serde_json::to_vec(&expanded).unwrap();
        let expanded = CapabilityPackEnvelope {
            manifest: expanded,
            signature: CapabilityPackSignature {
                algorithm: "ed25519".into(),
                key_id: "test-key".into(),
                value: STANDARD.encode(signing_key().sign(&canonical).to_bytes()),
            },
        };
        let preview = preview_capability_pack_with_key(&db, &expanded, &key).unwrap();
        assert!(
            preview["permissionDiff"]["added"]
                .as_array()
                .unwrap()
                .iter()
                .any(|permission| permission == "test.write")
        );
        assert_eq!(
            list_pack_installations(&db)[0].active_version.as_deref(),
            Some("1.0.0")
        );

        let mut missing_dependency = manifest("2.1.0");
        missing_dependency
            .contract
            .required_capabilities
            .push("missing.mcp.server".into());
        let canonical = serde_json::to_vec(&missing_dependency).unwrap();
        let missing_dependency = CapabilityPackEnvelope {
            manifest: missing_dependency,
            signature: CapabilityPackSignature {
                algorithm: "ed25519".into(),
                key_id: "test-key".into(),
                value: STANDARD.encode(signing_key().sign(&canonical).to_bytes()),
            },
        };
        let error =
            install_capability_pack_with_key(&db, &missing_dependency, "admin", &key).unwrap_err();
        assert!(error.contains("unavailable"), "{error}");

        let mut incompatible = manifest("3.0.0");
        incompatible.contract.minimum_flamel_version = Some(Version::new(999, 0, 0));
        let canonical = serde_json::to_vec(&incompatible).unwrap();
        let incompatible = CapabilityPackEnvelope {
            manifest: incompatible,
            signature: CapabilityPackSignature {
                algorithm: "ed25519".into(),
                key_id: "test-key".into(),
                value: STANDARD.encode(signing_key().sign(&canonical).to_bytes()),
            },
        };
        assert!(
            preview_capability_pack_with_key(&db, &incompatible, &key)
                .unwrap_err()
                .contains("requires Flamel")
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn install_update_rollback_and_uninstall_are_atomic_and_revoke_credentials() {
        let (root, db) = database();
        let key = signing_key().verifying_key().to_bytes();
        let first =
            install_capability_pack_with_key(&db, &envelope("1.0.0"), "admin", &key).unwrap();
        assert_eq!(first.active_version.as_deref(), Some("1.0.0"));
        assert!(first.previous_version.is_none());
        assert_eq!(
            crate::db::capabilities::list_tools(&db, "pack-test-pack", true).len(),
            1
        );
        let catalog = crate::db::capabilities::list_catalog(&db, "user-1", None);
        let catalog_entry = catalog
            .iter()
            .find(|entry| entry.id == "pack-test-pack")
            .unwrap();
        assert_eq!(catalog_entry.pack_id.as_deref(), Some("flamel.test-pack"));
        assert_eq!(catalog_entry.pack_version.as_deref(), Some("1.0.0"));
        assert!(catalog_entry.pack_signed);
        assert!(catalog_entry.schedule_eligible);
        let marketplace = crate::db::capabilities::list_marketplace(&db);
        let marketplace_entry = marketplace
            .iter()
            .find(|entry| entry["slug"] == "test-pack")
            .unwrap();
        assert_eq!(marketplace_entry["metadata"]["packVersion"], "1.0.0");
        assert_eq!(marketplace_entry["metadata"]["signed"], true);
        let pack_skill = crate::db::skill_catalog::list_skills(
            &db,
            "user-1",
            None,
            Some("codex"),
            "inspect test",
        )
        .into_iter()
        .find(|skill| skill["slug"] == "test-pack.inspect")
        .unwrap();
        assert_eq!(pack_skill["reviewState"], "approved");
        assert_eq!(pack_skill["signed"], true);
        assert_eq!(pack_skill["version"], "1.0.0");
        let pack_skill_id = pack_skill["id"].as_str().unwrap().to_string();
        assert!(
            crate::db::skill_catalog::install_skill(
                &db,
                &pack_skill_id,
                "user-1",
                None,
                None,
                true,
            )
            .is_err()
        );

        let mut second_envelope = envelope("2.0.0");
        second_envelope.manifest.tools[0].description = "Updated reader".into();
        let canonical = serde_json::to_vec(&second_envelope.manifest).unwrap();
        second_envelope.signature.value =
            STANDARD.encode(signing_key().sign(&canonical).to_bytes());
        let second =
            install_capability_pack_with_key(&db, &second_envelope, "admin", &key).unwrap();
        assert_eq!(second.active_version.as_deref(), Some("2.0.0"));
        assert_eq!(second.previous_version.as_deref(), Some("1.0.0"));
        assert!(
            install_capability_pack_with_key(&db, &envelope("1.5.0"), "admin", &key)
                .unwrap_err()
                .contains("rollback")
        );

        let rolled_back =
            rollback_capability_pack_with_key(&db, "flamel.test-pack", "admin", &key).unwrap();
        assert_eq!(rolled_back.active_version.as_deref(), Some("1.0.0"));
        assert_eq!(rolled_back.previous_version.as_deref(), Some("2.0.0"));

        crate::db::capabilities::connect(&db, "pack-test-pack", "user-1", None).unwrap();
        assert!(crate::db::capabilities::connection_has_exact_scopes(
            &db,
            "pack-test-pack",
            "user-1",
            None,
        ));
        crate::db::skill_catalog::install_skill(&db, &pack_skill_id, "user-1", None, None, true)
            .unwrap();
        assert!(
            crate::db::skill_catalog::active_skills(&db, "user-1", None, "codex")
                .iter()
                .any(|skill| skill["id"] == pack_skill_id)
        );
        assert!(
            crate::db::capabilities::validate_tool_arguments(
                &db,
                "pack-test-pack",
                "test.read",
                &json!({"id":"one"}),
            )
            .is_ok()
        );
        assert!(
            crate::db::capabilities::validate_tool_arguments(
                &db,
                "pack-test-pack",
                "test.read",
                &json!({"id":7}),
            )
            .is_err()
        );
        assert!(crate::db::capabilities::set_tool_enabled(
            &db,
            "pack-test-pack",
            "test.read",
            false,
        ));
        assert!(
            crate::db::capabilities::validate_tool_arguments(
                &db,
                "pack-test-pack",
                "test.read",
                &json!({"id":"one"}),
            )
            .is_err()
        );
        assert!(crate::db::capabilities::set_tool_enabled(
            &db,
            "pack-test-pack",
            "test.read",
            true,
        ));
        assert!(
            crate::db::capabilities::validate_credential_kind(&db, "pack-test-pack", "token",)
                .is_ok()
        );
        assert!(
            crate::db::capabilities::validate_credential_kind(
                &db,
                "pack-test-pack",
                "access_token",
            )
            .is_err()
        );
        crate::db::credentials::store(
            &db,
            "user-1",
            "pack-test-pack",
            "token",
            "sealed-value",
            &json!({}),
            None,
        )
        .unwrap();
        crate::db::capabilities::disconnect(&db, "pack-test-pack", "user-1", None);
        assert!(crate::db::credentials::encrypted(&db, "user-1", "pack-test-pack",).is_none());
        assert!(
            !crate::db::skill_catalog::active_skills(&db, "user-1", None, "codex")
                .iter()
                .any(|skill| skill["id"] == pack_skill_id)
        );
        crate::db::capabilities::connect(&db, "pack-test-pack", "user-1", None).unwrap();
        assert!(
            crate::db::skill_catalog::active_skills(&db, "user-1", None, "codex")
                .iter()
                .any(|skill| skill["id"] == pack_skill_id)
        );
        crate::db::credentials::store(
            &db,
            "user-1",
            "pack-test-pack",
            "token",
            "sealed-value",
            &json!({}),
            None,
        )
        .unwrap();
        let serialized =
            serde_json::to_string(&crate::db::capabilities::list_catalog(&db, "user-1", None))
                .unwrap();
        assert!(!serialized.contains("sealed-value"));
        let removed = uninstall_capability_pack(&db, "flamel.test-pack", "admin").unwrap();
        assert_eq!(removed.state, "uninstalled");
        assert!(removed.active_version.is_none());
        assert!(crate::db::credentials::encrypted(&db, "user-1", "pack-test-pack").is_none());
        assert!(
            crate::db::capabilities::list_catalog(&db, "user-1", None)
                .into_iter()
                .all(|entry| entry.id != "pack-test-pack")
        );
        assert!(
            crate::db::skill_catalog::list_skills(&db, "user-1", None, None, "")
                .into_iter()
                .all(|skill| skill["id"] != pack_skill_id)
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn installed_pack_health_task_and_version_state_survive_restart() {
        let (root, db) = database();
        let path = root.join("flamel.db");
        let key = signing_key().verifying_key().to_bytes();
        install_capability_pack_with_key(&db, &envelope("1.0.0"), "admin", &key).unwrap();
        crate::db::capabilities::connect(&db, "pack-test-pack", "user-1", None).unwrap();
        crate::db::capabilities::record_health(
            &db,
            "pack-test-pack",
            "user-1",
            None,
            true,
            12,
            1,
            None,
            None,
        );
        let task = crate::db::capabilities::create_task(
            &db,
            "pack-test-pack",
            None,
            "user-1",
            Some("session-1"),
            "test.read",
            &json!({"id":"one"}),
        );
        crate::db::capabilities::start_task(&db, &task.id);
        drop(db);

        let reopened = Db::open(path.to_string_lossy().as_ref());
        crate::db::capabilities::reconcile_interrupted_tasks(&reopened);
        let pack = list_pack_installations(&reopened).pop().unwrap();
        assert_eq!(pack.active_version.as_deref(), Some("1.0.0"));
        let catalog = crate::db::capabilities::list_catalog(&reopened, "user-1", None);
        let entry = catalog
            .into_iter()
            .find(|entry| entry.id == "pack-test-pack")
            .unwrap();
        assert_eq!(entry.status, "ready");
        assert_eq!(entry.latency_ms, Some(12));
        assert_eq!(
            crate::db::capabilities::list_tasks(&reopened, "user-1", 10)[0]["status"],
            "interrupted"
        );
        std::fs::remove_dir_all(root).ok();
    }
}
