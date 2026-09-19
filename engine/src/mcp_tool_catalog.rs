use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRisk {
    ReadOnly,
    Low,
    Medium,
    High,
}

impl ToolRisk {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRenderer {
    Generic,
    Console,
    Plan,
    Agent,
    File,
    Code,
    Assets,
    Question,
    Diagnostics,
}

impl ToolRenderer {
    fn as_str(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Console => "console",
            Self::Plan => "plan",
            Self::Agent => "agent",
            Self::File => "file",
            Self::Code => "code",
            Self::Assets => "assets",
            Self::Question => "question",
            Self::Diagnostics => "diagnostics",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolCapability {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub display_name: &'static str,
    pub category: &'static str,
    pub risk: ToolRisk,
    pub read_only: bool,
    pub destructive: bool,
    pub mutates_files: bool,
    pub requires_prior_file_read: bool,
    pub streams: bool,
    pub renderer: ToolRenderer,
    pub provider_routing: &'static [&'static str],
    pub validation: &'static [&'static str],
}

pub const FLAMEL_TOOL_CAPABILITIES: &[ToolCapability] = &[
    ToolCapability {
        name: "ponytail_instructions",
        aliases: &["mcp__flamel__ponytail_instructions"],
        display_name: "Ponytail instructions",
        category: "prompt",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &["mode is lite, full, or ultra"],
    },
    ToolCapability {
        name: "Agent",
        aliases: &["mcp__flamel__Agent"],
        display_name: "Sub-agent",
        category: "agent",
        risk: ToolRisk::High,
        read_only: false,
        destructive: true,
        mutates_files: true,
        requires_prior_file_read: false,
        streams: true,
        renderer: ToolRenderer::Agent,
        provider_routing: &["flamel"],
        validation: &[
            "requires self-contained prompt",
            "requires explicit specialised persona",
        ],
    },
    ToolCapability {
        name: "AgentBatch",
        aliases: &["mcp__flamel__AgentBatch"],
        display_name: "Agent batch",
        category: "agent",
        risk: ToolRisk::High,
        read_only: false,
        destructive: true,
        mutates_files: true,
        requires_prior_file_read: false,
        streams: true,
        renderer: ToolRenderer::Agent,
        provider_routing: &["flamel"],
        validation: &[
            "requires at least one agent",
            "requires review prompt for cross-checking",
        ],
    },
    ToolCapability {
        name: "AgentBatchStatus",
        aliases: &["mcp__flamel__AgentBatchStatus"],
        display_name: "Agent batch status",
        category: "agent",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: true,
        renderer: ToolRenderer::Agent,
        provider_routing: &["flamel"],
        validation: &["optional batch id"],
    },
    ToolCapability {
        name: "plan",
        aliases: &[],
        display_name: "Plan",
        category: "workflow",
        risk: ToolRisk::Medium,
        read_only: false,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Plan,
        provider_routing: &["flamel"],
        validation: &["action is required", "mutating actions require plan id"],
    },
    ToolCapability {
        name: "assistant",
        aliases: &[],
        display_name: "Assistant administration",
        category: "assistant",
        risk: ToolRisk::High,
        read_only: false,
        destructive: true,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &[
            "action is required",
            "mutations preview unless explicitly confirmed",
            "confirmed mutations require an idempotency key",
        ],
    },
    ToolCapability {
        name: "job",
        aliases: &[],
        display_name: "Job control",
        category: "workflow",
        risk: ToolRisk::High,
        read_only: false,
        destructive: true,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &[
            "action is required",
            "repository and target access are enforced by the control plane",
            "mutations require the action-specific approval and idempotency contract",
        ],
    },
    ToolCapability {
        name: "file_history",
        aliases: &["revert_file", "list_file_edits", "checkpoint", "rollback"],
        display_name: "File history",
        category: "filesystem",
        risk: ToolRisk::Medium,
        read_only: false,
        destructive: true,
        mutates_files: true,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::File,
        provider_routing: &["flamel"],
        validation: &["destructive history operations require explicit action"],
    },
    ToolCapability {
        name: "flamel_context_stats",
        aliases: &[],
        display_name: "Context stats",
        category: "diagnostics",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Diagnostics,
        provider_routing: &["flamel"],
        validation: &[],
    },
    ToolCapability {
        name: "diagnostics",
        aliases: &["runtime_logs"],
        display_name: "Diagnostics",
        category: "diagnostics",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Diagnostics,
        provider_routing: &["flamel"],
        validation: &["requires admin user for runtime logs"],
    },
    ToolCapability {
        name: "capability_search",
        aliases: &[],
        display_name: "Capability search",
        category: "integrations",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &["returns only administrator-approved capabilities"],
    },
    ToolCapability {
        name: "capability_execute",
        aliases: &[],
        display_name: "Capability execute",
        category: "integrations",
        risk: ToolRisk::Medium,
        read_only: false,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &[
            "requires an approved connected capability",
            "enforces scoped tool policy",
            "records invocation and audit evidence",
        ],
    },
    ToolCapability {
        name: "project_intelligence",
        aliases: &[],
        display_name: "Project intelligence",
        category: "code",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &[
            "index respects project ignore rules",
            "context includes source freshness and relevance evidence",
        ],
    },
    ToolCapability {
        name: "skill_search",
        aliases: &[],
        display_name: "Skill search",
        category: "knowledge",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &["returns only enabled administrator-curated skills"],
    },
    ToolCapability {
        name: "skill_catalog",
        aliases: &[],
        display_name: "Skill catalog",
        category: "knowledge",
        risk: ToolRisk::Medium,
        read_only: false,
        destructive: true,
        mutates_files: true,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &[
            "projects only approved signed instruction-only skill versions",
            "requires administrator review for approval and catalog updates",
            "preserves provider-native auth, cache, history, plugins, and skills",
        ],
    },
    ToolCapability {
        name: "work",
        aliases: &[],
        display_name: "Engineering work",
        category: "workflow",
        risk: ToolRisk::Low,
        read_only: false,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &[
            "all records are user and project scoped",
            "plan and orchestration links preserve their original models",
        ],
    },
    ToolCapability {
        name: "code_nav",
        aliases: &[
            "flamel_search",
            "get_symbols_overview",
            "find_symbol",
            "find_referencing_symbols",
            "get_imports",
            "gather_context",
            "dependency_graph",
        ],
        display_name: "Code navigation",
        category: "code",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Code,
        provider_routing: &["flamel"],
        validation: &["uses structured code search/navigation actions"],
    },
    ToolCapability {
        name: "shell",
        aliases: &[
            "flamel_shell",
            "shell_run",
            "shell_start",
            "shell_read",
            "shell_write",
            "shell_kill",
            "shell_status",
            "multi_shell",
            "bash",
            "exec_command",
            "command_execution",
            "terminal",
        ],
        display_name: "Shell",
        category: "command",
        risk: ToolRisk::High,
        read_only: false,
        destructive: true,
        mutates_files: true,
        requires_prior_file_read: false,
        streams: true,
        renderer: ToolRenderer::Console,
        provider_routing: &["flamel"],
        validation: &[
            "git commands route through git MCP",
            "file writes route through file_edit",
        ],
    },
    ToolCapability {
        name: "file_edit",
        aliases: &[
            "flamel_replace",
            "replace_content",
            "multi_replace",
            "replace_symbol_body",
            "insert_before_symbol",
            "insert_after_symbol",
            "create_text_file",
            "diff_apply",
            "format_file",
            "preview_edit",
        ],
        display_name: "File edit",
        category: "filesystem",
        risk: ToolRisk::High,
        read_only: false,
        destructive: true,
        mutates_files: true,
        requires_prior_file_read: true,
        streams: false,
        renderer: ToolRenderer::File,
        provider_routing: &["flamel"],
        validation: &[
            "existing files must be read before edit",
            "multi_replace requires every target file to be read",
        ],
    },
    ToolCapability {
        name: "decision_checkpoint",
        aliases: &[],
        display_name: "Decision checkpoint",
        category: "workflow",
        risk: ToolRisk::Low,
        read_only: false,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &["requires constraints, evidence, decision, risks, and next action"],
    },
    ToolCapability {
        name: "AskUserQuestion",
        aliases: &["mcp__flamel__AskUserQuestion"],
        display_name: "Ask user",
        category: "workflow",
        risk: ToolRisk::Low,
        read_only: false,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Question,
        provider_routing: &["flamel"],
        validation: &[
            "requires one concise consequential question",
            "when choices are present, requires 2-3 mutually exclusive options with the recommendation first",
        ],
    },
    ToolCapability {
        name: "file_read",
        aliases: &[
            "flamel_read",
            "read_file",
            "batch_read",
            "file_summary",
            "flamel_glob",
            "find_file",
            "list_dir",
        ],
        display_name: "File read",
        category: "filesystem",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::File,
        provider_routing: &["flamel"],
        validation: &["read action defaults to file read"],
    },
    ToolCapability {
        name: "code_quality",
        aliases: &[
            "build_errors",
            "run_tests",
            "explain_error",
            "find_code_issues",
        ],
        display_name: "Code quality",
        category: "code",
        risk: ToolRisk::Medium,
        read_only: false,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: true,
        renderer: ToolRenderer::Console,
        provider_routing: &["flamel"],
        validation: &["build and test commands stream through managed checks"],
    },
    ToolCapability {
        name: "get_recent_changes",
        aliases: &[],
        display_name: "Recent changes",
        category: "git",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Code,
        provider_routing: &["flamel"],
        validation: &[],
    },
    ToolCapability {
        name: "assets",
        aliases: &["media", "fetch_image", "fetch_model3d", "create_model3d"],
        display_name: "Assets",
        category: "assets",
        risk: ToolRisk::Medium,
        read_only: false,
        destructive: false,
        mutates_files: true,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Assets,
        provider_routing: &["flamel"],
        validation: &["model generation requires OpenSCAD source"],
    },
    ToolCapability {
        name: "git",
        aliases: &[],
        display_name: "Git",
        category: "git",
        risk: ToolRisk::High,
        read_only: false,
        destructive: true,
        mutates_files: true,
        requires_prior_file_read: false,
        streams: true,
        renderer: ToolRenderer::Code,
        provider_routing: &["flamel"],
        validation: &[
            "Flamel Git is the default provider",
            "GitHub and Gitea require explicit connected-provider actions",
            "raw provider API escape hatches are not exposed",
        ],
    },
    ToolCapability {
        name: "db",
        aliases: &[],
        display_name: "Database",
        category: "database",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Code,
        provider_routing: &["flamel"],
        validation: &["read-only exploration", "scoped database connection"],
    },
    ToolCapability {
        name: "browser_session",
        aliases: &["browser"],
        display_name: "Browser session",
        category: "browser",
        risk: ToolRisk::Medium,
        read_only: false,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: true,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &[
            "browser isolation is enforced by the gateway",
            "saved profiles require explicit user instruction",
        ],
    },
    ToolCapability {
        name: "browser_run",
        aliases: &[],
        display_name: "Browser actions",
        category: "browser",
        risk: ToolRisk::Medium,
        read_only: false,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: true,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &[
            "at most 25 actions per batch",
            "element targets use accessibility snapshot identifiers",
        ],
    },
    ToolCapability {
        name: "browser_inspect",
        aliases: &[],
        display_name: "Browser inspection",
        category: "browser",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: true,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &["inspection output is bounded by the gateway"],
    },
    ToolCapability {
        name: "memory",
        aliases: &[],
        display_name: "Memory",
        category: "memory",
        risk: ToolRisk::Medium,
        read_only: false,
        destructive: true,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &["user and project scope are required"],
    },
    ToolCapability {
        name: "resolve_library",
        aliases: &[],
        display_name: "Resolve documentation library",
        category: "documentation",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &["returns version-aware library identifiers"],
    },
    ToolCapability {
        name: "get_docs",
        aliases: &[],
        display_name: "Documentation",
        category: "documentation",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &["uses version-aware documentation sources"],
    },
    ToolCapability {
        name: "search_flamel_docs",
        aliases: &[],
        display_name: "Search Flamel documentation",
        category: "documentation",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &["returns indexed Flamel documentation matches"],
    },
    ToolCapability {
        name: "get_flamel_doc",
        aliases: &[],
        display_name: "Flamel documentation page",
        category: "documentation",
        risk: ToolRisk::ReadOnly,
        read_only: true,
        destructive: false,
        mutates_files: false,
        requires_prior_file_read: false,
        streams: false,
        renderer: ToolRenderer::Generic,
        provider_routing: &["flamel"],
        validation: &["requires an indexed documentation identifier"],
    },
];

pub fn capability_for_tool_name(tool_name: &str) -> Option<&'static ToolCapability> {
    let normalized = normalize_tool_name(tool_name);
    FLAMEL_TOOL_CAPABILITIES.iter().find(|capability| {
        names_match(capability.name, normalized)
            || capability
                .aliases
                .iter()
                .any(|alias| names_match(alias, normalized))
    })
}

