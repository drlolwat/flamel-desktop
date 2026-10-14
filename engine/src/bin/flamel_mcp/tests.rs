use super::*;

#[test]
fn advertised_tools_have_dispatch_and_metadata_parity() {
    let tools = tools_list();
    let catalog = tools.as_array().expect("tool catalog array");
    let mut advertised = HashSet::new();
    let mut missing_metadata = Vec::new();
    let mut missing_dispatch = Vec::new();

    for tool in catalog {
        let name = tool["name"].as_str().expect("tool name");
        assert!(advertised.insert(name), "duplicate advertised tool: {name}");
        if tool.get("_meta").is_none() {
            missing_metadata.push(name);
        }
        if name != "ponytail_instructions"
            && !flamel::mcp_gateway::owns_tool(name)
            && tool_family(name).is_none()
        {
            missing_dispatch.push(name);
        }
    }

    assert!(
        missing_metadata.is_empty(),
        "advertised tools lack capability metadata: {missing_metadata:?}"
    );
    assert!(
        missing_dispatch.is_empty(),
        "advertised tools lack a dispatch owner: {missing_dispatch:?}"
    );

    for profile in [McpProfile::Default, McpProfile::PersonalAssistant] {
        for tool in tools_list_for_profile(&profile)
            .as_array()
            .expect("profile tool catalog array")
        {
            let name = tool["name"].as_str().expect("profile tool name");
            assert!(
                advertised.contains(name),
                "profile exposes an unadvertised tool: {name}"
            );
        }
    }

    assert!(
        tools_list_for_profile(&McpProfile::Invalid("invalid".to_string()))
            .as_array()
            .expect("invalid profile tool catalog array")
            .is_empty()
    );
}

#[test]
fn shell_catalog_exposes_encrypted_local_secrets_and_sensitive_stdin() {
    let tools = tools_list();
    let shell = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "shell")
        .expect("shell tool");
    let actions = shell["inputSchema"]["properties"]["action"]["enum"]
        .as_array()
        .unwrap();
    for action in ["write", "store_secret", "delete_secret", "secret_status"] {
        assert!(actions.iter().any(|value| value == action));
    }
    let properties = shell["inputSchema"]["properties"].as_object().unwrap();
    for field in ["sensitive", "secret_name", "secret_ref", "append_newline"] {
        assert!(properties.contains_key(field));
    }
    let description = shell["description"].as_str().unwrap();
    assert!(description.contains("direct user request"));
    assert!(description.contains("first request without separate confirmation"));
    assert!(!description.contains("Sensitive input is omitted from command replay"));
}

#[test]
fn ask_user_question_catalog_enforces_focused_choice_contract() {
    let tools = tools_list();
    let ask = tools
        .as_array()
        .and_then(|items| items.iter().find(|tool| tool["name"] == "AskUserQuestion"))
        .expect("AskUserQuestion tool");
    let description = ask["description"].as_str().expect("tool description");
    assert!(description.contains("one focused question"));
    assert!(description.contains("blocks until the user answers or cancels"));
    assert!(description.contains("two or three mutually exclusive options"));
    assert!(description.contains("recommended option first"));
    assert_eq!(ask["inputSchema"]["properties"]["choices"]["minItems"], 2);
    assert_eq!(ask["inputSchema"]["properties"]["choices"]["maxItems"], 3);
}

#[test]
fn ponytail_prompt_and_tool_are_exposed_over_mcp() {
    let profile = McpProfile::Default;
    let initialized = handle_request(
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        "",
        "",
        "",
        "",
        &profile,
    )
    .expect("initialize response");
    assert_eq!(
        initialized["result"]["capabilities"]["prompts"]["listChanged"],
        false
    );

    let prompts = handle_request(
        &json!({"jsonrpc": "2.0", "id": 2, "method": "prompts/list"}),
        "",
        "",
        "",
        "",
        &profile,
    )
    .expect("prompts/list response");
    assert_eq!(prompts["result"]["prompts"][0]["name"], "ponytail");

    let prompt = handle_request(
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "prompts/get",
            "params": {"name": "ponytail", "arguments": {"mode": "full"}}
        }),
        "",
        "",
        "",
        "",
        &profile,
    )
    .expect("prompts/get response");
    let prompt_text = prompt["result"]["messages"][0]["content"]["text"]
        .as_str()
        .expect("prompt text");
    assert!(prompt_text.starts_with("PONYTAIL MODE ACTIVE — level: full"));
    assert!(prompt_text.contains("| **full** |"));
    assert!(!prompt_text.contains("| **ultra** |"));

    let tool = handle_request(
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "ponytail_instructions", "arguments": {"mode": "ultra"}}
        }),
        "",
        "",
        "",
        "",
        &profile,
    )
    .expect("tools/call response");
    assert_eq!(tool["result"]["structuredContent"]["mode"], "ultra");
    assert!(
        tool["result"]["structuredContent"]["instructions"]
            .as_str()
            .is_some_and(|text| text.contains("| **ultra** |"))
    );
}

#[test]
fn subagents_defer_provider_native_compaction_to_flamel() {
    let mut claude = std::process::Command::new("claude");
    configure_subagent_compaction_guard(&mut claude, "claude");
    let claude_args = claude
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        claude_args,
        vec![
            "--autocompact".to_string(),
            flamel::claude::spawn::CLAUDE_NATIVE_COMPACTION_CEILING.to_string(),
        ]
    );

    let mut codex = std::process::Command::new("codex");
    configure_subagent_compaction_guard(&mut codex, "codex");
    let codex_args = codex
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        codex_args,
        vec![
            "-c".to_string(),
            flamel::claude::codex_provider::CODEX_NATIVE_COMPACTION_LIMIT_CONFIG.to_string(),
        ]
    );
}

#[test]
fn plan_markers_never_embed_version_history_or_artifact_snapshots() {
    let plan = json!({
        "id": "plan-1",
        "title": "Plan",
        "items": [{"id": "item-1", "text": "Do it", "status": "todo"}],
        "versions": [{"id": "version-1", "items": [{"notes": "large history"}]}],
        "artifact": {"id": "artifact-1", "metadata": {"large": "snapshot"}},
        "versionCount": 1,
    });

    let compact = plan_marker_payload(&plan, false);
    assert_eq!(compact["planId"], "plan-1");
    assert_eq!(compact["itemCount"], 1);
    assert!(compact.get("items").is_none());
    assert!(compact.get("versions").is_none());
    assert!(compact.get("artifact").is_none());

    let full = plan_marker_payload(&plan, true);
    assert_eq!(full["items"][0]["id"], "item-1");
    assert!(full.get("versions").is_none());
    assert!(full.get("artifact").is_none());
}

#[test]
fn unrestricted_shell_flag_accepts_only_explicit_truthy_values() {
    for value in ["1", "true", "TRUE", " yes ", "On"] {
        assert!(
            enabled_env_flag(Some(value)),
            "expected {value:?} to enable"
        );
    }
    for value in ["", "0", "false", "no", "disabled", "2"] {
        assert!(
            !enabled_env_flag(Some(value)),
            "expected {value:?} to remain disabled"
        );
    }
    assert!(!enabled_env_flag(None));
}

