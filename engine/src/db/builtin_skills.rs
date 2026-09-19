#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltinUpgradePolicy {
    /// Advance the catalog default to the manifest's current version. Explicit
    /// personal or project installations remain pinned because their
    /// active_version and enabled state live in capability_skill_installations.
    AdvanceCatalogDefault,
}

#[derive(Clone, Copy, Debug)]
pub struct BuiltinSkillVersion {
    pub version: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub instructions: &'static str,
    pub triggers: &'static [&'static str],
    pub required_capabilities: &'static [&'static str],
    pub allowed_operations: &'static [&'static str],
    pub provider_compatibility: &'static [&'static str],
    pub scope: &'static str,
    pub provenance_json: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct BuiltinSkillManifest {
    pub slug: &'static str,
    pub current_version: &'static str,
    /// Catalog fallback when no personal or project installation exists.
    /// Explicit installations preserve their own enabled state.
    pub default_enabled: bool,
    pub upgrade_policy: BuiltinUpgradePolicy,
    pub versions: &'static [BuiltinSkillVersion],
}

const BOTH_PROVIDERS: &[&str] = &["codex", "claude"];
const NO_CAPABILITIES: &[&str] = &[];
const NO_OPERATIONS: &[&str] = &[];
const BOOTSTRAP_PROVENANCE: &str = r#"{"kind":"flamel_bootstrap","source":"native_safe_fallback"}"#;

/// The only native skill body is a safe offline bootstrap. The signed Runtime
/// Catalog supplies the reviewed global catalog without a native rebuild.
pub const BUILTIN_SKILLS: &[BuiltinSkillManifest] = &[BuiltinSkillManifest {
    slug: "debugging",
    current_version: "1.0.0",
    default_enabled: true,
    upgrade_policy: BuiltinUpgradePolicy::AdvanceCatalogDefault,
    versions: &[BuiltinSkillVersion {
        version: "1.0.0",
        title: "Debugging",
        description: "Trace failures from reproduction to verified root cause",
        category: "engineering",
        instructions: "Reproduce the failure, gather direct evidence, isolate the smallest causal boundary, implement a focused fix, and rerun the exact failing path plus relevant regressions.",
        triggers: &["bug", "crash", "error", "failure", "debug"],
        required_capabilities: NO_CAPABILITIES,
        allowed_operations: NO_OPERATIONS,
        provider_compatibility: BOTH_PROVIDERS,
        scope: "global",
        provenance_json: BOOTSTRAP_PROVENANCE,
    }],
}];