pub fn capability_meta_value(capability: &ToolCapability) -> Value {
    json!({
        "flamel": {
            "displayName": capability.display_name,
            "category": capability.category,
            "risk": capability.risk.as_str(),
            "renderer": capability.renderer.as_str(),
            "readOnly": capability.read_only,
            "destructive": capability.destructive,
            "mutatesFiles": capability.mutates_files,
            "requiresPriorFileRead": capability.requires_prior_file_read,
            "streams": capability.streams,
            "providerRouting": capability.provider_routing,
            "validation": capability.validation,
        }
    })
}

pub fn capability_annotations_value(capability: &ToolCapability) -> Value {
    json!({
        "title": capability.display_name,
        "readOnlyHint": capability.read_only,
        "destructiveHint": capability.destructive,
        "idempotentHint": capability.read_only && !capability.destructive,
        "openWorldHint": !capability.read_only || capability.streams || capability.renderer == ToolRenderer::Assets,
    })
}

pub fn apply_tool_capability_metadata(tools: &mut Value) {
    let Some(tools) = tools.as_array_mut() else {
        return;
    };

    for tool in tools {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(capability) = capability_for_tool_name(name) else {
            continue;
        };
        if let Some(tool_object) = tool.as_object_mut() {
            tool_object
                .entry("annotations".to_string())
                .or_insert_with(|| capability_annotations_value(capability));
            tool_object.insert("_meta".to_string(), capability_meta_value(capability));
        }
    }
}

