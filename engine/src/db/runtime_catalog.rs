use crate::db::{Db, skill_catalog};
use futures::{StreamExt, stream};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use reqwest::header::{ETAG, IF_NONE_MATCH};
use rusqlite::{OptionalExtension, Transaction, params};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

pub const RUNTIME_CATALOG_AUDIENCE: &str = "flamel-runtime-catalog";
pub const RUNTIME_CATALOG_ISSUER: &str = "https://flamel.dev";
const RUNTIME_CATALOG_ORIGIN: &str = "https://flamel.dev";
const MANIFEST_MAX_BYTES: usize = 256 * 1024;
const ARTIFACT_MAX_BYTES: usize = 128 * 1024;
const CATALOG_MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_ARTIFACTS: usize = 128;
const MAX_REVOCATIONS: usize = 256;
const MANIFEST_MAX_AGE_SECONDS: u64 = 8 * 24 * 60 * 60;
const CLOCK_SKEW_SECONDS: u64 = 300;

const ARTIFACT_KINDS: &[&str] = &[
    "skill",
    "persona",
    "model_metadata",
    "marketplace_metadata",
    "prompt_overlay",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCatalogArtifactDescriptor {
    pub kind: String,
    pub slug: String,
    pub version: String,
    pub sha256: String,
    pub byte_size: usize,
    pub fetch_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCatalogManifest {
    pub schema_version: u32,
    pub channel: String,
    pub revision: u64,
    pub min_runtime: String,
    pub max_runtime: String,
    pub artifacts: Vec<RuntimeCatalogArtifactDescriptor>,
    #[serde(default)]
    pub revocations: Vec<String>,
    pub iss: String,
    pub aud: String,
    pub iat: u64,
    pub exp: u64,
}

#[derive(Clone, Debug)]
pub struct VerifiedRuntimeCatalog {
    pub manifest: RuntimeCatalogManifest,
    pub jws: String,
    pub key_id: String,
}

#[derive(Clone, Debug)]
pub struct DownloadedRuntimeArtifact {
    pub descriptor: RuntimeCatalogArtifactDescriptor,
    pub content: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeCatalogPersonaInput {
    schema_version: u32,
    kind: String,
    slug: String,
    version: String,
    description: String,
    content: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCatalogPersona {
    pub slug: String,
    pub version: String,
    pub description: String,
    pub content: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeCatalogMarketplaceMetadataInput {
    schema_version: u32,
    kind: String,
    slug: String,
    version: String,
    entries: Vec<RuntimeCatalogMarketplaceEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCatalogMarketplaceEntry {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub publisher: String,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeCatalogModelMetadataInput {
    schema_version: u32,
    kind: String,
    slug: String,
    version: String,
    models: Vec<RuntimeCatalogModelEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCatalogModelEntry {
    pub id: String,
    pub label: String,
    pub provider: String,
    pub reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCatalogStatus {
    pub channel: String,
    pub active_revision: Option<u64>,
    pub previous_revision: Option<u64>,
    pub highest_accepted_revision: u64,
    pub active_key_id: Option<String>,
    pub etag: Option<String>,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
    pub frozen: bool,
    pub stale: bool,
    pub age_seconds: Option<i64>,
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn exact_artifact_url(hash: &str) -> String {
    format!("{RUNTIME_CATALOG_ORIGIN}/api/runtime-catalog/artifacts/{hash}")
}

fn unix_now() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

fn now_text() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn validate_manifest_contract(
    manifest: &RuntimeCatalogManifest,
    key_id: &str,
    runtime_version: &str,
    now: u64,
    highest_accepted_revision: u64,
    trusted_revocations: &BTreeSet<String>,
) -> Result<(), String> {
    if manifest.schema_version != 1 {
        return Err("Runtime catalog schema version is not supported".to_string());
    }
    if !matches!(manifest.channel.as_str(), "stable" | "beta") {
        return Err("Runtime catalog channel is invalid".to_string());
    }
    if manifest.revision <= highest_accepted_revision {
        return Err("Runtime catalog revision is a replay or downgrade".to_string());
    }
    if manifest.iat > now.saturating_add(CLOCK_SKEW_SECONDS)
        || manifest.exp <= now
        || manifest.exp <= manifest.iat
        || manifest.exp.saturating_sub(manifest.iat) > MANIFEST_MAX_AGE_SECONDS
    {
        return Err("Runtime catalog freshness window is invalid".to_string());
    }
    let runtime =
        Version::parse(runtime_version).map_err(|_| "Runtime version is not valid semver")?;
    let minimum =
        Version::parse(&manifest.min_runtime).map_err(|_| "Minimum runtime is invalid semver")?;
    let maximum =
        Version::parse(&manifest.max_runtime).map_err(|_| "Maximum runtime is invalid semver")?;
    if minimum > maximum || runtime < minimum || runtime > maximum {
        return Err("Runtime catalog is incompatible with this server".to_string());
    }
    if manifest.artifacts.is_empty() || manifest.artifacts.len() > MAX_ARTIFACTS {
        return Err("Runtime catalog artifact count is outside the supported bounds".to_string());
    }
    if manifest.revocations.len() > MAX_REVOCATIONS {
        return Err("Runtime catalog revocation list is too large".to_string());
    }
    let revocations = manifest
        .revocations
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if revocations.len() != manifest.revocations.len()
        || revocations.contains(&format!("kid:{key_id}"))
        || trusted_revocations.contains(&format!("kid:{key_id}"))
    {
        return Err("Runtime catalog signing key is revoked".to_string());
    }

    let mut identities = BTreeSet::new();
    let mut hashes = BTreeSet::new();
    let mut total_bytes = 0usize;
    for artifact in &manifest.artifacts {
        Version::parse(&artifact.version)
            .map_err(|_| "Runtime catalog artifact version is invalid semver")?;
        if !ARTIFACT_KINDS.contains(&artifact.kind.as_str())
            || !valid_slug(&artifact.slug)
            || !valid_hash(&artifact.sha256)
            || artifact.byte_size == 0
            || artifact.byte_size > ARTIFACT_MAX_BYTES
            || artifact.fetch_url != exact_artifact_url(&artifact.sha256)
            || revocations.contains(&format!("sha256:{}", artifact.sha256))
            || trusted_revocations.contains(&format!("sha256:{}", artifact.sha256))
        {
            return Err("Runtime catalog artifact descriptor is invalid".to_string());
        }
        total_bytes = total_bytes
            .checked_add(artifact.byte_size)
            .ok_or("Runtime catalog byte count overflow")?;
        if total_bytes > CATALOG_MAX_BYTES {
            return Err("Runtime catalog exceeds the total byte limit".to_string());
        }
        if !identities.insert(format!(
            "{}:{}:{}",
            artifact.kind, artifact.slug, artifact.version
        )) {
            return Err("Runtime catalog artifact identity is duplicated".to_string());
        }
        if !hashes.insert(&artifact.sha256) {
            return Err("Runtime catalog artifact content is duplicated".to_string());
        }
    }
    Ok(())
}

pub fn verify_runtime_catalog(
    jws: &str,
    jwks: &JwkSet,
    runtime_version: &str,
    now: u64,
    highest_accepted_revision: u64,
    trusted_revocations: &BTreeSet<String>,
) -> Result<VerifiedRuntimeCatalog, String> {
    if jws.is_empty() || jws.len() > MANIFEST_MAX_BYTES {
        return Err("Runtime catalog manifest exceeds its byte limit".to_string());
    }
    let header = decode_header(jws).map_err(|error| format!("Runtime catalog header: {error}"))?;
    if header.alg != Algorithm::RS256 {
        return Err("Runtime catalog must use RS256".to_string());
    }
    let key_id = header
        .kid
        .clone()
        .ok_or("Runtime catalog manifest is missing a key id")?;
    let candidates = jwks
        .keys
        .iter()
        .filter(|key| key.common.key_id.as_deref() == Some(key_id.as_str()))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err("Runtime catalog signing key is not trusted".to_string());
    }

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[RUNTIME_CATALOG_ISSUER]);
    validation.set_audience(&[RUNTIME_CATALOG_AUDIENCE]);
    validation.validate_exp = false;
    let mut decoded = None;
    for jwk in candidates {
        let Ok(key) = DecodingKey::from_jwk(jwk) else {
            continue;
        };
        if let Ok(token) = decode::<RuntimeCatalogManifest>(jws, &key, &validation) {
            decoded = Some(token.claims);
            break;
        }
    }
    let manifest = decoded.ok_or("Runtime catalog signature validation failed")?;
    validate_manifest_contract(
        &manifest,
        &key_id,
        runtime_version,
        now,
        highest_accepted_revision,
        trusted_revocations,
    )?;
    Ok(VerifiedRuntimeCatalog {
        manifest,
        jws: jws.to_string(),
        key_id,
    })
}

pub fn validate_runtime_artifacts(
    manifest: &RuntimeCatalogManifest,
    contents: HashMap<String, Vec<u8>>,
) -> Result<Vec<DownloadedRuntimeArtifact>, String> {
    if contents.len() != manifest.artifacts.len() {
        return Err("Runtime catalog artifact set is partial or contains extras".to_string());
    }
    let mut total = 0usize;
    let mut validated = Vec::with_capacity(manifest.artifacts.len());
    for descriptor in &manifest.artifacts {
        let content = contents
            .get(&descriptor.sha256)
            .ok_or("Runtime catalog artifact is missing")?;
        if content.len() != descriptor.byte_size || content.len() > ARTIFACT_MAX_BYTES {
            return Err("Runtime catalog artifact byte size does not match".to_string());
        }
        total = total
            .checked_add(content.len())
            .ok_or("Runtime catalog byte count overflow")?;
        if total > CATALOG_MAX_BYTES {
            return Err("Runtime catalog exceeds the total byte limit".to_string());
        }
        let hash = format!("{:x}", Sha256::digest(content));
        if hash != descriptor.sha256 {
            return Err("Runtime catalog artifact hash does not match".to_string());
        }
        if content.contains(&0) || std::str::from_utf8(content).is_err() {
            return Err("Runtime catalog artifacts must be NUL-free UTF-8".to_string());
        }
        validated.push(DownloadedRuntimeArtifact {
            descriptor: descriptor.clone(),
            content: content.clone(),
        });
    }
    Ok(validated)
}

fn parse_persona_artifact(
    kind: &str,
    slug: &str,
    version: &str,
    content: &[u8],
) -> Result<RuntimeCatalogPersonaInput, String> {
    let persona = serde_json::from_slice::<RuntimeCatalogPersonaInput>(content)
        .map_err(|error| format!("Runtime catalog persona parse: {error}"))?;
    if persona.schema_version != 1
        || persona.kind != "persona"
        || kind != "persona"
        || persona.slug != slug
        || persona.version != version
        || !valid_slug(&persona.slug)
        || Version::parse(&persona.version).is_err()
        || persona.description.trim().is_empty()
        || persona.content.trim().is_empty()
    {
        return Err("Runtime catalog persona identity or content is invalid".to_string());
    }
    Ok(persona)
}

fn valid_display_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value == value.trim() && value.len() <= max_bytes
}

fn parse_marketplace_metadata_artifact(
    kind: &str,
    slug: &str,
    version: &str,
    content: &[u8],
) -> Result<RuntimeCatalogMarketplaceMetadataInput, String> {
    let metadata = serde_json::from_slice::<RuntimeCatalogMarketplaceMetadataInput>(content)
        .map_err(|error| format!("Runtime catalog marketplace metadata parse: {error}"))?;
    if metadata.schema_version != 1
        || kind != "marketplace_metadata"
        || metadata.kind != kind
        || slug != "official-marketplace"
        || metadata.slug != slug
        || metadata.version != version
        || Version::parse(&metadata.version).is_err()
        || metadata.entries.is_empty()
        || metadata.entries.len() > 64
    {
        return Err("Runtime catalog marketplace metadata identity is invalid".to_string());
    }
    let official_slugs = [
        "sourcegraph",
        "sentry",
        "grafana",
        "github",
        "gitlab",
        "aws",
        "kubernetes",
        "terraform",
    ];
    let mut slugs = BTreeSet::new();
    for entry in &metadata.entries {
        if !official_slugs.contains(&entry.slug.as_str())
            || !slugs.insert(&entry.slug)
            || !valid_display_text(&entry.title, 96)
            || !valid_display_text(&entry.description, 512)
            || !valid_display_text(&entry.category, 64)
            || !valid_display_text(&entry.publisher, 96)
            || !matches!(
                entry.status.as_str(),
                "available" | "preview" | "unavailable" | "deprecated"
            )
        {
            return Err("Runtime catalog marketplace metadata content is invalid".to_string());
        }
    }
    Ok(metadata)
}

fn compiled_model_contract(id: &str) -> Option<(&'static str, &'static [&'static str])> {
    const CODEX_EXTENDED: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];
    const CODEX_STANDARD: &[&str] = &["low", "medium", "high", "xhigh"];
    const NO_REASONING: &[&str] = &[];
    match id {
        "gpt-6-astra" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna" | "gpt-5.6" => {
            Some(("codex", CODEX_EXTENDED))
        }
        "gpt-5.5"
        | "gpt-5.5-pro"
        | "gpt-5.4"
        | "gpt-5.4-pro"
        | "gpt-5.4-mini"
        | "gpt-5.3-codex"
        | "gpt-5.3-codex-spark"
        | "gpt-5.2"
        | "gpt-5.2-pro" => Some(("codex", CODEX_STANDARD)),
        "claude-sonnet-4-6" | "claude-opus-4-7" | "claude-opus-4-6" | "claude-haiku-4-5" => {
            Some(("claude", NO_REASONING))
        }
        _ => None,
    }
}

fn parse_model_metadata_artifact(
    kind: &str,
    slug: &str,
    version: &str,
    content: &[u8],
) -> Result<RuntimeCatalogModelMetadataInput, String> {
    let metadata = serde_json::from_slice::<RuntimeCatalogModelMetadataInput>(content)
        .map_err(|error| format!("Runtime catalog model metadata parse: {error}"))?;
    if metadata.schema_version != 1
        || kind != "model_metadata"
        || metadata.kind != kind
        || slug != "provider-models"
        || metadata.slug != slug
        || metadata.version != version
        || Version::parse(&metadata.version).is_err()
        || metadata.models.is_empty()
        || metadata.models.len() > 64
    {
        return Err("Runtime catalog model metadata identity is invalid".to_string());
    }
    let mut ids = BTreeSet::new();
    for model in &metadata.models {
        let Some((provider, allowed_efforts)) = compiled_model_contract(&model.id) else {
            return Err("Runtime catalog model metadata contains an unknown model".to_string());
        };
        let efforts = model
            .reasoning_efforts
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if !ids.insert(&model.id)
            || model.provider != provider
            || !valid_display_text(&model.label, 96)
            || efforts.len() != model.reasoning_efforts.len()
            || efforts
                .iter()
                .any(|effort| !allowed_efforts.contains(effort))
            || (provider == "codex"
                && (efforts.is_empty() || model.default_reasoning_effort.is_none()))
            || (provider == "claude"
                && (!efforts.is_empty() || model.default_reasoning_effort.is_some()))
            || model
                .default_reasoning_effort
                .as_deref()
                .is_some_and(|default| !efforts.contains(default))
        {
            return Err("Runtime catalog model metadata content is invalid".to_string());
        }
    }
    Ok(metadata)
}

fn import_artifact(
    transaction: &Transaction<'_>,
    revision: u64,
    artifact: &DownloadedRuntimeArtifact,
) -> Result<(), String> {
    match artifact.descriptor.kind.as_str() {
        "skill" => {
            let skill = serde_json::from_slice::<skill_catalog::RuntimeCatalogSkillInput>(
                &artifact.content,
            )
            .map_err(|error| format!("Runtime catalog skill parse: {error}"))?;
            if skill.kind != "skill"
                || skill.slug != artifact.descriptor.slug
                || skill.version != artifact.descriptor.version
            {
                return Err(
                    "Runtime catalog skill identity does not match its descriptor".to_string(),
                );
            }
            skill_catalog::import_runtime_catalog_skill(transaction, &skill, revision)?;
        }
        "persona" => {
            parse_persona_artifact(
                &artifact.descriptor.kind,
                &artifact.descriptor.slug,
                &artifact.descriptor.version,
                &artifact.content,
            )?;
        }
        "marketplace_metadata" => {
            parse_marketplace_metadata_artifact(
                &artifact.descriptor.kind,
                &artifact.descriptor.slug,
                &artifact.descriptor.version,
                &artifact.content,
            )?;
        }
        "model_metadata" => {
            parse_model_metadata_artifact(
                &artifact.descriptor.kind,
                &artifact.descriptor.slug,
                &artifact.descriptor.version,
                &artifact.content,
            )?;
        }
        _ => {}
    }
    Ok(())
}

pub fn active_runtime_personas(db: &Db) -> Result<Vec<RuntimeCatalogPersona>, String> {
    let conn = db.conn();
    let active_revision = conn
        .query_row(
            "SELECT active_revision FROM runtime_catalog_state WHERE id=1",
            [],
            |row| row.get::<_, Option<u64>>(0),
        )
        .map_err(|error| error.to_string())?;
    let Some(active_revision) = active_revision else {
        return Ok(Vec::new());
    };
    let mut statement = conn
        .prepare(
            "SELECT a.kind,a.slug,a.version,a.sha256,a.content
             FROM runtime_catalog_snapshot_artifacts sa
             JOIN runtime_catalog_artifacts a ON a.sha256=sa.sha256
             WHERE sa.revision=?1 AND a.kind='persona'
             ORDER BY sa.position",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([active_revision], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut personas = Vec::new();
    for row in rows {
        let (kind, slug, version, sha256, content) = row.map_err(|error| error.to_string())?;
        let persona = parse_persona_artifact(&kind, &slug, &version, &content)?;
        personas.push(RuntimeCatalogPersona {
            slug: persona.slug,
            version: persona.version,
            description: persona.description,
            content: persona.content,
            sha256,
        });
    }
    Ok(personas)
}

fn active_runtime_artifact(
    db: &Db,
    kind: &str,
    slug: &str,
) -> Result<Option<(String, String, String, Vec<u8>)>, String> {
    db.conn()
        .query_row(
            "SELECT a.kind,a.slug,a.version,a.content
             FROM runtime_catalog_state state
             JOIN runtime_catalog_snapshot_artifacts sa ON sa.revision=state.active_revision
             JOIN runtime_catalog_artifacts a ON a.sha256=sa.sha256
             WHERE state.id=1 AND a.kind=?1 AND a.slug=?2
             ORDER BY sa.position DESC LIMIT 1",
            params![kind, slug],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| error.to_string())
}

pub fn active_runtime_marketplace_metadata(
    db: &Db,
) -> Result<Vec<RuntimeCatalogMarketplaceEntry>, String> {
    let Some((kind, slug, version, content)) =
        active_runtime_artifact(db, "marketplace_metadata", "official-marketplace")?
    else {
        return Ok(Vec::new());
    };
    Ok(parse_marketplace_metadata_artifact(&kind, &slug, &version, &content)?.entries)
}

pub fn active_runtime_model_metadata(db: &Db) -> Result<Vec<RuntimeCatalogModelEntry>, String> {
    let Some((kind, slug, version, content)) =
        active_runtime_artifact(db, "model_metadata", "provider-models")?
    else {
        return Ok(Vec::new());
    };
    Ok(parse_model_metadata_artifact(&kind, &slug, &version, &content)?.models)
}

pub fn activate_runtime_catalog(
    db: &Db,
    verified: &VerifiedRuntimeCatalog,
    artifacts: &[DownloadedRuntimeArtifact],
    etag: Option<&str>,
) -> Result<RuntimeCatalogStatus, String> {
    if artifacts.len() != verified.manifest.artifacts.len() {
        return Err("Runtime catalog activation is missing artifacts".to_string());
    }
    let now = now_text();
    let manifest_json =
        serde_json::to_string(&verified.manifest).map_err(|error| error.to_string())?;
    let mut conn = db.conn();
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    let (current, highest_accepted_revision, trusted_revocations_json) = transaction
        .query_row(
            "SELECT active_revision,highest_accepted_revision,trusted_revocations_json
             FROM runtime_catalog_state WHERE id=1",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<u64>>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;
    if verified.manifest.revision <= highest_accepted_revision {
        return Err("Runtime catalog revision is a replay or concurrent downgrade".to_string());
    }
    let mut trusted_revocations =
        serde_json::from_str::<BTreeSet<String>>(&trusted_revocations_json)
            .map_err(|error| format!("Stored Runtime Catalog revocations: {error}"))?;
    trusted_revocations.extend(verified.manifest.revocations.iter().cloned());
    let trusted_revocations_json =
        serde_json::to_string(&trusted_revocations).map_err(|error| error.to_string())?;
    let existing_jws = transaction
        .query_row(
            "SELECT manifest_jws FROM runtime_catalog_snapshots WHERE revision=?1",
            [verified.manifest.revision],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if existing_jws
        .as_deref()
        .is_some_and(|stored| stored != verified.jws)
    {
        return Err("Immutable runtime catalog revision changed".to_string());
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO runtime_catalog_snapshots
             (revision,channel,manifest_jws,manifest_json,key_id,verified_at)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                verified.manifest.revision,
                verified.manifest.channel,
                verified.jws,
                manifest_json,
                verified.key_id,
                now,
            ],
        )
        .map_err(|error| error.to_string())?;

    for (position, artifact) in artifacts.iter().enumerate() {
        transaction
            .execute(
                "INSERT OR IGNORE INTO runtime_catalog_artifacts
                 (sha256,kind,slug,version,byte_size,content,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    artifact.descriptor.sha256,
                    artifact.descriptor.kind,
                    artifact.descriptor.slug,
                    artifact.descriptor.version,
                    artifact.descriptor.byte_size as u64,
                    artifact.content,
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;
        let stored_matches = transaction
            .query_row(
                "SELECT kind,slug,version,byte_size,content
                 FROM runtime_catalog_artifacts WHERE sha256=?1",
                [&artifact.descriptor.sha256],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, u64>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?;
        if stored_matches.0 != artifact.descriptor.kind
            || stored_matches.1 != artifact.descriptor.slug
            || stored_matches.2 != artifact.descriptor.version
            || stored_matches.3 != artifact.descriptor.byte_size as u64
            || stored_matches.4 != artifact.content
        {
            return Err("Runtime catalog artifact hash collision detected".to_string());
        }
        import_artifact(&transaction, verified.manifest.revision, artifact)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO runtime_catalog_snapshot_artifacts
                 (revision,position,sha256) VALUES (?1,?2,?3)",
                params![
                    verified.manifest.revision,
                    position as u64,
                    artifact.descriptor.sha256,
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    transaction
        .execute(
            "UPDATE runtime_catalog_state SET
                channel=?1,previous_revision=?2,active_revision=?3,
                highest_accepted_revision=MAX(highest_accepted_revision,?3),
                active_key_id=?4,etag=?5,trusted_revocations_json=?6,
                last_attempt_at=?7,last_success_at=?7,last_error=NULL,updated_at=?7
             WHERE id=1",
            params![
                verified.manifest.channel,
                current,
                verified.manifest.revision,
                verified.key_id,
                etag,
                trusted_revocations_json,
                now,
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    drop(conn);
    Ok(runtime_catalog_status(db))
}

pub fn runtime_catalog_status(db: &Db) -> RuntimeCatalogStatus {
    let conn = db.conn();
    conn.query_row(
        "SELECT channel,active_revision,previous_revision,highest_accepted_revision,
                active_key_id,etag,last_attempt_at,last_success_at,last_error,frozen
         FROM runtime_catalog_state WHERE id=1",
        [],
        |row| {
            let last_success_at: Option<String> = row.get(7)?;
            let age_seconds = last_success_at.as_deref().and_then(|value| {
                chrono::DateTime::parse_from_rfc3339(value)
                    .ok()
                    .map(|date| {
                        (chrono::Utc::now() - date.with_timezone(&chrono::Utc))
                            .num_seconds()
                            .max(0)
                    })
            });
            let active_revision = row.get(1)?;
            Ok(RuntimeCatalogStatus {
                channel: row.get(0)?,
                active_revision,
                previous_revision: row.get(2)?,
                highest_accepted_revision: row.get(3)?,
                active_key_id: row.get(4)?,
                etag: row.get(5)?,
                last_attempt_at: row.get(6)?,
                last_success_at,
                last_error: row.get(8)?,
                frozen: row.get::<_, i64>(9)? == 1,
                stale: active_revision.is_none() || age_seconds.is_none_or(|age| age > 3_600),
                age_seconds,
            })
        },
    )
    .unwrap_or(RuntimeCatalogStatus {
        channel: "stable".to_string(),
        active_revision: None,
        previous_revision: None,
        highest_accepted_revision: 0,
        active_key_id: None,
        etag: None,
        last_attempt_at: None,
        last_success_at: None,
        last_error: Some("Runtime catalog state is unavailable".to_string()),
        frozen: true,
        stale: true,
        age_seconds: None,
    })
}

pub fn record_runtime_catalog_error(db: &Db, error: &str) {
    let now = now_text();
    let _ = db.conn().execute(
        "UPDATE runtime_catalog_state
         SET last_attempt_at=?1,last_error=?2,updated_at=?1 WHERE id=1",
        params![now, error.chars().take(500).collect::<String>()],
    );
}

fn record_not_modified(db: &Db, etag: Option<&str>) -> RuntimeCatalogStatus {
    let now = now_text();
    let _ = db.conn().execute(
        "UPDATE runtime_catalog_state SET
            etag=COALESCE(?1,etag),last_attempt_at=?2,last_success_at=?2,
            last_error=NULL,updated_at=?2 WHERE id=1",
        params![etag, now],
    );
    runtime_catalog_status(db)
}

pub fn set_runtime_catalog_frozen(db: &Db, frozen: bool) -> RuntimeCatalogStatus {
    let now = now_text();
    let _ = db.conn().execute(
        "UPDATE runtime_catalog_state SET frozen=?1,updated_at=?2 WHERE id=1",
        params![i64::from(frozen), now],
    );
    runtime_catalog_status(db)
}

pub fn set_runtime_catalog_channel(db: &Db, channel: &str) -> Result<RuntimeCatalogStatus, String> {
    if !matches!(channel, "stable" | "beta") {
        return Err("Runtime catalog channel must be stable or beta".to_string());
    }
    let now = now_text();
    db.conn()
        .execute(
            "UPDATE runtime_catalog_state SET channel=?1,etag=NULL,updated_at=?2 WHERE id=1",
            params![channel, now],
        )
        .map_err(|error| error.to_string())?;
    Ok(runtime_catalog_status(db))
}

pub fn rollback_runtime_catalog(db: &Db) -> Result<RuntimeCatalogStatus, String> {
    let status = runtime_catalog_status(db);
    let revision = status
        .previous_revision
        .ok_or("No previous verified runtime catalog snapshot is available")?;
    let conn = db.conn();
    let manifest_json = conn
        .query_row(
            "SELECT manifest_json FROM runtime_catalog_snapshots WHERE revision=?1",
            [revision],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| error.to_string())?;
    let manifest: RuntimeCatalogManifest =
        serde_json::from_str(&manifest_json).map_err(|error| error.to_string())?;
    let mut artifacts = Vec::new();
    for descriptor in &manifest.artifacts {
        let content = conn
            .query_row(
                "SELECT content FROM runtime_catalog_artifacts WHERE sha256=?1",
                [&descriptor.sha256],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .map_err(|error| error.to_string())?;
        artifacts.push(DownloadedRuntimeArtifact {
            descriptor: descriptor.clone(),
            content,
        });
    }
    drop(conn);

    let now = now_text();
    let mut conn = db.conn();
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    for artifact in &artifacts {
        import_artifact(&transaction, revision, artifact)?;
    }
    transaction
        .execute(
            "UPDATE runtime_catalog_state SET
                active_revision=?1,previous_revision=?2,channel=?3,
                active_key_id=(SELECT key_id FROM runtime_catalog_snapshots WHERE revision=?1),
                frozen=1,last_attempt_at=?4,last_success_at=?4,last_error=NULL,updated_at=?4
             WHERE id=1",
            params![revision, status.active_revision, manifest.channel, now],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    drop(conn);
    Ok(runtime_catalog_status(db))
}

async fn bounded_response(response: reqwest::Response, maximum: usize) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        return Err("Runtime catalog response exceeds its byte limit".to_string());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("Runtime catalog read: {error}"))?;
        if body.len().saturating_add(chunk.len()) > maximum {
            return Err("Runtime catalog response exceeds its byte limit".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn fetch_artifact(
    client: reqwest::Client,
    descriptor: RuntimeCatalogArtifactDescriptor,
) -> Result<(String, Vec<u8>), String> {
    let response = client
        .get(&descriptor.fetch_url)
        .send()
        .await
        .map_err(|error| format!("Runtime catalog artifact fetch: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Runtime catalog artifact returned HTTP {}",
            response.status()
        ));
    }
    let body = bounded_response(response, descriptor.byte_size.min(ARTIFACT_MAX_BYTES)).await?;
    Ok((descriptor.sha256, body))
}

fn trusted_runtime_catalog_revocations(db: &Db) -> Result<BTreeSet<String>, String> {
    let serialized = db
        .conn()
        .query_row(
            "SELECT trusted_revocations_json FROM runtime_catalog_state WHERE id=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&serialized)
        .map_err(|error| format!("Stored Runtime Catalog revocations: {error}"))
}

fn runtime_catalog_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Runtime catalog client: {error}"))
}

pub async fn sync_runtime_catalog(db: &Db, jwks: &JwkSet) -> Result<RuntimeCatalogStatus, String> {
    tokio::time::timeout(
        Duration::from_secs(30),
        sync_runtime_catalog_inner(db, jwks),
    )
    .await
    .map_err(|_| "Runtime catalog sync exceeded its 30-second deadline".to_string())?
}

async fn sync_runtime_catalog_inner(
    db: &Db,
    jwks: &JwkSet,
) -> Result<RuntimeCatalogStatus, String> {
    let before = runtime_catalog_status(db);
    if before.frozen {
        return Ok(before);
    }
    let client = runtime_catalog_client(Duration::from_secs(10))?;
    let mut request = client.get(format!(
        "{RUNTIME_CATALOG_ORIGIN}/api/runtime-catalog/{}",
        before.channel
    ));
    if let Some(etag) = before.etag.as_deref() {
        request = request.header(IF_NONE_MATCH, etag);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Runtime catalog fetch: {error}"))?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok());
        return Ok(record_not_modified(db, etag));
    }
    if !response.status().is_success() {
        return Err(format!(
            "Runtime catalog returned HTTP {}",
            response.status()
        ));
    }
    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = bounded_response(response, MANIFEST_MAX_BYTES).await?;
    let jws = std::str::from_utf8(&body)
        .map_err(|_| "Runtime catalog manifest is not UTF-8")?
        .trim();
    let trusted_revocations = trusted_runtime_catalog_revocations(db)?;
    let verified = verify_runtime_catalog(
        jws,
        jwks,
        env!("CARGO_PKG_VERSION"),
        unix_now(),
        before.highest_accepted_revision,
        &trusted_revocations,
    )?;

    let downloads = stream::iter(verified.manifest.artifacts.clone())
        .map(|descriptor| fetch_artifact(client.clone(), descriptor))
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
    let mut contents = HashMap::new();
    for result in downloads {
        let (hash, content) = result?;
        contents.insert(hash, content);
    }
    let artifacts = validate_runtime_artifacts(&verified.manifest, contents)?;
    activate_runtime_catalog(db, &verified, &artifacts, etag.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use std::fs;
    use std::path::PathBuf;

    const VALID_JWS: &str = include_str!("../../../fixtures/runtime-catalog/valid-manifest.jws");
    const BAD_SIGNATURE_JWS: &str =
        include_str!("../../../fixtures/runtime-catalog/bad-signature-manifest.jws");
    const INCOMPATIBLE_JWS: &str =
        include_str!("../../../fixtures/runtime-catalog/incompatible-manifest.jws");
    const REPLAY_JWS: &str = include_str!("../../../fixtures/runtime-catalog/replay-manifest.jws");
    const REVOKED_KEY_JWS: &str =
        include_str!("../../../fixtures/runtime-catalog/revoked-key-manifest.jws");
    const ROLLBACK_JWS: &str =
        include_str!("../../../fixtures/runtime-catalog/rollback-manifest.jws");
    const ARTIFACT: &[u8] = include_bytes!("../../../fixtures/runtime-catalog/skill.json");
    const PRIVATE_KEY: &str =
        include_str!("../../../fixtures/runtime-catalog/fixture-private-key.pem");
    const FIXED_NOW: u64 = 1_787_600_000;

    fn jwks() -> JwkSet {
        serde_json::from_str(include_str!("../../../fixtures/runtime-catalog/jwks.json")).unwrap()
    }

    fn verified(jws: &str, highest: u64) -> VerifiedRuntimeCatalog {
        verify_runtime_catalog(
            jws.trim(),
            &jwks(),
            "0.1.0",
            FIXED_NOW,
            highest,
            &BTreeSet::new(),
        )
        .unwrap()
    }

    fn downloaded(value: &VerifiedRuntimeCatalog) -> Vec<DownloadedRuntimeArtifact> {
        let mut contents = HashMap::new();
        contents.insert(
            value.manifest.artifacts[0].sha256.clone(),
            ARTIFACT.to_vec(),
        );
        validate_runtime_artifacts(&value.manifest, contents).unwrap()
    }

    fn database() -> Db {
        Db::open(":memory:")
    }

    fn signed_catalog(
        revision: u64,
        highest: u64,
        artifacts: Vec<(&str, &str, &str, Vec<u8>)>,
    ) -> (VerifiedRuntimeCatalog, Vec<DownloadedRuntimeArtifact>) {
        let mut manifest = verified(VALID_JWS, 40).manifest;
        manifest.revision = revision;
        manifest.artifacts = artifacts
            .iter()
            .map(|(kind, slug, version, content)| {
                let sha256 = format!("{:x}", Sha256::digest(content));
                RuntimeCatalogArtifactDescriptor {
                    kind: (*kind).to_string(),
                    slug: (*slug).to_string(),
                    version: (*version).to_string(),
                    byte_size: content.len(),
                    fetch_url: exact_artifact_url(&sha256),
                    sha256,
                }
            })
            .collect();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("runtime-catalog-fixture-1".to_string());
        let jws = encode(
            &header,
            &manifest,
            &EncodingKey::from_rsa_pem(PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap();
        let verified =
            verify_runtime_catalog(&jws, &jwks(), "0.1.0", FIXED_NOW, highest, &BTreeSet::new())
                .unwrap();
        let contents = artifacts
            .into_iter()
            .zip(&verified.manifest.artifacts)
            .map(|((_, _, _, content), descriptor)| (descriptor.sha256.clone(), content))
            .collect();
        let downloaded = validate_runtime_artifacts(&verified.manifest, contents).unwrap();
        (verified, downloaded)
    }

    #[test]
    fn shared_manifest_verifies_and_negative_signatures_fail_closed() {
        let valid = verified(VALID_JWS, 40);
        assert_eq!(valid.manifest.revision, 41);
        assert_eq!(valid.key_id, "runtime-catalog-fixture-1");
        assert!(
            verify_runtime_catalog(
                BAD_SIGNATURE_JWS.trim(),
                &jwks(),
                "0.1.0",
                FIXED_NOW,
                40,
                &BTreeSet::new(),
            )
            .is_err()
        );
        assert!(
            verify_runtime_catalog(
                INCOMPATIBLE_JWS.trim(),
                &jwks(),
                "0.1.0",
                FIXED_NOW,
                41,
                &BTreeSet::new(),
            )
            .is_err()
        );
        assert!(
            verify_runtime_catalog(
                REVOKED_KEY_JWS.trim(),
                &jwks(),
                "0.1.0",
                FIXED_NOW,
                41,
                &BTreeSet::new(),
            )
            .is_err()
        );
        assert!(
            verify_runtime_catalog(
                REPLAY_JWS.trim(),
                &jwks(),
                "0.1.0",
                FIXED_NOW,
                41,
                &BTreeSet::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn artifact_hash_or_partial_fetch_never_activates() {
        let db = database();
        let valid = verified(VALID_JWS, 40);
        let mut wrong = HashMap::new();
        wrong.insert(
            valid.manifest.artifacts[0].sha256.clone(),
            b"tampered".to_vec(),
        );
        assert!(validate_runtime_artifacts(&valid.manifest, wrong).is_err());

        let mut invalid_import = downloaded(&valid);
        invalid_import[0].content = b"{}".to_vec();
        assert!(activate_runtime_catalog(&db, &valid, &invalid_import, None).is_err());
        let persisted: (u64, u64) = db
            .conn()
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM runtime_catalog_snapshots),
                    (SELECT COUNT(*) FROM runtime_catalog_artifacts)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(persisted, (0, 0));
        assert_eq!(runtime_catalog_status(&db).active_revision, None);
    }

    #[test]
    fn activation_is_atomic_persistent_and_locally_rollbackable() {
        let path = std::env::temp_dir().join(format!(
            "flamel-runtime-catalog-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Db::open(path.to_str().unwrap());
        let valid = verified(VALID_JWS, 40);
        activate_runtime_catalog(&db, &valid, &downloaded(&valid), Some("\"beta-41\"")).unwrap();
        assert_eq!(runtime_catalog_status(&db).active_revision, Some(41));
        drop(db);

        let db = Db::open(path.to_str().unwrap());
        assert_eq!(runtime_catalog_status(&db).active_revision, Some(41));
        let rollback = verified(ROLLBACK_JWS, 41);
        activate_runtime_catalog(
            &db,
            &rollback,
            &downloaded(&rollback),
            Some("\"stable-43\""),
        )
        .unwrap();
        let rolled_back = rollback_runtime_catalog(&db).unwrap();
        assert_eq!(rolled_back.active_revision, Some(41));
        assert_eq!(rolled_back.highest_accepted_revision, 43);
        assert!(rolled_back.frozen);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn invalid_expired_replayed_and_failed_downloads_keep_the_last_known_good() {
        let db = database();
        let valid = verified(VALID_JWS, 40);
        activate_runtime_catalog(&db, &valid, &downloaded(&valid), Some("\"beta-41\"")).unwrap();

        let mut expired_manifest = valid.manifest.clone();
        expired_manifest.revision = 42;
        expired_manifest.iat = FIXED_NOW.saturating_sub(3_600);
        expired_manifest.exp = FIXED_NOW;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("runtime-catalog-fixture-1".to_string());
        let expired_jws = encode(
            &header,
            &expired_manifest,
            &EncodingKey::from_rsa_pem(PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap();

        for rejected in [
            BAD_SIGNATURE_JWS.trim(),
            REPLAY_JWS.trim(),
            expired_jws.as_str(),
        ] {
            assert!(
                verify_runtime_catalog(
                    rejected,
                    &jwks(),
                    "0.1.0",
                    FIXED_NOW,
                    41,
                    &BTreeSet::new(),
                )
                .is_err()
            );
            assert_eq!(runtime_catalog_status(&db).active_revision, Some(41));
        }

        let warm = record_not_modified(&db, Some("\"beta-41\""));
        assert_eq!(warm.active_revision, Some(41));
        assert!(warm.last_error.is_none());
        record_runtime_catalog_error(&db, "offline");
        let offline = runtime_catalog_status(&db);
        assert_eq!(offline.active_revision, Some(41));
        assert_eq!(offline.last_error.as_deref(), Some("offline"));
    }

    #[test]
    fn mutable_metadata_is_presentation_only_narrowing_and_rollback_safe() {
        let marketplace_source = parse_marketplace_metadata_artifact(
            "marketplace_metadata",
            "official-marketplace",
            "1.0.0",
            include_bytes!("../../../runtime-catalog/marketplace-metadata.json"),
        )
        .unwrap();
        let model_source = parse_model_metadata_artifact(
            "model_metadata",
            "provider-models",
            "1.0.1",
            include_bytes!("../../../runtime-catalog/model-metadata.json"),
        )
        .unwrap();
        assert_eq!(marketplace_source.entries.len(), 8);
        assert_eq!(model_source.models.len(), 18);

        let db = database();
        crate::db::capabilities::seed_marketplace(&db);
        let compiled = crate::db::capabilities::list_marketplace(&db);
        let compiled_sourcegraph = compiled
            .iter()
            .find(|entry| entry["slug"] == "sourcegraph")
            .unwrap();

        let marketplace_v1 = br#"{
            "schemaVersion":1,
            "kind":"marketplace_metadata",
            "slug":"official-marketplace",
            "version":"1.0.0",
            "entries":[{
                "slug":"sourcegraph",
                "title":"Sourcegraph Cloud",
                "description":"Current code intelligence copy",
                "category":"code-intelligence",
                "publisher":"Sourcegraph",
                "status":"preview"
            }]
        }"#
        .to_vec();
        let models_v1 = br#"{
            "schemaVersion":1,
            "kind":"model_metadata",
            "slug":"provider-models",
            "version":"1.0.0",
            "models":[{
                "id":"gpt-5.5",
                "label":"GPT-5.5 Current",
                "provider":"codex",
                "reasoningEfforts":["high"],
                "defaultReasoningEffort":"high"
            }]
        }"#
        .to_vec();
        let (catalog_v1, artifacts_v1) = signed_catalog(
            42,
            41,
            vec![
                (
                    "marketplace_metadata",
                    "official-marketplace",
                    "1.0.0",
                    marketplace_v1,
                ),
                ("model_metadata", "provider-models", "1.0.0", models_v1),
            ],
        );
        activate_runtime_catalog(&db, &catalog_v1, &artifacts_v1, None).unwrap();

        let overlaid = crate::db::capabilities::list_marketplace(&db);
        let sourcegraph = overlaid
            .iter()
            .find(|entry| entry["slug"] == "sourcegraph")
            .unwrap();
        assert_eq!(sourcegraph["title"], "Sourcegraph Cloud");
        assert_eq!(sourcegraph["status"], "preview");
        for authority_field in ["remoteUrl", "authKind", "permissions", "transport"] {
            assert_eq!(
                sourcegraph[authority_field], compiled_sourcegraph[authority_field],
                "{authority_field} must remain compiled"
            );
        }
        let models = active_runtime_model_metadata(&db).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-5.5");
        assert_eq!(models[0].reasoning_efforts, vec!["high".to_string()]);

        let forbidden_marketplace = br#"{
            "schemaVersion":1,
            "kind":"marketplace_metadata",
            "slug":"official-marketplace",
            "version":"2.0.0",
            "entries":[{
                "slug":"sourcegraph",
                "title":"Unsafe",
                "description":"Unsafe authority injection",
                "category":"code-intelligence",
                "publisher":"Sourcegraph",
                "status":"available",
                "remoteUrl":"https://attacker.invalid/mcp"
            }]
        }"#
        .to_vec();
        let (forbidden_catalog, forbidden_artifacts) = signed_catalog(
            43,
            42,
            vec![(
                "marketplace_metadata",
                "official-marketplace",
                "2.0.0",
                forbidden_marketplace,
            )],
        );
        assert!(
            activate_runtime_catalog(&db, &forbidden_catalog, &forbidden_artifacts, None).is_err()
        );
        assert_eq!(runtime_catalog_status(&db).active_revision, Some(42));

        let unknown_model = br#"{
            "schemaVersion":1,
            "kind":"model_metadata",
            "slug":"provider-models",
            "version":"2.0.0",
            "models":[{
                "id":"untrusted-model",
                "label":"Untrusted",
                "provider":"codex",
                "reasoningEfforts":["high"],
                "defaultReasoningEffort":"high"
            }]
        }"#
        .to_vec();
        let (unknown_catalog, unknown_artifacts) = signed_catalog(
            43,
            42,
            vec![("model_metadata", "provider-models", "2.0.0", unknown_model)],
        );
        assert!(activate_runtime_catalog(&db, &unknown_catalog, &unknown_artifacts, None).is_err());
        assert_eq!(runtime_catalog_status(&db).active_revision, Some(42));

        let marketplace_v2 = br#"{
            "schemaVersion":1,
            "kind":"marketplace_metadata",
            "slug":"official-marketplace",
            "version":"2.0.0",
            "entries":[{
                "slug":"sourcegraph",
                "title":"Sourcegraph Next",
                "description":"Next code intelligence copy",
                "category":"code-intelligence",
                "publisher":"Sourcegraph",
                "status":"available"
            }]
        }"#
        .to_vec();
        let (catalog_v2, artifacts_v2) = signed_catalog(
            43,
            42,
            vec![(
                "marketplace_metadata",
                "official-marketplace",
                "2.0.0",
                marketplace_v2,
            )],
        );
        activate_runtime_catalog(&db, &catalog_v2, &artifacts_v2, None).unwrap();
        assert_eq!(
            crate::db::capabilities::list_marketplace(&db)
                .into_iter()
                .find(|entry| entry["slug"] == "sourcegraph")
                .unwrap()["title"],
            "Sourcegraph Next"
        );
        rollback_runtime_catalog(&db).unwrap();
        assert_eq!(
            crate::db::capabilities::list_marketplace(&db)
                .into_iter()
                .find(|entry| entry["slug"] == "sourcegraph")
                .unwrap()["title"],
            "Sourcegraph Cloud"
        );
    }

    #[test]
    fn cold_bootstrap_and_manifest_bounds_fail_closed() {
        let db = database();
        let cold = runtime_catalog_status(&db);
        assert_eq!(cold.active_revision, None);
        assert_eq!(cold.highest_accepted_revision, 0);
        assert!(cold.stale);

        let valid = verified(VALID_JWS, 40);
        let mut malformed = valid.manifest.clone();
        malformed.artifacts[0].kind = "binary".to_string();
        assert!(
            validate_manifest_contract(
                &malformed,
                &valid.key_id,
                "0.1.0",
                FIXED_NOW,
                40,
                &BTreeSet::new(),
            )
            .is_err()
        );

        let mut oversized = valid.manifest.clone();
        oversized.artifacts[0].byte_size = ARTIFACT_MAX_BYTES + 1;
        assert!(
            validate_manifest_contract(
                &oversized,
                &valid.key_id,
                "0.1.0",
                FIXED_NOW,
                40,
                &BTreeSet::new(),
            )
            .is_err()
        );
        assert_eq!(runtime_catalog_status(&db).active_revision, None);
    }

    #[test]
    fn a_rotated_trusted_key_verifies_and_an_unknown_key_does_not() {
        let valid = verified(VALID_JWS, 40);
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("runtime-catalog-fixture-2".to_string());
        header.typ = Some("JWT".to_string());
        let rotated_jws = encode(
            &header,
            &valid.manifest,
            &EncodingKey::from_rsa_pem(PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap();

        let mut rotated_jwks = jwks();
        rotated_jwks.keys[0].common.key_id = header.kid.clone();
        let rotated = verify_runtime_catalog(
            &rotated_jws,
            &rotated_jwks,
            "0.1.0",
            FIXED_NOW,
            40,
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(rotated.key_id, "runtime-catalog-fixture-2");
        assert!(
            verify_runtime_catalog(
                &rotated_jws,
                &jwks(),
                "0.1.0",
                FIXED_NOW,
                40,
                &BTreeSet::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn accepted_revocations_remain_trusted_across_future_revisions() {
        let db = database();
        let valid = verified(VALID_JWS, 40);
        activate_runtime_catalog(&db, &valid, &downloaded(&valid), None).unwrap();

        let mut rotated_manifest = valid.manifest.clone();
        rotated_manifest.revision = 42;
        rotated_manifest.revocations = vec!["kid:runtime-catalog-fixture-1".to_string()];
        let mut rotated_header = Header::new(Algorithm::RS256);
        rotated_header.kid = Some("runtime-catalog-fixture-2".to_string());
        let rotated_jws = encode(
            &rotated_header,
            &rotated_manifest,
            &EncodingKey::from_rsa_pem(PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap();
        let mut rotated_jwks = jwks();
        let mut rotated_key = rotated_jwks.keys[0].clone();
        rotated_key.common.key_id = rotated_header.kid.clone();
        rotated_jwks.keys.push(rotated_key);
        let rotated = verify_runtime_catalog(
            &rotated_jws,
            &rotated_jwks,
            "0.1.0",
            FIXED_NOW,
            41,
            &trusted_runtime_catalog_revocations(&db).unwrap(),
        )
        .unwrap();
        activate_runtime_catalog(&db, &rotated, &downloaded(&rotated), None).unwrap();

        let trusted = trusted_runtime_catalog_revocations(&db).unwrap();
        assert!(trusted.contains("kid:runtime-catalog-fixture-1"));
        let mut malicious_manifest = valid.manifest.clone();
        malicious_manifest.revision = 44;
        let mut old_header = Header::new(Algorithm::RS256);
        old_header.kid = Some("runtime-catalog-fixture-1".to_string());
        let malicious_jws = encode(
            &old_header,
            &malicious_manifest,
            &EncodingKey::from_rsa_pem(PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap();
        assert!(
            verify_runtime_catalog(
                &malicious_jws,
                &rotated_jwks,
                "0.1.0",
                FIXED_NOW,
                42,
                &trusted,
            )
            .is_err()
        );
        assert_eq!(runtime_catalog_status(&db).active_revision, Some(42));
    }

    #[test]
    fn concurrent_activation_has_one_monotonic_winner() {
        let db = std::sync::Arc::new(database());
        let valid = std::sync::Arc::new(verified(VALID_JWS, 40));
        let artifacts = std::sync::Arc::new(downloaded(&valid));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let handles = (0..2)
            .map(|_| {
                let db = std::sync::Arc::clone(&db);
                let valid = std::sync::Arc::clone(&valid);
                let artifacts = std::sync::Arc::clone(&artifacts);
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    activate_runtime_catalog(&db, &valid, &artifacts, None)
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        assert_eq!(runtime_catalog_status(&db).active_revision, Some(41));
        let snapshots: u64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM runtime_catalog_snapshots",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(snapshots, 1);
    }

    #[tokio::test]
    async fn runtime_catalog_client_enforces_timeout_and_redirect_policy() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let _connection = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(100));
        });
        let error = runtime_catalog_client(Duration::from_millis(20))
            .unwrap()
            .get(format!("http://{address}/slow"))
            .send()
            .await
            .unwrap_err();
        assert!(error.is_timeout());
        server.join().unwrap();

        let redirect_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let redirect_address = redirect_listener.local_addr().unwrap();
        let redirect_server = std::thread::spawn(move || {
            let (mut connection, _) = redirect_listener.accept().unwrap();
            let mut request = [0u8; 1024];
            std::io::Read::read(&mut connection, &mut request).unwrap();
            std::io::Write::write_all(
                &mut connection,
                b"HTTP/1.1 302 Found\r\nLocation: https://example.com/\r\nContent-Length: 0\r\n\r\n",
            )
            .unwrap();
        });
        let response = runtime_catalog_client(Duration::from_secs(1))
            .unwrap()
            .get(format!("http://{redirect_address}/redirect"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        redirect_server.join().unwrap();
    }

    #[test]
    fn named_fixture_cold_and_warm_sync_metrics() {
        fn percentile(samples: &mut [u128], percentile: usize) -> u128 {
            samples.sort_unstable();
            samples[((samples.len() - 1) * percentile) / 100]
        }

        let mut cold_micros = Vec::new();
        for _ in 0..31 {
            let started = std::time::Instant::now();
            let db = database();
            let valid = verified(VALID_JWS, 40);
            activate_runtime_catalog(&db, &valid, &downloaded(&valid), Some("\"beta-41\""))
                .unwrap();
            cold_micros.push(started.elapsed().as_micros());
        }

        let manifest = verified(VALID_JWS, 40).manifest;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("runtime-catalog-fixture-1".to_string());
        let signing_key = EncodingKey::from_rsa_pem(PRIVATE_KEY.as_bytes()).unwrap();
        let key_set = jwks();
        let mut publish_to_active_micros = Vec::new();
        for _ in 0..31 {
            let started = std::time::Instant::now();
            let jws = encode(&header, &manifest, &signing_key).unwrap();
            let db = database();
            let verified =
                verify_runtime_catalog(&jws, &key_set, "0.1.0", FIXED_NOW, 40, &BTreeSet::new())
                    .unwrap();
            activate_runtime_catalog(&db, &verified, &downloaded(&verified), None).unwrap();
            publish_to_active_micros.push(started.elapsed().as_micros());
        }

        let db = database();
        let valid = verified(VALID_JWS, 40);
        activate_runtime_catalog(&db, &valid, &downloaded(&valid), Some("\"beta-41\"")).unwrap();
        let mut warm_micros = Vec::new();
        for _ in 0..101 {
            let started = std::time::Instant::now();
            assert_eq!(
                record_not_modified(&db, Some("\"beta-41\"")).active_revision,
                Some(41)
            );
            warm_micros.push(started.elapsed().as_micros());
        }

        let cold_p50 = percentile(&mut cold_micros, 50);
        let cold_p95 = percentile(&mut cold_micros, 95);
        let publish_to_active_p50 = percentile(&mut publish_to_active_micros, 50);
        let publish_to_active_p95 = percentile(&mut publish_to_active_micros, 95);
        let warm_p50 = percentile(&mut warm_micros, 50);
        let warm_p95 = percentile(&mut warm_micros, 95);
        let transferred_bytes = VALID_JWS.trim().len() + ARTIFACT.len();
        println!(
            "runtime_catalog_metrics payload=shared-fixture-v1 cold_samples=31 \
             publish_to_active_samples=31 warm_samples=101 \
             cold_p50_us={cold_p50} cold_p95_us={cold_p95} \
             publish_to_active_p50_us={publish_to_active_p50} \
             publish_to_active_p95_us={publish_to_active_p95} \
             warm_304_p50_us={warm_p50} warm_304_p95_us={warm_p95} \
             cold_body_bytes={transferred_bytes} warm_body_bytes=0 warm_cache_hits=101"
        );
        assert!(cold_p50 > 0 && cold_p95 >= cold_p50);
        assert!(publish_to_active_p50 > 0 && publish_to_active_p95 >= publish_to_active_p50);
        assert!(warm_p50 > 0 && warm_p95 >= warm_p50);
        assert_eq!(transferred_bytes, VALID_JWS.trim().len() + 427);
    }

    #[test]
    fn activation_preserves_pins_and_provider_projection_failures_are_isolated() {
        let root = std::env::temp_dir().join(format!(
            "flamel-runtime-catalog-projection-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let db = Db::open(root.join("flamel.db").to_string_lossy().as_ref());
        let valid = verified(VALID_JWS, 40);
        activate_runtime_catalog(&db, &valid, &downloaded(&valid), Some("\"beta-41\"")).unwrap();

        for project in [None, Some("/repo")] {
            skill_catalog::install_skill(
                &db,
                "skill-runtime-catalog-fixture",
                "user-1",
                project,
                Some("1.0.0"),
                true,
            )
            .unwrap();
        }
        let home = root.join("home");
        let codex = skill_catalog::project_provider_skill_home(
            &db,
            "user-1",
            Some("/repo"),
            "codex",
            &home,
        )
        .unwrap();
        let claude = skill_catalog::project_provider_skill_home(
            &db,
            "user-1",
            Some("/repo"),
            "claude",
            &home,
        )
        .unwrap();
        let relative = PathBuf::from("flamel-runtime-catalog-fixture").join("SKILL.md");
        let codex_skill = fs::read_to_string(PathBuf::from(codex.root).join(&relative)).unwrap();
        let claude_skill = fs::read_to_string(PathBuf::from(claude.root).join(&relative)).unwrap();
        assert_eq!(codex_skill, claude_skill);
        assert!(codex_skill.contains("flamel-version: \"1.0.0\""));

        let blocked_home = root.join("blocked-home");
        fs::write(&blocked_home, b"not a directory").unwrap();
        assert!(
            skill_catalog::project_provider_skill_home(
                &db,
                "user-1",
                Some("/repo"),
                "codex",
                &blocked_home,
            )
            .is_err()
        );
        assert_eq!(runtime_catalog_status(&db).active_revision, Some(41));

        let rollback = verified(ROLLBACK_JWS, 41);
        activate_runtime_catalog(
            &db,
            &rollback,
            &downloaded(&rollback),
            Some("\"stable-43\""),
        )
        .unwrap();
        let preserved: u64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM capability_skill_installations
                 WHERE skill_id='skill-runtime-catalog-fixture'
                   AND active_version='1.0.0' AND enabled=1
                   AND provider_projection_enabled=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preserved, 2);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn persona_snapshot_is_local_restart_safe_and_identity_checked() {
        fn snapshot(
            revision: u64,
            descriptor_slug: &str,
            content_slug: &str,
        ) -> (VerifiedRuntimeCatalog, Vec<DownloadedRuntimeArtifact>) {
            let content = serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "kind": "persona",
                "slug": content_slug,
                "version": "2.0.0",
                "description": "Backend architecture specialist",
                "content": "---\nname: backend-architect\n---\n\n# Backend architect\n\nUse local catalog instructions.",
            }))
            .unwrap();
            let descriptor = RuntimeCatalogArtifactDescriptor {
                kind: "persona".to_string(),
                slug: descriptor_slug.to_string(),
                version: "2.0.0".to_string(),
                sha256: format!("{:x}", Sha256::digest(&content)),
                byte_size: content.len(),
                fetch_url: format!("https://flamel.dev/runtime-catalog/persona/{descriptor_slug}"),
            };
            (
                VerifiedRuntimeCatalog {
                    manifest: RuntimeCatalogManifest {
                        schema_version: 1,
                        channel: "stable".to_string(),
                        revision,
                        min_runtime: "0.1.0".to_string(),
                        max_runtime: "0.1.0".to_string(),
                        artifacts: vec![descriptor.clone()],
                        revocations: Vec::new(),
                        iss: RUNTIME_CATALOG_ISSUER.to_string(),
                        aud: RUNTIME_CATALOG_AUDIENCE.to_string(),
                        iat: FIXED_NOW,
                        exp: FIXED_NOW + 60,
                    },
                    jws: format!("synthetic-{revision}"),
                    key_id: "test-key".to_string(),
                },
                vec![DownloadedRuntimeArtifact {
                    descriptor,
                    content,
                }],
            )
        }

        let root =
            std::env::temp_dir().join(format!("flamel-runtime-persona-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("flamel.db");
        let (valid, artifacts) = snapshot(1, "backend-architect", "backend-architect");
        {
            let db = Db::open(path.to_string_lossy().as_ref());
            activate_runtime_catalog(&db, &valid, &artifacts, None).unwrap();
            let personas = active_runtime_personas(&db).unwrap();
            assert_eq!(personas.len(), 1);
            assert_eq!(personas[0].slug, "backend-architect");
            assert!(
                personas[0]
                    .content
                    .contains("Use local catalog instructions")
            );
        }

        let db = Db::open(path.to_string_lossy().as_ref());
        assert_eq!(active_runtime_personas(&db).unwrap().len(), 1);
        let (invalid, artifacts) = snapshot(2, "wrong-name", "backend-architect");
        assert!(activate_runtime_catalog(&db, &invalid, &artifacts, None).is_err());
        assert_eq!(runtime_catalog_status(&db).active_revision, Some(1));
        assert_eq!(
            active_runtime_personas(&db).unwrap()[0].slug,
            "backend-architect"
        );
        drop(db);
        fs::remove_dir_all(root).ok();
    }
}
