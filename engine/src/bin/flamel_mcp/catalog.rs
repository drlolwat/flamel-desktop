use super::*;

pub(super) fn tools_list() -> Value {
    let mut persona_names = persona_registry().keys().cloned().collect::<Vec<_>>();
    persona_names.sort_unstable();
    let persona_field_description = format!(
        "Persona for the sub-agent. Pick the narrowest reviewed persona that fits the delegated task. Available thin personas: {}. Built-ins: Explore (read-only navigation), Plan (planning only), and general-purpose (last resort).",
        persona_names.join(", ")
    );
    let mut tools = json!([
        {
            "name": "Agent",
            "description": "Spawn one focused sub-agent when isolation or independent specialist work materially improves the task. Reviewed personas are thin domain overlays that inherit Flamel runtime rules and activate governed skills; choose from the current subagent_type catalog. Explore remains read-only, Plan remains planning-only, and general-purpose is a last resort. Pass a self-contained prompt because the child has no conversation memory. Use AgentBatch for two or more independent children. Returns the child's final output.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "description": {"type": "string", "description": "Short (3-5 word) description of the task"},
                    "prompt": {"type": "string", "description": "Self-contained task for the sub-agent. Include all context it needs — it does not see this conversation."},
                    "subagent_type": {"type": "string", "description": persona_field_description.clone()},
                    "model": {"type": "string", "description": "Optional compatible child-model override. On an xhigh/max/ultra parent, prefer a lower-cost model for bounded discovery or review when the parent will verify the evidence. Otherwise inherit the parent unless the user or approved Plan explicitly directs delegation."}
                },
                "required": ["description", "prompt", "subagent_type"]
            }
        },
        {
            "name": "plan",
            "description": "Manage plans — create, read, update, and track progress live. Plan items are implementation handoffs: every item carries a structured reasoning_floor selected by the high-reasoning planner as the minimum safe Auto tier. Auto execution may escalate above that floor when concrete evidence warrants it, but must never downgrade it. Substantial item notes must contain verified file/symbol anchors, behavior and data/control-flow contracts, dependencies/order, item-scoped validation and acceptance evidence, material risks/rollback constraints, and genuinely independent delegation opportunities. Resolve repository questions during planning so a lower-reasoning executor does not repeat broad discovery. An exact plan_id supplied by the user or Plan-panel Start action is authoritative even across sessions: call get directly and start that same ID; never replace it with list or title search. During execution, keep exactly one item in_progress. Mark each item done immediately after its own acceptance checks pass and before any work or tool call for the next item, then mark the next item in_progress. Never defer completed statuses or batch-complete items at the end; cross-cutting final validation should be its own item. Actions: create (new plan, STOP after), start/resume/pause/block (execution state), get (read items+IDs), list (find plans; defaults to the current chat session when available), update_item (change status/text/notes/reasoning floor), add_items (append), delete_item, replace_items (restructure), rename, mark_finished. A new pending plan needs explicit execution approval. Once implementation is approved, incorporate in-scope corrections and continue the same plan without another approval round. Pause only for a concrete missing decision, user Stop, or an out-of-scope action.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["create", "start", "resume", "pause", "block", "get", "list", "update_item", "add_items", "delete_item", "replace_items", "rename", "mark_finished", "require_browser_review", "browser_review"], "description": "create: new plan (needs title, items). start/resume: begin only after an explicit user execution approval for an unapproved plan; preserve existing authorization for in-scope revisions (needs plan_id). pause/block: update execution state. get: read plan (needs plan_id). list: find plans. update_item: change item (needs plan_id, item_id). add_items: append (needs plan_id, items). delete_item: remove (needs plan_id, item_id). replace_items: restructure (needs plan_id, items). rename: change title (needs plan_id, title). mark_finished: archive (needs plan_id)."},
                    "plan_id": {"type": "string", "description": "Exact plan ID. If the user or Plan-panel Start message supplies one, use it directly for get/start/update calls even if session-scoped list does not return it; never substitute a title match or another session plan."},
                    "browser_review": {"type":"object","description":"Required for browser_review: desktopToolId and mobileToolId from distinct successful browser screenshot calls, functionalToolId from successful browser actions/test command, revision, journeys (URLs, auth states, actual assertions and reload/persistence outcomes), visualFindings (reviewed desktop/mobile screenshots, layout/copy/accessibility findings and fixes). Evidence must be recorded in this plan session after review was required. Use plan(action=get) browserReview.availableReceipts to look up actual transport tool IDs; screenshot artifact names are not tool IDs."},
                    "item_id": {"type": "string"},
                    "title": {"type": "string", "description": "Plan title (create, rename)"},
                    "description": {"type": "string", "description": "Plan description (create, rename)"},
                    "items": {
                        "type": "array",
                        "description": "Plan items (create, replace_items, add_items)",
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": {"type": "string", "description": "Short item title (1 line)"},
                                "status": {"type": "string", "enum": ["todo", "in_progress", "done", "blocked"]},
                                "section": {"type": "string", "description": "Phase/section name for grouping"},
                                "notes": {"type": "string", "description": "Execution-ready markdown handoff: verified files/symbols or subsystems; intended behavior and data/control-flow contracts; dependencies/order; item-scoped validation and acceptance evidence; material risks/rollback constraints; independent delegation opportunities. Do not invent unverified anchors."},
                                "reasoning_floor": {"type": "string", "enum": ["low", "medium", "high", "xhigh", "max", "ultra"], "description": "Minimum safe reasoning tier when execution uses Auto. The executor may raise this value after discovering concrete complexity but must never lower the planner-selected value."}
                            },
                            "required": ["text", "reasoning_floor"]
                        }
                    },
                    "status": {"type": "string", "enum": ["todo", "in_progress", "done", "blocked"], "description": "Item status (update_item). Set in_progress before starting the item. Set done immediately after that item's acceptance checks pass and before starting the next item; never wait until the end of the plan."},
                    "text": {"type": "string", "description": "Item text (update_item)"},
                    "section": {"type": "string", "description": "Item section (update_item)"},
                    "notes": {"type": "string", "description": "Execution-ready item handoff (update_item): verified anchors, contracts, dependencies, validation/evidence, risks/rollback, and independent delegation opportunities."},
                    "reasoning_floor": {"type": "string", "enum": ["low", "medium", "high", "xhigh", "max", "ultra"], "description": "Minimum safe Auto reasoning tier. During execution this may only be raised, never lowered."},
                    "query": {"type": "string", "description": "Search query (list)"},
                    "project_path": {"type": "string", "description": "Filter by project (list)"},
                    "session_id": {"type": "string", "description": "Filter by session (list). Defaults to the current chat session when available."}
                },
                "required": ["action"]
            }
        },
        {
            "name": "assistant",
            "description": "One typed conversational administration surface for Flamel Assistant. Read status, create and inspect bounded same-owner assistant delegations, inspect or manage the private brain, manage reviewed skills and signed packs, authorize or revoke connections, preview and control automations, operate durable Jobs, inspect bounded diagnostics, and run confirmed idempotent recoveries or visible synthetic end-to-end probes. Every mutation previews first unless confirm=true; confirmed mutations require a caller-stable idempotency_key. Delegation sender and parent identity are injected by the runtime and cannot be supplied in arguments. Identity is always resolved by Flamel from the authenticated session or opaque Assistant channel invocation.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["assistant.status", "assistant.set_provider", "delegation.list", "delegation.create", "delegation.approve", "delegation.cancel", "brain.inspect", "brain.remember", "brain.forget", "brain.export", "capability.snapshot", "skill.list", "skill.inspect", "skill.propose", "skill.review", "skill.install", "skill.enable", "skill.disable", "skill.rollback", "skill.uninstall", "pack.list", "pack.preview", "pack.install", "pack.rollback", "pack.uninstall", "openclaw.preview", "connection.list", "connection.authorize", "connection.revoke", "connection.test", "connection.tools", "connection.set_tool", "automation.list", "automation.get", "automation.preview", "automation.create", "automation.update", "automation.pause", "automation.resume", "automation.run", "automation.retry", "automation.history", "automation.delete", "job.list", "job.get", "job.create", "job.update", "job.run", "job.rerun", "job.pause", "job.resume", "job.cancel", "job.steer", "job.approve", "job.events", "diagnostics.status", "diagnostics.recover"]
                    },
                    "arguments": {"type": "object", "description": "Action-specific arguments. The server applies an exact field allowlist for the selected action."},
                    "confirm": {"type": "boolean", "description": "Omit or false to preview a mutation. Set true only after user approval."},
                    "idempotency_key": {"type": "string", "maxLength": 200, "description": "Caller-stable key required for every confirmed mutation and replayed by the authoritative service."},
                    "channel_invocation_id": {"type": "string", "description": "Opaque FlamelInvocationId from the current authorized Chattle, Telegram, or Slack message. Never substitute raw actor, space, channel, or message IDs."},
                    "chattle_invocation_id": {"type": "string", "description": "Deprecated Chattle-only alias for channel_invocation_id. Never send both."}
                },
                "required": ["action"]
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "contractVersion": {"type": "integer"},
                    "surfaceContract": {"type": "string"},
                    "action": {"type": "string"},
                    "ok": {"type": "boolean"},
                    "state": {"type": "string", "enum": ["approval_required", "authorization_required", "confirmed", "failed"]},
                    "approvalRequired": {"type": "boolean"},
                    "mutated": {"type": "boolean"},
                    "replayed": {"type": "boolean"},
                    "summary": {"type": "object"},
                    "result": {}
                },
                "required": ["contractVersion", "surfaceContract", "action", "ok", "state", "approvalRequired", "mutated", "replayed", "summary"]
            }
        },
        {
            "name": "job",
            "description": "Canonical durable control plane for Flamel repository Jobs, exact issue or pull-request targets, run lifecycle, approvals, event replay, and Automation Profiles. Use this for natural repository work from Work or an Assistant channel instead of gh, raw Forge APIs, or guessed repository tools. Personal calls are limited to pull_request autonomy; merge, deploy, secret, environment, and arbitrary API operations are not exposed.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "get", "create", "update", "run", "rerun", "pause", "resume", "cancel", "steer", "approve", "events", "profile_list", "profile_get", "profile_preview", "profile_create", "profile_update", "profile_delete", "profile_enable", "profile_disable", "profile_pause", "profile_resume", "profile_run", "profile_retry", "profile_event"],
                        "description": "Exact Job or Automation Profile action."
                    },
                    "job_id": {"type": "string", "description": "Job UUID for get/update/run/rerun/control/events."},
                    "run_id": {"type": "string", "description": "Run UUID for pause/resume/cancel/steer/approve/events."},
                    "profile_id": {"type": "string", "description": "Automation Profile UUID for profile actions."},
                    "occurrence_id": {"type": "string", "description": "Failed Automation occurrence UUID for profile_retry."},
                    "owner": {"type": "string", "description": "Exact Flamel Git repository owner for create."},
                    "repo": {"type": "string", "description": "Exact Flamel Git repository name for create."},
                    "title": {"type": "string", "description": "Job title for create/update."},
                    "objective": {"type": "string", "description": "Concrete Job objective for create/update."},
                    "autonomy": {"type": "string", "enum": ["pull_request"], "description": "Personal MCP Jobs are constrained to pull_request autonomy."},
                    "repository_ids": {"type": "array", "maxItems": 16, "items": {"type": "string"}, "description": "Exact additional repository UUIDs for create."},
                    "channel_invocation_id": {"type": "string", "description": "Opaque FlamelInvocationId from the current authorized Assistant channel message. Use only for create and never invent it or substitute raw destination IDs."},
                    "chattle_invocation_id": {"type": "string", "description": "Deprecated Chattle-only alias for channel_invocation_id. Never send both."},
                    "notification_policy": {"type": "string", "enum": ["terminal_only", "milestones", "muted"], "description": "Proactive update policy for a channel-promoted Job."},
                    "settings": {"type": "object", "description": "Bounded Job execution settings, including exact target and allowed_operations. Origin metadata is server-managed."},
                    "instruction": {"type": "string", "description": "Run guidance for steer."},
                    "cursor": {"type": "integer", "minimum": 0, "description": "Event cursor for events."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500, "description": "Bounded list or event result count."},
                    "idempotency_key": {"type": "string", "description": "Caller-stable key for profile_run or profile_retry."},
                    "event_kind": {"type": "string", "description": "Exact webhook event kind granted by the Automation Profile."},
                    "delivery_id": {"type": "string", "description": "Provider-stable delivery ID used to deduplicate profile_event."},
                    "payload": {"description": "Bounded non-secret public event payload for profile_event."},
                    "profile": {"type": "object", "description": "Whitelisted Automation Profile create, update, or preview fields, including assistant actions, exact capability grants, Chattle destination, retry policy, trigger, schedule, repository target, and Job policy."}
                },
                "required": ["action"]
            }
        },
        {
            "name": "file_history",
            "description": "File history and checkpoints. Actions: revert — undo edits from backup history (params: file, steps?); list — show edit history (params: file?); checkpoint — save named snapshot via git stash (params: name?); rollback — restore a named checkpoint (params: name). Legacy names: revert_file, list_file_edits, checkpoint, rollback.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["revert", "list", "checkpoint", "rollback"], "description": "Action (default: list)"},
                    "file": {"type": "string", "description": "File path (for revert/list)"},
                    "steps": {"type": "number", "description": "Edits to go back (for revert, default: 1)"},
                    "name": {"type": "string", "description": "Checkpoint name (for checkpoint/rollback)"}
                }
            }
        },
        {
            "name": "flamel_context_stats",
            "description": "Check how full the context window is. Returns current token usage, % full, remaining tokens, auto-compact threshold, and estimated turns remaining. Call this when you're unsure if you're close to the context limit, or proactively every ~20 turns on long sessions.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "diagnostics",
            "description": "Admin-only Flamel server diagnostics. Actions: runtime_logs — return recent sanitized server-side tracing events captured by the Flamel runtime log buffer. Requires the forwarded chat user to be an admin.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["runtime_logs"], "description": "Diagnostics action (default: runtime_logs)"},
                    "limit": {"type": "integer", "description": "Maximum log entries to return, 1-200 (default 50)"},
                    "min_level": {"type": "string", "enum": ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"], "description": "Minimum severity to include"}
                }
            }
        },
        {
            "name": "capability_search",
            "description": "Discover Flamel built-ins, signed first-party capability packs, and administrator-approved integrations without loading every schema into context. Pack signature, active/rollback version, exact scopes, and schedule eligibility come from the same Capability Hub catalog shown in Settings. Use connected_only=true when the task requires a capability that is ready for this user.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Plain-language capability, product, or task to find"},
                    "category": {"type": "string", "description": "Optional exact category filter"},
                    "connected_only": {"type": "boolean", "description": "Return only built-in or connected capabilities"}
                }
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "count": {"type": "integer"},
                    "capabilities": {"type": "array", "items": {"type": "object"}},
                    "admin_managed": {"type": "boolean"}
                },
                "required": ["count", "capabilities", "admin_managed"]
            }
        },
        {
            "name": "capability_execute",
            "description": "Execute an infrequent tool from an administrator-approved connected capability. Signed packs enforce their exact granted scopes, enabled tool, and typed input schema before dispatch. Discover the capability and schema first with capability_search; frequent Flamel coding tools remain direct.",
            "inputSchema": {"type":"object","properties":{"capability_id":{"type":"string"},"tool_name":{"type":"string"},"arguments":{"type":"object"},"background":{"type":"boolean"}},"required":["capability_id","tool_name"]},
            "outputSchema": {"type":"object","properties":{"taskId":{"type":"string"},"result":{}},"required":["taskId"]}
        },
        {
            "name": "project_intelligence",
            "description": "Incremental project intelligence. index updates the ignore-aware Tree-sitter index; context returns a token-budgeted evidence bundle with paths, symbols, line ranges, dependencies, reasons, and freshness; verification detects the build/test/lint/format/typecheck graph.",
            "inputSchema":{"type":"object","properties":{"action":{"type":"string","enum":["index","context","verification","verify"]},"project_path":{"type":"string"},"query":{"type":"string"},"max_tokens":{"type":"integer"},"checks":{"type":"array","items":{"type":"string"}}},"required":["action"]}
        },
        {
            "name": "skill_search",
            "description": "Search the governed Flamel instruction-only skill catalog for the current user, project, and provider. Results include provenance, review/signature state, capability requirements, install state, version, and content hash. Use only active approved skills and read their projected SKILL.md before applying one.",
            "inputSchema":{"type":"object","properties":{"query":{"type":"string"},"project_path":{"type":"string"}},"required":["query"]}
        },
        {
            "name": "skill_catalog",
            "description": "Govern Flamel's versioned instruction-only skill catalog. List/search/inspect are read-only. Propose submits content for review. Review and add_version require an administrator. Install, rollback, enable, disable, and uninstall refresh isolated Codex and Claude skill snapshots immediately; no skill action executes third-party code.",
            "inputSchema":{"type":"object","properties":{"action":{"type":"string","enum":["list","search","inspect","propose","review","add_version","install","rollback","enable","disable","uninstall"]},"id":{"type":"string"},"query":{"type":"string"},"project_path":{"type":"string"},"provider":{"type":"string","enum":["codex","claude"]},"slug":{"type":"string"},"version":{"type":"string"},"review_action":{"type":"string","enum":["approve","reject"]},"provider_projection_enabled":{"type":"boolean"},"skill":{"type":"object"}},"required":["action"]}
        },
        {
            "name": "work",
            "description": "Native Engineering Work Hub. Search, create, update, comment, assign human or agent owners, review, link plans/runs, add dependencies/artifacts, and manage release status for project work items.",
            "inputSchema":{"type":"object","properties":{"action":{"type":"string","enum":["list","search","get","create","update","comment","assign","review","link_plan","release_status","add_dependency","add_artifact"]},"id":{"type":"string"},"project_path":{"type":"string"},"query":{"type":"string"},"body":{"type":"string"},"item":{"type":"object"},"depends_on_id":{"type":"string"},"artifact":{"type":"object"}},"required":["action"]}
        },
        {
            "name": "code_nav",
            "description": "Code search and navigation. Actions: search (default) — ripgrep pattern search (params: pattern, path?, include?, context?); symbols — list top-level symbols in file (params: relative_path); find — find symbol by name_path (params: relative_path, name_path, include_body?, depth?); refs — find all references to a symbol (params: symbol_name, relative_path?, context_lines?); imports — list import/use statements (params: file); context — auto-find relevant files for a task (params: task, max_files?); deps — dependency graph for a file (params: file). Legacy: flamel_search, get_symbols_overview, find_symbol, find_referencing_symbols, get_imports, gather_context, dependency_graph.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["search", "symbols", "find", "refs", "imports", "context", "deps"], "description": "Action (default: search)"},
                    "pattern": {"type": "string", "description": "Search pattern (for search)"},
                    "path": {"type": "string", "description": "Subdirectory to search (for search)"},
                    "include": {"type": "string", "description": "Glob filter e.g. \"*.rs\" (for search)"},
                    "context": {"type": "integer", "description": "Context lines (for search, default: 2)"},
                    "relative_path": {"type": "string", "description": "File path (for symbols/find/refs)"},
                    "name_path": {"type": "string", "description": "Symbol path like 'fn_name' or 'Class/method' (for find)"},
                    "include_body": {"type": "boolean", "description": "Include full source (for find)"},
                    "depth": {"type": "integer", "description": "Nesting depth (for find)"},
                    "symbol_name": {"type": "string", "description": "Symbol to search for (for refs)"},
                    "context_lines": {"type": "integer", "description": "Context lines around refs (for refs)"},
                    "file": {"type": "string", "description": "File path (for imports/deps)"},
                    "task": {"type": "string", "description": "Task description (for context)"},
                    "max_files": {"type": "integer", "description": "Max files (for context, default: 5)"}
                }
            }
        },
        {
            "name": "shell",
            "description": "Run shell commands. Actions: run (default) — execute a command with streaming output, blocks until done (300s timeout); start — launch interactive session, returns session_id; read — read buffered output; write — send stdin exactly as supplied; store_secret/delete_secret/secret_status — optional encrypted local storage when the user asks for it; kill — terminate session; status — check if running; multi — run up to 5 commands in parallel. A direct user request to use supplied input is sufficient authorization: send it on the first request without separate confirmation or a mandatory privacy mode. Git/grep/cat/find/sed commands are rejected — use dedicated tools. Legacy names: flamel_shell, shell_run, shell_start, shell_read, shell_write, shell_kill, shell_status, multi_shell.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["run", "start", "read", "write", "store_secret", "delete_secret", "secret_status", "kill", "status", "multi"], "description": "Shell action (default: run)"},
                    "command": {"type": "string", "description": "Shell command to run (for run/start actions)"},
                    "cwd": {"type": "string", "description": "Working directory (defaults to project root)"},
                    "timeout_secs": {"type": "integer", "description": "Max seconds (default 300 for run, 120 for multi)"},
                    "session_id": {"type": "string", "description": "Session ID (for read/write/kill/status actions)"},
                    "timeout_ms": {"type": "integer", "description": "Wait ms for output before reading (for read action, default 100)"},
                    "sensitive": {"type": "boolean", "description": "For write: omit stdin from command events and replay"},
                    "text": {"type": "string", "description": "Text for write or store_secret; redacted when sensitive"},
                    "secret_name": {"type": "string", "description": "Local encrypted secret name for store_secret/delete_secret/secret_status"},
                    "secret_ref": {"type": "string", "description": "For write: inject a stored local secret by name instead of placing it in tool input"},
                    "append_newline": {"type": "boolean", "description": "For write: append a newline after the supplied text or stored secret"},
                    "commands": {"type": "array", "description": "Array of command strings or {command, cwd?} objects (for multi action)", "items": {}}
                }
            }
        },
        // (read_file/find_file/list_dir schemas consolidated into file_read above)
        {
            "name": "file_edit",
            "description": "Edit, create, and format files. Before editing any existing file, first read it in this session with file_read; for multi_replace, read every target file before constructing edits[]. Prefer one multi_replace call for independent exact edits across known files. A successful edit result is authoritative: do not immediately read and apply the same edit again unless the file subsequently changed, the result reported failure, or validation found a mismatch. Actions: replace (default) — exact string swap with quote normalization (params: file, old_string, new_string); replace_content — regex/text pattern replace (params: relative_path, needle, repl, mode?); multi_replace — batch edits across files (params: edits[]); replace_symbol — rewrite entire function/class (params: relative_path, name_path, new_body); insert_before/insert_after — insert code before/after a symbol (params: relative_path, name_path, new_code); create — create a new file (params: relative_path, content); delete — delete a file after prior read validation (params: file or relative_path); diff_apply — apply unified diff (params: diff); format — auto-format file (params: file); preview — preview edit diff (params: file, old_string, new_string). Legacy names: flamel_replace, replace_content, multi_replace, replace_symbol_body, insert_before_symbol, insert_after_symbol, create_text_file, diff_apply, format_file, preview_edit.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["replace", "replace_content", "multi_replace", "replace_symbol", "insert_before", "insert_after", "create", "delete", "diff_apply", "format", "preview"], "description": "Action (default: replace)"},
                    "file": {"type": "string", "description": "File path (for replace/delete/format/preview)"},
                    "relative_path": {"type": "string", "description": "Path (for replace_content/replace_symbol/insert_*/create/delete)"},
                    "old_string": {"type": "string", "description": "Exact string to find (for replace/preview)"},
                    "new_string": {"type": "string", "description": "Replacement string (for replace/preview)"},
                    "needle": {"type": "string", "description": "Search pattern (for replace_content)"},
                    "repl": {"type": "string", "description": "Replacement (for replace_content)"},
                    "mode": {"type": "string", "enum": ["text", "regex"], "description": "Match mode for replace_content (default: text)"},
                    "name_path": {"type": "string", "description": "Symbol path like 'fn_name' or 'Class/method' (for replace_symbol/insert_*)"},
                    "new_body": {"type": "string", "description": "Full replacement including signature (for replace_symbol)"},
                    "new_code": {"type": "string", "description": "Code to insert (for insert_before/insert_after)"},
                    "content": {"type": "string", "description": "File content (for create)"},
                    "edits": {"type": "array", "description": "Array of {file, old_string, new_string} (for multi_replace)", "items": {"type": "object", "properties": {"file": {"type": "string"}, "old_string": {"type": "string"}, "new_string": {"type": "string"}}, "required": ["file", "old_string", "new_string"]}},
                    "diff": {"type": "string", "description": "Unified diff string (for diff_apply)"}
                }
            }
        },
        // (get_symbols_overview, find_symbol, find_referencing_symbols, replace_content schemas consolidated into code_nav/file_edit above)
        // (flamel_replace schema consolidated into file_edit above)
        {
            "name": "AskUserQuestion",
            "description": "Pause execution and ask the user one focused question. The active provider blocks until the user answers or cancels. Use this tool for a consequential decision, ambiguity, risk, or approach choice that available tools cannot resolve; never ask inline in chat. Call it once per question so each answer can inform the next. When choices fit, provide two or three mutually exclusive options, put the recommended option first, and state each option's impact briefly.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {"type": "string", "description": "The question to ask the user. Be specific and concise."},
                    "choices": {
                        "type": "array",
                        "items": {"type": "string"},
                        "minItems": 2,
                        "maxItems": 3,
                        "description": "Optional two or three mutually exclusive choices. Put the recommended choice first and include a short impact. Omit for free-text only."
                    },
                    "allowFreeText": {"type": "boolean", "description": "Whether to allow the user to type a custom answer in addition to choices. Defaults to true."}
                },
                "required": ["question"]
            }
        },
        {
            "name": "file_read",
            "description": "Read, find, and inspect files. Prefer one batch call when multiple known files are needed. A successful read result is authoritative: do not immediately read the same unchanged file again unless the result was partial, reported failure, or a later operation changed it. Actions: read (default) — read file with optional line range/tail (params: file, start_line?, end_line?, tail?); batch — read multiple files in one call (params: files[]); summary — file overview with symbols/imports (params: file); glob — find files by glob pattern (params: pattern, path?); find — find files by pattern (params: pattern, relative_path?); list_dir — list directory contents (params: relative_path). Legacy names: flamel_read, read_file, batch_read, file_summary, flamel_glob, find_file, list_dir.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["read", "batch", "summary", "glob", "find", "list_dir"], "description": "Action (default: read)"},
                    "file": {"type": "string", "description": "File path (for read/summary)"},
                    "relative_path": {"type": "string", "description": "Path (for find/list_dir, or alias for file)"},
                    "start_line": {"type": "integer", "description": "First line, 1-based (for read)"},
                    "end_line": {"type": "integer", "description": "Last line, inclusive (for read)"},
                    "tail": {"type": "integer", "description": "Read last N lines (for read)"},
                    "files": {"type": "array", "description": "Array of paths or {path, start_line?, end_line?} (for batch)", "items": {}},
                    "pattern": {"type": "string", "description": "Glob pattern (for glob/find)"},
                    "path": {"type": "string", "description": "Subdirectory to search (for glob)"}
                }
            }
        },
        // (multi_replace, diff_apply, get_imports schemas consolidated into file_edit/code_nav above)
        // ── Code quality ──────────────────────────────────────────────────
        {
            "name": "code_quality",
            "description": "Build, test, and code analysis. Actions: build (default) — run build/typecheck, return only errors (params: command?, cwd?); test — run tests, return only failures (params: command?, filter?, cwd?); explain — extract file:line from error and show source context (params: error, context_lines?); issues — scan for TODOs/debug prints/unwrap (params: path?). Legacy: build_errors, run_tests, explain_error, find_code_issues.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["build", "test", "explain", "issues"], "description": "Action (default: build)"},
                    "command": {"type": "string", "description": "Override auto-detected command (for build/test)"},
                    "cwd": {"type": "string", "description": "Working directory (for build/test)"},
                    "filter": {"type": "string", "description": "Test name prefix filter (for test)"},
                    "error": {"type": "string", "description": "Error message to parse (for explain)"},
                    "context_lines": {"type": "integer", "description": "Source context lines (for explain, default: 10)"},
                    "path": {"type": "string", "description": "Path filter (for issues)"}
                }
            }
        },
        // (format_file, preview_edit, gather_context schemas consolidated into file_edit/code_nav above)
        {
            "name": "get_recent_changes",
            "description": "Return the last N commits with their changed files and stats. Use at the start of a session to understand what's changed recently. Optionally filter to a specific path/directory. Much faster than reading git log manually.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "Number of commits to return (default 10)"},
                    "path": {"type": "string", "description": "Filter to commits that touched this path/file (optional)"}
                }
            }
        },
        // (explain_error, find_code_issues consolidated into code_quality above)
        // (dependency_graph, checkpoint, rollback consolidated into code_nav/file_history above)

        {
            "name": "media",
            "description": "Fetch images and 3D models. Actions: fetch_image (default) — download image from URL, return as base64 (params: url); fetch_model — load 3D model (GLTF/GLB/OBJ/STL) from URL or path, return metadata + preview (params: url?, path?); create_model — generate 3D model from OpenSCAD code (params: scad, output_format?, output_path?). Legacy: fetch_image, fetch_model3d, create_model3d.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["fetch_image", "fetch_model", "create_model"], "description": "Action (default: fetch_image)"},
                    "url": {"type": "string", "description": "URL (for fetch_image/fetch_model)"},
                    "path": {"type": "string", "description": "Local file path (for fetch_model)"},
                    "scad": {"type": "string", "description": "OpenSCAD source code (for create_model)"},
                    "output_format": {"type": "string", "description": "Output format: stl, off, amf, 3mf (for create_model)"},
                    "output_path": {"type": "string", "description": "Where to save (for create_model)"}
                }
            }
        },
        {
            "name": "AgentBatch",
            "description": "Spawn multiple sub-agents in parallel, wait/poll for all to complete while streaming live progress events, then return all results in a single response for cross-checking. Children are not read-only by default. Markdown-backed personas are materialized as full `.md` files for each child to read before work starts. Implementation personas can edit files, run shell/build/test checks, use browser automation, and use the typed git tool when their delegated task calls for it. Use Explore for read-only research and Plan for planning-only work. Use AgentBatch instead of multiple sequential Agent calls when:\n- Tasks are independent and need genuinely different expertise\n- You need to review results holistically across distinct workstreams\n- Speed matters (parallel > serial)\n\nChoose each agent's subagent_type from the task's real domain; do not default to a backend/frontend/test trio unless those are actually the three best expertises. Do not continue inline work while this tool is running; it blocks until every child finishes, fails, times out, or is skipped. If the host reports a tool-layer timeout, use AgentBatchStatus to check the latest running batch instead of falling back to direct inline work. Returns structured output with per-agent results, file conflict warnings, and your review_prompt appended for cross-checking.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "description": {"type": "string", "description": "High-level description of what this batch accomplishes (shown in UI header)"},
                    "agents": {
                        "type": "array",
                        "description": "Array of agent specs to run in parallel",
                        "items": {
                            "type": "object",
                            "properties": {
                                "description": {"type": "string", "description": "Short label for this agent (shown in UI pill)"},
                                "prompt": {"type": "string", "description": "Self-contained task for this agent"},
                                "subagent_type": {"type": "string", "description": persona_field_description.clone()},
                                "model": {"type": "string", "description": "Optional compatible child-model override. Maximum-reasoning parents may choose a lower-cost model for bounded discovery/review; implementation delegation still requires explicit user or approved-Plan authorization."}
                            },
                            "required": ["description", "prompt", "subagent_type"]
                        },
                        "minItems": 1,
                        "maxItems": 10
                    },
                    "review_prompt": {"type": "string", "description": "Appended after all results — tells you what to cross-check (e.g. 'Verify API contracts match between frontend and backend')"},
                    "failure_strategy": {"type": "string", "enum": ["continue", "fail_fast"], "description": "continue (default): run all agents regardless of failures. fail_fast: abort remaining on first failure."},
                    "timeout_secs": {"type": "integer", "description": "Idle/hung watchdog seconds for each agent. Defaults to 600s. There is no hard wall-clock batch cap; active agents can keep running and the batch emits progress events while waiting. Incomplete agents return timeout results only after their idle watchdog fires or the worker channel closes."}
                },
                "required": ["description", "agents"]
            }
        },
        {
            "name": "AgentBatchStatus",
            "description": "Return the durable AgentBatch receipt for this session and parent agent, including after an MCP process restart. Use this after a tool-layer timeout or interrupted AgentBatch result to determine whether child agents are still running. If batch_id is omitted, returns the latest running batch, or the latest recorded batch if none are running. Set timeout_ms to wait inside this tool call until progress occurs, the batch finishes, or the timeout elapses; this avoids sending repeated tool calls while a batch is still running. Poll this tool until running=0 or status is unknown/not_found. Unknown or not_found means completion is UNCONFIRMED, never successful: inspect saved child outputs and workspace changes before deciding whether any work needs retrying. Do not automatically spawn duplicate agents. Each completed agent includes a capped output field; treat those outputs as the recovered AgentBatch result and do not redo the delegated work inline.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "batch_id": {"type": "string", "description": "Optional AgentBatch id. Omit to inspect the latest running batch."},
                    "timeout_ms": {"type": "integer", "description": "Optional long-poll timeout in milliseconds. Defaults to 0 for an immediate snapshot; values above 3600000 are clamped to one hour."}
                }
            }
        },
    ]);
    if let Some(catalog) = tools.as_array_mut() {
        catalog.push(json!({
            "name": "ponytail_instructions",
            "description": "Return Ponytail's minimal, shortest-working-solution coding instructions. Full mode is the default; lite and ultra adjust the intensity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["lite", "full", "ultra"],
                        "description": "Ponytail intensity (default: full)"
                    }
                },
                "additionalProperties": false
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "mode": {"type": "string", "enum": ["lite", "full", "ultra"]},
                    "instructions": {"type": "string"}
                },
                "required": ["mode", "instructions"],
                "additionalProperties": false
            }
        }));
        if let Some(media) = catalog
            .iter_mut()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some("media"))
        {
            media["name"] = json!("assets");
            media["description"] = json!(
                "Inspect and transform existing image and 3D assets. Native provider image-generation skills remain the preferred way to create new raster images. Actions: fetch_image, fetch_model, create_model."
            );
        }
        catalog.push(json!({
            "name": "decision_checkpoint",
            "description": "Record a concise engineering decision when work is ambiguous, risky, or has material trade-offs. Do not use for routine reads or mechanical edits. Return constraints, evidence, the selected decision, risks, and the immediate next action without exposing private chain-of-thought.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {"type": "string"},
                    "constraints": {"type": "array", "items": {"type": "string"}},
                    "evidence": {"type": "array", "items": {"type": "string"}},
                    "decision": {"type": "string"},
                    "risks": {"type": "array", "items": {"type": "string"}},
                    "next_action": {"type": "string"}
                },
                "required": ["question", "constraints", "evidence", "decision", "risks", "next_action"]
            }
        }));
        catalog.extend(flamel::mcp_gateway::tools());
    }
    apply_tool_capability_metadata(&mut tools);
    tools
}

pub(super) fn tools_list_for_profile(profile: &McpProfile) -> Value {
    let mut tools = tools_list();
    let Some(catalog) = tools.as_array_mut() else {
        return json!([]);
    };
    catalog.retain(|tool| {
        tool.get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| profile.allows_tool(name))
    });
    tools
}