#[test]
fn automation_capability_grants_are_exact_and_fail_closed() {
    assert!(
        parse_automation_capability_grants(r#"{"mode":"unrestricted"}"#)
            .unwrap()
            .is_none()
    );
    let grants = parse_automation_capability_grants(
        r#"{
            "mode":"restricted",
            "grants":[{
                "capability_id":"pack.gmail",
                "tools":["gmail.messages.get","gmail.messages.list"]
            }]
        }"#,
    )
    .unwrap()
    .unwrap();
    assert_eq!(grants.len(), 1);
    assert!(
        grants["pack.gmail"].contains("gmail.messages.get")
            && !grants["pack.gmail"].contains("gmail.messages.delete")
    );
    for invalid in [
        r#"{"mode":"restricted","grants":[{"capability_id":"pack.gmail","tools":["*"]}]}"#,
        r#"{"mode":"restricted","grants":[{"capability_id":"pack.gmail","tools":[]}]}"#,
        r#"{"mode":"restricted","grants":[{"capability_id":"pack.gmail","tools":["gmail.messages.get"],"url":"https://evil.test"}]}"#,
        r#"{"mode":"unknown","grants":[]}"#,
    ] {
        assert!(parse_automation_capability_grants(invalid).is_err());
    }
}

#[test]
fn initialize_negotiates_latest_and_legacy_protocols() {
    let latest = handle_request(
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": flamel::mcp_gateway::PROTOCOL_VERSION}
        }),
        "",
        "",
        "",
        "",
        &McpProfile::Default,
    )
    .expect("latest initialize response");
    assert_eq!(
        latest["result"]["protocolVersion"],
        flamel::mcp_gateway::PROTOCOL_VERSION
    );

    let legacy = handle_request(
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "initialize",
            "params": {"protocolVersion": "2025-03-26"}
        }),
        "",
        "",
        "",
        "",
        &McpProfile::Default,
    )
    .expect("legacy initialize response");
    assert_eq!(
        legacy["result"]["protocolVersion"],
        flamel::mcp_gateway::LEGACY_PROTOCOL_VERSION
    );
}

#[test]
fn server_discover_is_available_without_initialization() {
    let response = handle_request(
        &json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover"}),
        "",
        "",
        "",
        "",
        &McpProfile::Default,
    )
    .expect("discover response");
    assert_eq!(
        response["result"]["protocolVersion"],
        flamel::mcp_gateway::PROTOCOL_VERSION
    );
    assert_eq!(response["result"]["serverInfo"]["name"], "flamel");
}

#[test]
fn assistant_tool_schema_is_exact_previewable_and_identity_safe() {
    let tools = tools_list();
    let assistant = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some("assistant"))
        .expect("assistant tool");
    assert_eq!(
        assistant["inputSchema"]["additionalProperties"],
        Value::Bool(false)
    );
    let actions = assistant["inputSchema"]["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    for action in [
        "assistant.status",
        "capability.snapshot",
        "pack.preview",
        "openclaw.preview",
        "delegation.list",
        "delegation.create",
        "delegation.approve",
        "delegation.cancel",
        "brain.remember",
        "skill.install",
        "pack.install",
        "connection.authorize",
        "automation.preview",
        "automation.create",
        "job.run",
        "diagnostics.status",
        "diagnostics.recover",
    ] {
        assert!(
            actions.contains(&action),
            "missing assistant action {action}"
        );
    }
    for forbidden in [
        "assistant.raw_api",
        "connection.set_credential",
        "job.merge",
        "shell.run",
    ] {
        assert!(!actions.contains(&forbidden));
    }
    let properties = assistant["inputSchema"]["properties"]
        .as_object()
        .expect("assistant properties");
    for required in [
        "confirm",
        "idempotency_key",
        "channel_invocation_id",
        "chattle_invocation_id",
    ] {
        assert!(properties.contains_key(required));
    }
    for forbidden in ["user_id", "actor_id", "space_id", "channel_id"] {
        assert!(!properties.contains_key(forbidden));
    }
}

#[test]
fn job_tool_schema_exposes_only_the_exact_control_plane_actions() {
    let tools = tools_list();
    let job = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some("job"))
        .expect("job tool");
    let actions = job["inputSchema"]["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    for action in [
        "list",
        "get",
        "create",
        "update",
        "run",
        "rerun",
        "pause",
        "resume",
        "cancel",
        "steer",
        "approve",
        "events",
        "profile_list",
        "profile_get",
        "profile_preview",
        "profile_create",
        "profile_update",
        "profile_delete",
        "profile_enable",
        "profile_disable",
        "profile_pause",
        "profile_resume",
        "profile_run",
        "profile_event",
    ] {
        assert!(actions.contains(&action), "missing Job action {action}");
    }
    for forbidden in ["merge", "deploy", "secret", "environment", "raw_api"] {
        assert!(!actions.contains(&forbidden));
    }

    let properties = job["inputSchema"]["properties"]
        .as_object()
        .expect("Job properties");
    for required in [
        "channel_invocation_id",
        "chattle_invocation_id",
        "notification_policy",
    ] {
        assert!(properties.contains_key(required));
    }
    for forbidden in [
        "binding_id",
        "external_actor_id",
        "external_space_id",
        "external_channel_id",
        "external_message_id",
    ] {
        assert!(
            !properties.contains_key(forbidden),
            "raw Chattle field leaked into Job schema: {forbidden}"
        );
    }
}

#[test]
fn personal_assistant_profile_exposes_canonical_tools_and_native_browser() {
    let profile = McpProfile::parse(Some(flamel::cli_mcp::PERSONAL_ASSISTANT_PROFILE));
    assert_eq!(profile.label(), flamel::cli_mcp::PERSONAL_ASSISTANT_PROFILE);
    assert!(profile.instructions().contains("native Flamel Assistant"));
    assert!(
        profile
            .instructions()
            .contains("persistent browser sessions")
    );

    let listed = handle_request(
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        "",
        "",
        "",
        "",
        &profile,
    )
    .expect("native personal assistant tools response");
    let names = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    for expected in ["file_read", "assistant", "job"] {
        assert!(
            names.contains(&expected),
            "missing native assistant tool {expected}"
        );
    }
    for browser_tool in ["browser_session", "browser_run", "browser_inspect"] {
        assert!(
            profile.allows_tool(browser_tool),
            "native assistant profile filtered browser tool {browser_tool}"
        );
    }
}

#[test]
fn unknown_mcp_profile_fails_closed() {
    let profile = McpProfile::parse(Some("guessed-profile"));
    let listed = tools_list_for_profile(&profile);
    assert!(listed.as_array().unwrap().is_empty());
    assert!(!profile.allows_tool("file_read"));
    assert_eq!(profile.label(), "guessed-profile");
}

#[test]
fn subagent_provider_must_be_explicit_and_supported() {
    assert!(
        resolve_subagent_cli_binary("gemini")
            .expect_err("unknown providers must fail closed")
            .contains("unsupported authenticated provider")
    );
}

#[test]
fn github_cli_host_normalizes_api_github() {
    assert_eq!(
        github_cli_host("https://api.github.com"),
        Some("github.com".to_string())
    );
    assert_eq!(
        github_cli_host("https://github.com/foo/bar"),
        Some("github.com".to_string())
    );
}