fn normalize_tool_name(tool_name: &str) -> &str {
    tool_name
        .strip_prefix("mcp__flamel__")
        .or_else(|| tool_name.strip_prefix("flamel__"))
        .unwrap_or(tool_name)
}

fn names_match(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right) || normalize_tool_name(left).eq_ignore_ascii_case(right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_matches_prefixed_and_legacy_tool_names() {
        assert_eq!(
            capability_for_tool_name("mcp__flamel__file_edit").map(|capability| capability.name),
            Some("file_edit")
        );
        assert_eq!(
            capability_for_tool_name("flamel_replace").map(|capability| capability.name),
            Some("file_edit")
        );
        assert_eq!(
            capability_for_tool_name("mcp__flamel__AgentBatch").map(|capability| capability.name),
            Some("AgentBatch")
        );
        assert_eq!(
            capability_for_tool_name("browser").map(|capability| capability.name),
            Some("browser_session")
        );
        assert_eq!(
            capability_for_tool_name("browser_inspect").map(|capability| capability.name),
            Some("browser_inspect")
        );
    }

    #[test]
    fn catalog_metadata_marks_read_and_write_tools() {
        let file_read = capability_for_tool_name("file_read").expect("file_read capability");
        assert!(file_read.read_only);
        assert!(!file_read.destructive);
        assert!(!file_read.mutates_files);

        let file_edit = capability_for_tool_name("file_edit").expect("file_edit capability");
        assert!(!file_edit.read_only);
        assert!(file_edit.destructive);
        assert!(file_edit.mutates_files);
        assert!(file_edit.requires_prior_file_read);
        assert_eq!(file_edit.risk, ToolRisk::High);
    }

    #[test]
    fn apply_tool_capability_metadata_enriches_tool_list_entries() {
        let mut tools = json!([
            { "name": "shell", "description": "Run shell commands", "inputSchema": {"type": "object"} },
            { "name": "unknown_tool", "description": "Unknown", "inputSchema": {"type": "object"} }
        ]);

        apply_tool_capability_metadata(&mut tools);

        let shell = &tools[0];
        assert_eq!(shell["_meta"]["flamel"]["renderer"], "console");
        assert_eq!(shell["_meta"]["flamel"]["streams"], true);
        assert_eq!(shell["annotations"]["readOnlyHint"], false);
        assert_eq!(shell["annotations"]["destructiveHint"], true);
        assert!(tools[1].get("_meta").is_none());
    }
}