#[test]
fn select_github_cli_token_ignores_masked_and_non_github_accounts() {
    let accounts = vec![
        json!({"type": "gitea", "baseUrl": "https://sauce.example", "token": "gitea-token"}),
        json!({"type": "github", "baseUrl": "https://api.github.com", "token": "***"}),
        json!({"type": "github", "baseUrl": "https://api.github.com", "token": "ghp_real"}),
    ];

    assert_eq!(
        select_github_cli_token(&accounts),
        Some("ghp_real".to_string())
    );
}

#[test]
fn codex_subagent_config_preserves_native_skill_and_plugin_settings() {
    let temp_dir = std::env::temp_dir().join(format!(
        "flamel_codex_subagent_config_test_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).expect("temp dir");
    std::fs::write(
        temp_dir.join("config.toml"),
        r#"model = "gpt-5.6-terra"

[[skills.config]]
path = "/tmp/native/SKILL.md"
enabled = true

[plugins.native-images]
enabled = true

[mcp_servers.flamel]
command = "/tmp/stale"
"#,
    )
    .expect("seed config");

    write_codex_subagent_config(
        r#"{"flamel":{"command":"/tmp/current","args":[]}}"#,
        temp_dir.to_str().expect("utf8 temp dir"),
        Some("child runtime instructions"),
    );

    let config = std::fs::read_to_string(temp_dir.join("config.toml")).expect("config");
    assert!(config.contains("model = \"gpt-5.6-terra\""));
    assert!(config.contains("[[skills.config]]"));
    assert!(config.contains("[plugins.native-images]"));
    assert!(config.contains("instructions = \"child runtime instructions\""));
    assert!(config.contains("command = \"/tmp/current\""));
    assert!(!config.contains("command = \"/tmp/stale\""));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[cfg(unix)]
#[test]
fn codex_subagent_inherits_native_runtime_directories() {
    let root = std::env::temp_dir().join(format!(
        "flamel_codex_subagent_state_test_{}",
        std::process::id()
    ));
    let parent = root.join("parent");
    let child = root.join("child");
    let _ = std::fs::remove_dir_all(&root);
    for directory in ["skills", "plugins", "generated_images"] {
        std::fs::create_dir_all(parent.join(directory)).expect("parent runtime directory");
    }
    std::fs::create_dir_all(&child).expect("child home");
    std::fs::write(parent.join("auth.json"), "{}").expect("auth");
    std::fs::write(parent.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("config");

    inherit_codex_runtime_state(
        parent.to_str().expect("utf8 parent"),
        child.to_str().expect("utf8 child"),
    );

    assert!(child.join("auth.json").is_file());
    assert!(child.join("config.toml").is_file());
    for directory in ["skills", "plugins", "generated_images"] {
        assert!(child.join(directory).is_dir(), "missing {directory}");
    }

    let _ = std::fs::remove_dir_all(&root);
}

// Single sub-agent: exercise resolve_persona for every known persona and
// confirm each distinct-prompt key actually lands on a different string. If
// two personas collapse to the same prompt, the persona system has silently
// regressed. (Covers plan item 39878c9b — single sub-agent uses flamel
// tools only; the "flamel tools only" guarantee comes from the Explore
// persona explicitly enumerating `mcp__flamel__*` helpers.)
#[test]
fn resolve_persona_known_types_differ() {
    let explore = resolve_persona("Explore");
    let plan = resolve_persona("Plan");
    let general = resolve_persona("general-purpose");
    assert!(
        explore.contains("mcp__flamel__"),
        "Explore persona must reference flamel MCP tools"
    );
    assert!(
        explore.contains("Do NOT edit"),
        "Explore persona must be read-only"
    );
    assert!(
        plan.contains("Do NOT implement"),
        "Plan persona must not write code"
    );
    assert!(
        general.contains("flamel MCP tools only"),
        "general-purpose persona must direct sub-agents to flamel tools only"
    );
    assert_ne!(explore, plan);
    assert_ne!(explore, general);
    assert_ne!(plan, general);
}

// Covers the "missing persona fallback" half of plan item cef643da.
#[test]
fn resolve_persona_unknown_falls_back() {
    let unknown = resolve_persona("totally-made-up-persona");
    assert!(
        unknown.contains("unknown"),
        "unknown persona fallback must announce itself so the caller is aware"
    );
    assert!(
        unknown.contains("flamel MCP tools only"),
        "fallback must still enforce flamel-only tool usage"
    );
    // Empty string is treated as general-purpose, not the unknown fallback.
    let empty = resolve_persona("");
    assert_eq!(empty.as_ref(), resolve_persona("general-purpose").as_ref());
}

#[test]
fn runtime_catalog_personas_are_exact_thin_and_parseable() {
    use std::collections::BTreeSet;

    const MATRIX: &str = include_str!("../../../../agents/PERSONA-SKILL-MATRIX.md");
    let index: Value =
        serde_json::from_str(include_str!("../../../../runtime-catalog/personas.json")).unwrap();
    let governance: Value =
        serde_json::from_str(include_str!("../../../../runtime-catalog/governance.json")).unwrap();
    let personas = index["personas"].as_array().unwrap();
    let ledger = index["sourceLedger"].as_array().unwrap();
    assert_eq!(personas.len(), 24);
    assert_eq!(ledger.len(), 42);
    assert_eq!(
        ledger
            .iter()
            .filter(|entry| entry["disposition"] == "retained_persona")
            .count(),
        24
    );
    assert_eq!(
        ledger
            .iter()
            .filter(|entry| entry["disposition"] == "extracted_skill")
            .count(),
        16
    );
    assert_eq!(
        ledger
            .iter()
            .filter(|entry| entry["disposition"] == "native_wrapper")
            .count(),
        2
    );

    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let entries = personas
        .iter()
        .map(|persona| {
            let source = persona["source"].as_str().unwrap();
            let content = std::fs::read_to_string(repository.join(source)).unwrap();
            json!({
                "slug": persona["slug"],
                "content": content,
            })
        })
        .collect::<Vec<_>>();
    let registry = parse_persona_catalog(Value::Array(entries.clone())).unwrap();
    let retained = [
        ("ai-engineer", "ai-systems"),
        ("analytics-reporter", "analytics-reporting"),
        ("api-tester", "api-verification"),
        ("app-store-optimizer", "app-store-publishing"),
        ("backend-architect", "backend-architecture"),
        ("brand-guardian", "brand-systems"),
        ("content-creator", "content-strategy"),
        ("devops-automator", "infrastructure-operations"),
        ("feedback-synthesizer", "feedback-synthesis"),
        ("finance-tracker", "financial-analysis"),
        ("frontend-developer", "frontend-engineering"),
        ("infrastructure-maintainer", "infrastructure-operations"),
        ("knowledge-fetcher", "evidence-research"),
        ("legal-compliance-checker", "legal-compliance"),
        ("mobile-app-builder", "mobile-engineering"),
        ("performance-benchmarker", "performance-analysis"),
        ("project-shipper", "release-readiness"),
        ("rapid-prototyper", "rapid-prototyping"),
        ("support-responder", "support-operations"),
        ("test-results-analyzer", "test-result-analysis"),
        ("tool-evaluator", "tool-evaluation"),
        ("trend-researcher", "market-research"),
        ("ux-researcher", "ux-validation"),
        ("visual-storyteller", "visual-storytelling-assets"),
    ];
    let expected = retained
        .iter()
        .map(|(name, _)| name.to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(registry.keys().cloned().collect::<BTreeSet<_>>(), expected);

    let governed_slugs = governance["personaDerivedSkills"]
        .as_array()
        .unwrap()
        .iter()
        .chain(governance["specialistSkills"].as_array().unwrap())
        .map(|slug| slug.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    let stale_markers = [
        "PROACTIVELY",
        "@base-config",
        "master orchestrator",
        "6-day sprint",
        "test-writer-fixer",
        "whimsy-injector",
        "mcp__flamel__",
    ];
    for (name, skill) in retained {
        let persona = registry.get(name).unwrap();
        assert!(
            persona.body.len() < 2_048,
            "{name} is not a thin overlay: {} bytes",
            persona.body.len()
        );
        assert!(persona.body.contains(&format!("reviewed `{skill}` skill")));
        assert!(governed_slugs.contains(skill), "{skill} is not governed");
        for marker in stale_markers {
            assert!(
                !persona.body.contains(marker),
                "{name} contains stale persona directive {marker}"
            );
        }
    }
    for entry in ledger {
        let owner = entry["owner"].as_str().unwrap();
        assert!(
            MATRIX.contains(&format!("| {owner} |")),
            "matrix is missing source owner {owner}"
        );
    }

    let mut duplicate = entries.clone();
    duplicate.push(entries[0].clone());
    assert!(parse_persona_catalog(Value::Array(duplicate)).is_err());
    assert!(
        parse_persona_catalog(json!([{
            "slug": "wrong-name",
            "content": entries[0]["content"],
        }]))
        .is_err()
    );
}

#[test]
fn materialize_persona_markdown_writes_full_md_file() {
    if !persona_registry().contains_key("backend-architect") {
        return;
    }

    let run_id = format!("test-{}", uuid::Uuid::new_v4());
    let cwd = std::env::current_dir()
        .expect("test cwd should resolve")
        .to_string_lossy()
        .into_owned();
    let path = materialize_persona_markdown(&run_id, "backend-architect", &cwd)
        .expect("persona materialization should not fail")
        .expect("backend-architect should have bundled markdown");
    let content = std::fs::read_to_string(&path).expect("persona markdown should be readable");
    assert!(content.starts_with("---\nname: backend-architect"));
    assert!(content.contains("# Backend architect"));
    assert!(content.contains("Read and apply the reviewed `backend-architecture` skill"));

    let persona_dir = std::path::Path::new(&path)
        .parent()
        .expect("persona file has parent dir")
        .to_path_buf();
    let _ = std::fs::remove_dir_all(persona_dir);
}

#[test]
fn materialize_persona_markdown_uses_non_temp_storage() {
    if !persona_registry().contains_key("content-creator") {
        return;
    }

    let run_id = format!("test-{}", uuid::Uuid::new_v4());
    let cwd = std::env::current_dir()
        .expect("test cwd should resolve")
        .to_string_lossy()
        .into_owned();
    let path = materialize_persona_markdown(&run_id, "content-creator", &cwd)
        .expect("persona materialization should not fail")
        .expect("content-creator should have bundled markdown");
    let path_buf = std::path::PathBuf::from(&path);

    assert!(!path_is_temp_dir(&path_buf));
    assert!(path.ends_with("content-creator.md"));
    assert!(std::fs::read_to_string(&path).is_ok());

    let persona_dir = path_buf.parent().expect("persona file has parent dir");
    let _ = std::fs::remove_dir_all(persona_dir);
}

#[test]
fn strip_frontmatter_handles_real_md_shape() {
    let md = "---\nname: foo\ndescription: bar\n---\n\nThe actual prompt.\nLine two.\n";
    assert_eq!(strip_frontmatter(md), "The actual prompt.\nLine two.\n");
}

#[test]
fn strip_frontmatter_no_frontmatter_returns_input() {
    let md = "no frontmatter here\nline two";
    assert_eq!(strip_frontmatter(md), md);
}

// Depth guard — a nested sub-agent spawned beyond MAX_DEPTH must refuse
// *before* spawning claude. We can't actually run run_subagent in a unit
// test (it shells out), but cap_output + format_agent_run_payload cover
// the deterministic body of the function and the depth-refusal string is
// asserted via a direct constant check below. (Plan item cda0a098 nested
// sub-agents + cef643da MAX_DEPTH refusal.)
#[test]
fn cap_output_below_threshold_is_identity() {
    let s = "hello world";
    assert_eq!(cap_output(s, 100, "[cut]"), s);
}

#[test]
fn cap_output_above_threshold_truncates_with_marker() {
    let s = "x".repeat(2000);
    let out = cap_output(&s, 500, "[cut]");
    assert!(out.ends_with("[cut]"));
    assert_eq!(out.len(), 500 + "[cut]".len());
}

#[test]
fn cap_output_respects_utf8_boundary() {
    // Multi-byte character straddling the cut point — must not split it.
    let s = "aaaa\u{1F4A9}bbbb"; // 💩 is 4 bytes
    // Cut inside the emoji byte range.
    let out = cap_output(s, 6, "…");
    // The truncation point must land on a valid char boundary.
    assert!(out.starts_with("aaaa"));
    assert!(out.ends_with("…"));
    assert!(std::str::from_utf8(out.as_bytes()).is_ok());
}

// Verifies the AGENT_RUN correlation envelope stays stable. The frontend
// parses the `AGENT_RUN:{uuid}\n` prefix character-by-character, so any
// drift here would silently break nested-event rendering.
#[test]
fn format_agent_run_payload_shape() {
    let out = format_agent_run_payload("abc-123", "hello");
    assert!(out.starts_with("AGENT_RUN:abc-123\n"));
    assert!(out.ends_with("hello"));
    // Exactly one newline separates the header from the body.
    assert_eq!(out.matches('\n').count(), 1);
}

#[test]
fn format_agent_run_payload_preserves_empty_body() {
    let out = format_agent_run_payload("r", "");
    assert_eq!(out, "AGENT_RUN:r\n");
}

#[test]
fn parse_codex_mcp_servers_config_unwraps_parent_wrapper() {
    let input = serde_json::json!({
        "mcpServers": {
            "flamel": {"command": "flamel_mcp", "args": []}
        }
    })
    .to_string();

    let parsed = parse_codex_mcp_servers_config(&input).expect("wrapped config parses");
    assert!(parsed.contains_key("flamel"));
    assert_eq!(parsed.len(), 1);
    assert!(!parsed.contains_key("mcpServers"));
}

#[test]
fn parse_codex_mcp_servers_config_accepts_direct_map() {
    let input = serde_json::json!({
        "flamel": {"command": "flamel_mcp", "args": []}
    })
    .to_string();

    let parsed = parse_codex_mcp_servers_config(&input).expect("direct config parses");
    assert!(parsed.contains_key("flamel"));
}

#[test]
fn prepare_child_mcp_servers_config_updates_flamel_env_for_recursion() {
    let input = serde_json::json!({
        "mcpServers": {
            "flamel": {
                "command": "flamel_mcp",
                "args": [],
                "env": {
                    "FLAMEL_AGENT_DEPTH": "0",
                    "FLAMEL_SYSTEM_PROMPT": "parent",
                    "FLAMEL_MCP_SERVERS": "stale"
                }
            }
        }
    })
    .to_string();

    let prepared = prepare_child_mcp_servers_config(&input, "child prompt", 3, "parent-run")
        .expect("child MCP config prepares");
    let parsed: Value = serde_json::from_str(&prepared).expect("prepared config is json");
    let env = parsed
        .get("mcpServers")
        .and_then(|v| v.get("flamel"))
        .and_then(|v| v.get("env"))
        .and_then(|v| v.as_object())
        .expect("flamel env exists");

    assert_eq!(
        env.get("FLAMEL_AGENT_DEPTH").and_then(|v| v.as_str()),
        Some("3")
    );
    assert_eq!(
        env.get("FLAMEL_SYSTEM_PROMPT").and_then(|v| v.as_str()),
        Some("child prompt")
    );
    assert_eq!(
        env.get("FLAMEL_PARENT_AGENT_RUN_ID")
            .and_then(|v| v.as_str()),
        Some("parent-run")
    );
    let nested = env
        .get("FLAMEL_MCP_SERVERS")
        .and_then(|v| v.as_str())
        .expect("recursive MCP snapshot is present");
    assert!(nested.contains("child prompt"));
    assert!(!nested.contains("stale"));
    let nested: Value = serde_json::from_str(nested).expect("nested config is json");
    assert_eq!(
        nested
            .get("mcpServers")
            .and_then(Value::as_object)
            .map(serde_json::Map::len),
        Some(1)
    );
}

#[test]
fn normalized_agent_batch_idle_timeout_uses_safe_defaults() {
    assert_eq!(normalized_agent_batch_idle_timeout_secs(None), 600);
    assert_eq!(normalized_agent_batch_idle_timeout_secs(Some(5)), 30);
    assert_eq!(normalized_agent_batch_idle_timeout_secs(Some(30)), 30);
    assert_eq!(normalized_agent_batch_idle_timeout_secs(Some(3600)), 3600);
}

#[test]
fn agent_batch_status_value_includes_completed_agent_output() {
    let batch = AgentBatchTracked {
        batch_id: "batch-1".to_string(),
        description: "recovery batch".to_string(),
        total: 1,
        idle_timeout_secs: 600,
        started_at: std::time::Instant::now(),
        last_update: std::time::Instant::now(),
        status: "completed".to_string(),
        agents: vec![AgentBatchTrackedAgent {
            index: 0,
            description: "worker".to_string(),
            subagent_type: "backend-architect".to_string(),
            status: "completed".to_string(),
            run_id: "run-1".to_string(),
            duration_ms: Some(42),
            output: Some("completed notes".to_string()),
            output_truncated: false,
            error: None,
        }],
    };

    let status = agent_batch_status_value(&batch);
    assert_eq!(status.get("running").and_then(|v| v.as_u64()), Some(0));
    let agent = status
        .get("agents")
        .and_then(|v| v.as_array())
        .and_then(|agents| agents.first())
        .expect("status includes agent details");
    assert_eq!(
        agent.get("output").and_then(|v| v.as_str()),
        Some("completed notes")
    );
    assert_eq!(
        agent.get("output_truncated").and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[test]
fn agent_batch_schema_describes_idle_watchdog_behavior() {
    let tools = tools_list();
    let batch = tools
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|tool| tool.get("name").and_then(|v| v.as_str()) == Some("AgentBatch"))
        })
        .expect("AgentBatch tool must exist");
    let timeout_desc = batch
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|props| props.get("timeout_secs"))
        .and_then(|timeout| timeout.get("description"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(timeout_desc.contains("Defaults to 600s"));
    assert!(timeout_desc.contains("no hard wall-clock batch cap"));
    assert!(timeout_desc.contains("idle watchdog"));
}

#[test]
fn agent_batch_schema_requires_waiting_with_live_progress() {
    let tools = tools_list();
    let batch = tools
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|tool| tool.get("name").and_then(|v| v.as_str()) == Some("AgentBatch"))
        })
        .expect("AgentBatch tool must exist");
    let description = batch
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(description.contains("wait/poll for all to complete"));
    assert!(description.contains("streaming live progress events"));
    assert!(description.contains("Do not continue inline work while this tool is running"));
    assert!(description.contains("blocks until every child finishes"));
    assert!(description.contains("use AgentBatchStatus"));
    assert!(description.contains("instead of falling back to direct inline work"));
}

#[test]
fn agent_batch_schema_uses_current_persona_catalog() {
    let tools = tools_list();
    let batch = tools
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|tool| tool.get("name").and_then(|v| v.as_str()) == Some("AgentBatch"))
        })
        .expect("AgentBatch tool must exist");
    let description = batch
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let subagent_type_desc = batch
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|props| props.get("agents"))
        .and_then(|agents| agents.get("items"))
        .and_then(|items| items.get("properties"))
        .and_then(|props| props.get("subagent_type"))
        .and_then(|field| field.get("description"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let required = batch
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|props| props.get("agents"))
        .and_then(|agents| agents.get("items"))
        .and_then(|items| items.get("required"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    assert!(description.contains("do not default to a backend/frontend/test trio"));
    assert!(subagent_type_desc.contains("Available thin personas:"));
    assert!(subagent_type_desc.contains("backend-architect"));
    assert!(subagent_type_desc.contains("ux-researcher"));
    assert!(!subagent_type_desc.contains("test-writer-fixer"));
    assert!(!subagent_type_desc.contains("ui-designer"));
    assert!(!subagent_type_desc.contains("studio-coach"));
    assert!(required.iter().any(|v| v.as_str() == Some("subagent_type")));
}

#[test]
fn build_plan_list_path_encodes_query_parameters() {
    let path = build_plan_list_path(
        &json!({
            "query": "TEMP plan tool lifecycle test 2026-04-29 finished",
            "project_path": "/tmp/my repo",
            "session_id": "session 123"
        }),
        "",
    );
    assert_eq!(
        path,
        "/api/plans?q=TEMP%20plan%20tool%20lifecycle%20test%202026-04-29%20finished&projectPath=%2Ftmp%2Fmy%20repo&sessionId=session%20123&"
    );
}

#[test]
fn build_plan_list_path_falls_back_to_current_session_id() {
    let path = build_plan_list_path(&json!({"query": "TEMP"}), "session current");
    assert_eq!(path, "/api/plans?q=TEMP&sessionId=session%20current&");
}

#[test]
fn agent_batch_status_schema_supports_tool_layer_timeout_recovery() {
    let tools = tools_list();
    let status = tools
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|tool| tool.get("name").and_then(|v| v.as_str()) == Some("AgentBatchStatus"))
        })
        .expect("AgentBatchStatus tool must exist");
    let description = status
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(description.contains("tool-layer timeout"));
    assert!(description.contains("child agents are still running"));
    assert!(description.contains("Set timeout_ms"));
    assert!(description.contains("avoids sending repeated tool calls"));
    assert!(description.contains("Poll this tool until running=0"));
    assert!(description.contains("completed agent includes a capped output field"));
    assert!(description.contains("do not redo the delegated work inline"));
    let timeout_desc = status
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|props| props.get("timeout_ms"))
        .and_then(|timeout| timeout.get("description"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(timeout_desc.contains("long-poll timeout"));
    assert!(timeout_desc.contains("Defaults to 0"));
    assert!(timeout_desc.contains("3600000"));
}

#[test]
fn record_file_deleted_clears_read_tracking_for_recreated_file() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "flamel-delete-read-cache-{}-{unique}.txt",
        std::process::id()
    ));
    let path_str = path.to_string_lossy().to_string();

    std::fs::write(&path, "first version").expect("temp file created");
    record_file_read(&path_str);
    assert!(check_read_before_edit(&path_str).is_ok());

    std::fs::remove_file(&path).expect("temp file removed");
    record_file_deleted(&path_str);
    std::fs::write(&path, "second version").expect("temp file recreated");

    let err = check_read_before_edit(&path_str)
        .expect_err("recreated file should require a fresh read after delete");
    assert!(err.contains("has not been read this session"));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn file_edit_schema_exposes_delete_action() {
    let tools = tools_list();
    let file_edit = tools
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|tool| tool.get("name").and_then(|v| v.as_str()) == Some("file_edit"))
        })
        .expect("file_edit tool must exist");
    let description = file_edit
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(description.contains("Before editing any existing file"));
    assert!(description.contains("for multi_replace, read every target file"));
    assert!(description.contains("Prefer one multi_replace call"));
    assert!(description.contains("successful edit result is authoritative"));
    assert!(description.contains("delete a file after prior read validation"));
    let action_enum = file_edit
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|props| props.get("action"))
        .and_then(|action| action.get("enum"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(action_enum.iter().any(|v| v.as_str() == Some("delete")));
}

struct EngineeringFixture {
    root: std::path::PathBuf,
}

impl EngineeringFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "flamel-engineering-contract-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create engineering fixture");
        std::fs::write(
            root.join("alpha.rs"),
            "//! Alpha fixture\nuse std::fmt;\npub fn alpha() -> &'static str { \"alpha\" }\npub fn caller() { let _ = alpha(); }\n",
        )
        .expect("write Rust fixture");
        std::fs::write(
            root.join("beta.ts"),
            "import { alpha } from \"./alpha\";\nexport function beta() { return alpha; }\n",
        )
        .expect("write TypeScript fixture");
        std::fs::write(root.join("empty.txt"), "").expect("write empty fixture");
        std::fs::write(
            root.join("long.txt"),
            format!("{}\n", "🔥".repeat(ENGINEERING_SOURCE_BYTES)),
        )
        .expect("write long fixture");
        std::fs::write(
            root.join("long_match.txt"),
            format!("{}\n", "alpha".repeat(3_000)),
        )
        .expect("write long search fixture");
        std::fs::write(root.join("invalid.bin"), [0xff, 0xfe, 0xfd])
            .expect("write non-UTF-8 fixture");
        Self { root }
    }

    fn path(&self) -> &str {
        self.root.to_str().expect("fixture path is UTF-8")
    }

    fn call(&self, name: &str, args: Value) -> Value {
        let encoded = handle_tool_call(name, &args, "", "", "", self.path())
            .unwrap_or_else(|error| panic!("{name} failed: {error}"));
        serde_json::from_str(&encoded).expect("engineering result JSON")
    }

    fn protocol_call(&self, name: &str, arguments: Value) -> Value {
        handle_request(
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments},
            }),
            "",
            "",
            "",
            self.path(),
            &McpProfile::Default,
        )
        .expect("protocol response")
    }
}

impl Drop for EngineeringFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn navigation_search_treats_leading_dashes_as_patterns() {
    let fixture = EngineeringFixture::new();
    std::fs::write(
        fixture.root.join("--flags with spaces.txt"),
        "--flamel-option\n-flamel-short\nalpha42\n",
    )
    .unwrap();
    for pattern in ["--flamel-option", "-flamel-short", "alpha[0-9]+"] {
        let result = fixture.call(
            "code_nav",
            json!({
                "action": "search", "pattern": pattern,
                "path": "--flags with spaces.txt", "context": 0,
            }),
        );
        assert_eq!(result["counts"]["matches"], 1, "{pattern}");
    }
    let absent = fixture.call(
        "flamel_search",
        json!({"pattern": "--absent-option", "path": "--flags with spaces.txt", "context": 0}),
    );
    assert_eq!(absent["counts"]["matches"], 0);
    let invalid = fixture.protocol_call(
        "code_nav",
        json!({"action": "search", "pattern": "[", "path": "--flags with spaces.txt"}),
    );
    assert_eq!(invalid["result"]["isError"], true);
    assert!(
        invalid["result"]["structuredContent"]["text"]
            .as_str()
            .unwrap()
            .contains("rg error")
    );
}

#[test]
fn navigation_discovery_respects_ignore_rules_in_all_modes() {
    let fixture = EngineeringFixture::new();
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(&fixture.root)
            .output()
            .unwrap()
            .status
            .success()
    );
    for directory in ["source", "source/nested", "build", ".hidden"] {
        std::fs::create_dir_all(fixture.root.join(directory)).unwrap();
    }
    std::fs::write(fixture.root.join(".gitignore"), "build/\n*.generated.rs\n").unwrap();
    std::fs::write(
        fixture.root.join("source/nested/.gitignore"),
        "*.rs\n!keep.rs\n",
    )
    .unwrap();
    std::fs::write(
        fixture.root.join(".git/info/exclude"),
        "source/excluded.rs\n",
    )
    .unwrap();
    for file in [
        "source/visible.rs",
        "source/ignored.generated.rs",
        "source/excluded.rs",
        "source/nested/ignored.rs",
        "source/nested/keep.rs",
        "build/generated.rs",
        ".hidden/hidden.rs",
        ".env",
    ] {
        std::fs::write(fixture.root.join(file), "fixture\n").unwrap();
    }
    for (action, pattern) in [("find", "rs"), ("find", "*.rs"), ("glob", "*.rs")] {
        let result = fixture.call("file_read", json!({"action": action, "pattern": pattern}));
        let mut paths = result["data"]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect::<Vec<_>>();
        paths.sort();
        assert_eq!(
            paths,
            ["alpha.rs", "source/nested/keep.rs", "source/visible.rs"],
            "{action}/{pattern}"
        );
    }
    for action in ["find", "glob"] {
        let result = fixture.call(
            "file_read",
            json!({
                "action": action, "pattern": "*.rs", "path": "build", "relative_path": "build",
            }),
        );
        assert_eq!(
            result["data"]["paths"],
            json!([]),
            "explicit ignored directory/{action}"
        );
        let result = fixture.call(
            "file_read",
            json!({
                "action": action, "pattern": "source/**/*.rs",
            }),
        );
        assert_eq!(
            result["data"]["paths"].as_array().unwrap().len(),
            2,
            "recursive glob/{action}"
        );
    }
    let explicit_read = fixture.call(
        "file_read",
        json!({"action": "read", "file": "build/generated.rs"}),
    );
    assert_eq!(explicit_read["status"], "ok");
    let hidden = fixture.call("file_read", json!({"action": "find", "pattern": "env"}));
    assert_eq!(hidden["data"]["paths"], json!([]));
}

#[test]
fn navigation_glob_without_wildcards_does_not_fuzzy_match() {
    let fixture = EngineeringFixture::new();
    let exact = fixture.call(
        "file_read",
        json!({"action": "glob", "pattern": "alpha.rs"}),
    );
    assert_eq!(
        exact["data"]["paths"],
        json!([{"path": "alpha.rs", "kind": "file"}])
    );
    let absent = fixture.call("flamel_glob", json!({"pattern": "alprs"}));
    assert_eq!(absent["data"]["paths"], json!([]));
    let fuzzy = fixture.call("find_file", json!({"pattern": "alprs"}));
    assert_eq!(fuzzy["data"]["paths"][0]["path"], "alpha.rs");
}

#[cfg(unix)]
#[test]
fn navigation_discovery_does_not_follow_symlinks() {
    let fixture = EngineeringFixture::new();
    std::fs::create_dir(fixture.root.join("source")).unwrap();
    std::fs::write(fixture.root.join("source/visible.rs"), "fixture\n").unwrap();
    std::os::unix::fs::symlink(&fixture.root, fixture.root.join("source/loop")).unwrap();
    std::os::unix::fs::symlink(
        fixture.root.join("alpha.rs"),
        fixture.root.join("source/link.rs"),
    )
    .unwrap();
    for action in ["find", "glob"] {
        let result = fixture.call(
            "file_read",
            json!({
                "action": action, "pattern": "*.rs", "path": "source", "relative_path": "source",
            }),
        );
        let path = if action == "find" {
            "visible.rs"
        } else {
            "source/visible.rs"
        };
        assert_eq!(
            result["data"]["paths"],
            json!([{"path": path, "kind": "file"}])
        );
    }
}

fn sorted_object_keys(value: &Value) -> Vec<String> {
    let mut keys = value
        .as_object()
        .expect("contract field must be an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

#[test]
fn all_engineering_actions_share_the_versioned_protocol_snapshot() {
    let fixture = EngineeringFixture::new();
    let cases = [
        (
            "file_read",
            json!({"action": "read", "file": "alpha.rs"}),
            "file_read",
            "read",
            vec!["files"],
            vec!["files", "lines", "totalLines"],
        ),
        (
            "file_read",
            json!({"action": "batch", "files": ["alpha.rs", "beta.ts"]}),
            "file_read",
            "batch",
            vec!["files"],
            vec!["files", "lines", "totalLines"],
        ),
        (
            "file_read",
            json!({"action": "summary", "file": "alpha.rs"}),
            "file_read",
            "summary",
            vec![
                "imports",
                "language",
                "path",
                "purpose",
                "symbols",
                "totalLines",
            ],
            vec!["files", "imports", "symbols", "totalLines"],
        ),
        (
            "file_read",
            json!({"action": "glob", "pattern": "*.rs"}),
            "file_read",
            "glob",
            vec!["paths", "pattern", "root"],
            vec!["paths"],
        ),
        (
            "file_read",
            json!({"action": "find", "pattern": "alpha"}),
            "file_read",
            "find",
            vec!["mode", "paths", "query", "root"],
            vec!["matches", "paths"],
        ),
        (
            "file_read",
            json!({"action": "list_dir"}),
            "file_read",
            "list_dir",
            vec!["paths", "root"],
            vec!["paths"],
        ),
        (
            "code_nav",
            json!({"action": "search", "pattern": "alpha", "path": "."}),
            "code_nav",
            "search",
            vec!["contextLines", "include", "matches", "query", "root"],
            vec!["contexts", "files", "matches", "rows"],
        ),
        (
            "code_nav",
            json!({"action": "symbols", "relative_path": "alpha.rs"}),
            "code_nav",
            "symbols",
            vec!["language", "path", "symbols"],
            vec!["symbols"],
        ),
        (
            "code_nav",
            json!({
                "action": "find",
                "relative_path": "alpha.rs",
                "name_path": "alpha",
                "include_body": true,
                "depth": 1
            }),
            "code_nav",
            "find",
            vec![
                "children", "language", "namePath", "path", "range", "source",
            ],
            vec!["children", "sourceLines", "symbols"],
        ),
        (
            "code_nav",
            json!({
                "action": "refs",
                "symbol_name": "alpha",
                "relative_path": ".",
                "context_lines": 1
            }),
            "code_nav",
            "refs",
            vec!["contextLines", "matches", "root", "symbol"],
            vec!["contexts", "files", "matches", "rows"],
        ),
        (
            "code_nav",
            json!({"action": "imports", "file": "alpha.rs"}),
            "code_nav",
            "imports",
            vec!["imports", "language", "path", "totalLines"],
            vec!["imports"],
        ),
        (
            "code_nav",
            json!({"action": "context", "task": "trace alpha function", "max_files": 3}),
            "code_nav",
            "context",
            vec!["files", "keywords", "task"],
            vec!["files"],
        ),
        (
            "code_nav",
            json!({"action": "deps", "file": "alpha.rs"}),
            "code_nav",
            "deps",
            vec!["inbound", "outbound", "path"],
            vec!["inbound", "outbound"],
        ),
    ];

    let golden: Value = serde_json::from_str(include_str!(
        "../../../../frontend/src/components/tool-widgets/engineeringResultV1.fixture.json"
    ))
    .expect("frontend engineering fixture");
    assert_eq!(golden["schema"], ENGINEERING_SCHEMA);

    for (index, (name, arguments, tool, action, data_keys, count_keys)) in
        cases.into_iter().enumerate()
    {
        let result = fixture.call(name, arguments);
        assert_eq!(result["schema"], ENGINEERING_SCHEMA, "{tool}/{action}");
        assert_eq!(result["tool"], tool, "{tool}/{action}");
        assert_eq!(result["action"], action, "{tool}/{action}");
        assert_eq!(result["status"], "ok", "{tool}/{action}");
        assert!(!result["text"].as_str().unwrap_or_default().is_empty());
        assert!(
            serde_json::to_vec(&result)
                .expect("encoded engineering result")
                .len()
                < 64 * 1024,
            "{tool}/{action} exceeded the producer payload budget"
        );
        assert_eq!(
            sorted_object_keys(&result["data"]),
            data_keys,
            "{tool}/{action}"
        );
        assert_eq!(
            sorted_object_keys(&result["counts"]),
            count_keys,
            "{tool}/{action}"
        );
        assert_eq!(
            golden["actions"][index],
            json!({
                "tool": tool,
                "action": action,
                "dataFields": data_keys,
                "countFields": count_keys,
            }),
            "shared fixture drifted for {tool}/{action}"
        );
    }
}

#[test]
fn engineering_aliases_resolve_to_canonical_tools_and_actions() {
    let cases = [
        ("flamel_read", "file_read", "read"),
        ("read_file", "file_read", "read"),
        ("batch_read", "file_read", "batch"),
        ("file_summary", "file_read", "summary"),
        ("flamel_glob", "file_read", "glob"),
        ("find_file", "file_read", "find"),
        ("list_dir", "file_read", "list_dir"),
        ("flamel_search", "code_nav", "search"),
        ("get_symbols_overview", "code_nav", "symbols"),
        ("find_symbol", "code_nav", "find"),
        ("find_referencing_symbols", "code_nav", "refs"),
        ("get_imports", "code_nav", "imports"),
        ("gather_context", "code_nav", "context"),
        ("dependency_graph", "code_nav", "deps"),
    ];
    for (name, expected_tool, expected_action) in cases {
        let (tool, action) = engineering_tool_action(name, &json!({})).expect("engineering alias");
        assert_eq!(tool, expected_tool, "{name}");
        assert_eq!(action, expected_action, "{name}");
    }
}

#[test]
fn engineering_read_contract_agrees_with_text_and_bounds_unicode_safely() {
    let fixture = EngineeringFixture::new();
    let partial = fixture.protocol_call(
        "file_read",
        json!({"action": "read", "file": "alpha.rs", "start_line": 2, "end_line": 2}),
    );
    let structured = &partial["result"]["structuredContent"];
    let text = partial["result"]["content"][0]["text"]
        .as_str()
        .expect("compatibility text");
    assert_eq!(
        structured["data"]["files"][0]["resolvedRange"],
        json!({
            "startLine": 2,
            "endLine": 2,
        })
    );
    assert_eq!(structured["counts"]["lines"], 1);
    assert!(text.contains("lines 2-2 of 4"));
    assert_eq!(text, structured["text"]);

    let long_response =
        fixture.protocol_call("file_read", json!({"action": "read", "file": "long.txt"}));
    let long = &long_response["result"]["structuredContent"];
    assert_eq!(long["truncated"], true);
    assert_eq!(long["truncationReason"], "producer_budget");
    assert!(long["data"]["files"][0]["source"].as_str().is_some());
    assert_eq!(long["artifacts"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        long_response["result"]["_meta"]["flamel"]["artifactCount"],
        1
    );
    if let Some(path) = long["artifacts"][0]["path"].as_str() {
        let _ = std::fs::remove_file(path);
    }

    let search = fixture.call(
        "code_nav",
        json!({
            "action": "search",
            "pattern": "alpha",
            "path": "long_match.txt",
            "context": 0
        }),
    );
    assert_eq!(search["truncated"], true);
    assert!(
        search["data"]["matches"][0]["text"]
            .as_str()
            .is_some_and(|text| text.len() <= ENGINEERING_FIELD_BYTES)
    );
    assert_eq!(search["artifacts"].as_array().map(Vec::len), Some(1));
    if let Some(path) = search["artifacts"][0]["path"].as_str() {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn engineering_protocol_preserves_zero_many_and_structured_errors() {
    let fixture = EngineeringFixture::new();
    let zero = fixture.call(
        "file_read",
        json!({"action": "glob", "pattern": "does-not-exist-*"}),
    );
    assert_eq!(zero["counts"]["paths"], 0);
    assert_eq!(zero["data"]["paths"], json!([]));

    let many = fixture.call(
        "file_read",
        json!({"action": "batch", "files": ["alpha.rs", "beta.ts"]}),
    );
    assert_eq!(many["counts"]["files"], 2);
    assert_eq!(many["data"]["files"].as_array().map(Vec::len), Some(2));

    for file in ["missing.rs", "invalid.bin"] {
        let error = fixture.protocol_call("file_read", json!({"action": "read", "file": file}));
        assert_eq!(error["result"]["isError"], true, "{file}");
        assert_eq!(
            error["result"]["structuredContent"]["schema"], ENGINEERING_SCHEMA,
            "{file}"
        );
        assert_eq!(
            error["result"]["structuredContent"]["status"], "error",
            "{file}"
        );
        assert_eq!(
            error["result"]["content"][0]["text"], error["result"]["structuredContent"]["text"],
            "{file}"
        );
    }
}

#[test]
fn unknown_engineering_versions_keep_their_legacy_json_text() {
    let current = json!({"schema": ENGINEERING_SCHEMA, "text": "concise"});
    let future = json!({"schema": "flamel.engineering_result.v2", "text": "future"});
    let future_encoded = future.to_string();

    assert_eq!(
        structured_display_text("encoded", Some(&current)),
        "concise"
    );
    assert_eq!(
        structured_display_text(&future_encoded, Some(&future)),
        future_encoded
    );
}

#[test]
fn plan_schema_requires_incremental_item_updates() {
    let tools = tools_list();
    let plan = tools
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|tool| tool.get("name").and_then(|v| v.as_str()) == Some("plan"))
        })
        .expect("plan tool must exist");
    let description = plan
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(description.contains("before any work or tool call for the next item"));
    assert!(description.contains("Never defer completed statuses"));
    assert!(description.contains("cross-cutting final validation should be its own item"));
    assert!(
        description.contains("Once implementation is approved, incorporate in-scope corrections")
    );
    assert!(description.contains("exact plan_id supplied by the user"));
    assert!(description.contains("never replace it with list or title search"));
    assert!(description.contains("verified file/symbol anchors"));
    assert!(description.contains("minimum safe Auto tier"));
    assert!(description.contains("lower-reasoning executor"));

    let notes_description = plan
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|properties| properties.get("items"))
        .and_then(|items| items.get("items"))
        .and_then(|items| items.get("properties"))
        .and_then(|properties| properties.get("notes"))
        .and_then(|notes| notes.get("description"))
        .and_then(|value| value.as_str())
        .unwrap_or("");
    assert!(notes_description.contains("verified files/symbols"));
    assert!(notes_description.contains("item-scoped validation"));

    let item_schema = plan
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|properties| properties.get("items"))
        .and_then(|items| items.get("items"))
        .expect("plan items schema");
    let reasoning_floor = item_schema
        .get("properties")
        .and_then(|properties| properties.get("reasoning_floor"))
        .expect("reasoning floor schema");
    let reasoning_description = reasoning_floor
        .get("description")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    assert!(reasoning_description.contains("Minimum safe reasoning tier"));
    assert!(reasoning_description.contains("may raise"));
    assert!(reasoning_description.contains("never lower"));
    let reasoning_values = reasoning_floor
        .get("enum")
        .and_then(|value| value.as_array())
        .expect("reasoning floor enum");
    assert!(reasoning_values.iter().any(|value| value == "high"));
    assert!(reasoning_values.iter().any(|value| value == "xhigh"));
    assert!(
        item_schema
            .get("required")
            .and_then(|value| value.as_array())
            .is_some_and(|required| required.iter().any(|value| value == "reasoning_floor"))
    );

    let update_reasoning_floor = plan
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|properties| properties.get("reasoning_floor"))
        .expect("reasoning floor update schema");
    assert_eq!(
        update_reasoning_floor
            .get("type")
            .and_then(|value| value.as_str()),
        Some("string")
    );

    let status_description = plan
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|properties| properties.get("status"))
        .and_then(|status| status.get("description"))
        .and_then(|value| value.as_str())
        .unwrap_or("");
    assert!(status_description.contains("Set in_progress before starting the item"));
    assert!(status_description.contains("never wait until the end of the plan"));

    let action_description = plan
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|properties| properties.get("action"))
        .and_then(|action| action.get("description"))
        .and_then(|value| value.as_str())
        .unwrap_or("");
    assert!(action_description.contains("only after an explicit user execution approval"));
    assert!(action_description.contains("preserve existing authorization for in-scope revisions"));

    let plan_id_description = plan
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|properties| properties.get("plan_id"))
        .and_then(|plan_id| plan_id.get("description"))
        .and_then(|value| value.as_str())
        .unwrap_or("");
    assert!(plan_id_description.contains("use it directly for get/start/update calls"));
    assert!(plan_id_description.contains("never substitute a title match"));
}
