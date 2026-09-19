use rusqlite::Connection;

pub fn run_migrations(conn: &Connection) {
    super::agent_tracking::migrate(conn).expect("Failed to initialize agent receipts");
    crate::git_operations::workspaces::migrate(conn)
        .expect("Failed to initialize session workspaces");
    super::chat_inputs::migrate(conn).expect("Failed to initialize chat input receipts");
    conn.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            claude_session_id TEXT,
            project_path TEXT NOT NULL,
            title TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            message_count INTEGER DEFAULT 0,
            pinned INTEGER DEFAULT 0,
            model TEXT,
            input_tokens INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            provider TEXT DEFAULT 'claude',
            continuity_status TEXT DEFAULT 'idle',
            recovery_strategy TEXT,
            active_request_id TEXT,
            last_checkpoint_at TEXT,
            last_error TEXT,
            session_owner_kind TEXT,
            orchestration_id TEXT,
            orchestration_run_id TEXT,
            orchestration_role TEXT,
            parent_session_id TEXT,
            active_plan_id TEXT,
            requested_model TEXT DEFAULT 'auto',
            requested_reasoning_effort TEXT DEFAULT 'auto',
            resolved_reasoning_effort TEXT,
            routing_reason TEXT
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS sessions_fts USING fts5(
            id UNINDEXED,
            title,
            content='sessions',
            content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS sessions_fts_insert AFTER INSERT ON sessions BEGIN
            INSERT INTO sessions_fts(rowid, id, title) VALUES (new.rowid, new.id, new.title);
        END;

        CREATE TRIGGER IF NOT EXISTS sessions_fts_update AFTER UPDATE ON sessions BEGIN
            INSERT INTO sessions_fts(sessions_fts, rowid, id, title) VALUES ('delete', old.rowid, old.id, old.title);
            INSERT INTO sessions_fts(rowid, id, title) VALUES (new.rowid, new.id, new.title);
        END;

        CREATE TRIGGER IF NOT EXISTS sessions_fts_delete AFTER DELETE ON sessions BEGIN
            INSERT INTO sessions_fts(sessions_fts, rowid, id, title) VALUES ('delete', old.rowid, old.id, old.title);
        END;


        CREATE TABLE IF NOT EXISTS generated_artifacts (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            tool_call_id TEXT,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            media_type TEXT NOT NULL,
            byte_size INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            storage_path TEXT NOT NULL UNIQUE,
            metadata TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_generated_artifacts_user_session
            ON generated_artifacts(user_id, session_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_generated_artifacts_tool_call
            ON generated_artifacts(user_id, tool_call_id);

        CREATE TABLE IF NOT EXISTS prompts (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL,
            use_count INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS mcp_configs (
            id TEXT PRIMARY KEY,
            project_path TEXT,
            name TEXT NOT NULL,
            command TEXT NOT NULL,
            args TEXT,
            enabled INTEGER DEFAULT 1,
            is_builtin INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS plans (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            items TEXT NOT NULL,
            session_id TEXT,
            project_path TEXT,
            execution_status TEXT NOT NULL DEFAULT 'ready',
            approval_status TEXT NOT NULL DEFAULT 'approved',
            active_item_id TEXT,
            approval_requested_at TEXT,
            approved_at TEXT,
            execution_started_at TEXT,
            execution_updated_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS plan_items (
            id TEXT PRIMARY KEY,
            plan_id TEXT NOT NULL,
            text TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'todo',
            notes TEXT,
            reasoning_floor TEXT NOT NULL DEFAULT 'medium',
            position INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (plan_id) REFERENCES plans(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS plan_versions (
            id TEXT PRIMARY KEY,
            plan_id TEXT NOT NULL,
            snapshot TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (plan_id) REFERENCES plans(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS plan_artifacts (
            id TEXT PRIMARY KEY,
            plan_id TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL DEFAULT 'plan',
            schema_version INTEGER NOT NULL DEFAULT 1,
            session_id TEXT,
            project_path TEXT,
            user_id TEXT,
            metadata TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (plan_id) REFERENCES plans(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_plan_artifacts_user_session ON plan_artifacts(user_id, session_id);
        CREATE INDEX IF NOT EXISTS idx_plan_artifacts_user_project ON plan_artifacts(user_id, project_path);

        CREATE TABLE IF NOT EXISTS session_blocks (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id)
        );

        CREATE TABLE IF NOT EXISTS session_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            role TEXT NOT NULL,
            message TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_session_messages_session_id ON session_messages(session_id);

        CREATE TABLE IF NOT EXISTS session_history_revisions (
            session_id TEXT PRIMARY KEY,
            revision INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        INSERT OR IGNORE INTO session_history_revisions (session_id, revision)
        SELECT id, 0 FROM sessions;

        CREATE TABLE IF NOT EXISTS user_settings (
            user_id TEXT PRIMARY KEY,
            git_name TEXT,
            git_email TEXT,
            git_signing_key TEXT,
            preferred_provider TEXT DEFAULT 'codex',
            cross_provider_fallback INTEGER DEFAULT 1,
            codex_planning_reasoning_effort TEXT DEFAULT 'xhigh',
            claude_planning_reasoning_effort TEXT DEFAULT 'xhigh',
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
    ",
    )
    .unwrap();

    // Migrate plans table: add new columns if not present
    let _ = conn.execute("ALTER TABLE plans ADD COLUMN description TEXT", []);
    let _ = conn.execute("ALTER TABLE plans ADD COLUMN share_token TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE plans ADD COLUMN finished INTEGER DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE plans ADD COLUMN archived INTEGER DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE plans ADD COLUMN execution_status TEXT NOT NULL DEFAULT 'ready'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE plans ADD COLUMN approval_status TEXT NOT NULL DEFAULT 'approved'",
        [],
    );
    let _ = conn.execute("ALTER TABLE plans ADD COLUMN active_item_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE plans ADD COLUMN approval_requested_at TEXT",
        [],
    );
    let _ = conn.execute("ALTER TABLE plans ADD COLUMN approved_at TEXT", []);
    let _ = conn.execute("ALTER TABLE plans ADD COLUMN execution_started_at TEXT", []);
    let _ = conn.execute("ALTER TABLE plans ADD COLUMN execution_updated_at TEXT", []);

    // claude_credentials: per-user OAuth tokens for claude.ai (Max subscription)
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS claude_credentials (
            user_id TEXT PRIMARY KEY,
            access_token TEXT NOT NULL,
            refresh_token TEXT,
            expires_at INTEGER NOT NULL,
            scopes TEXT NOT NULL DEFAULT 'openid user:inference user:profile',
            updated_at TEXT NOT NULL
        );
    ",
    )
    .unwrap();

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS git_accounts (
            id         TEXT PRIMARY KEY,
            type       TEXT NOT NULL,
            label      TEXT NOT NULL,
            base_url   TEXT NOT NULL,
            token      TEXT NOT NULL,
            username   TEXT,
            avatar_url TEXT,
            created_at TEXT NOT NULL
        );
    ",
    )
    .unwrap();

    // Migrations
    let _ = conn.execute("ALTER TABLE git_accounts ADD COLUMN username TEXT", []);
    let _ = conn.execute("ALTER TABLE git_accounts ADD COLUMN avatar_url TEXT", []);

    // Ownership is assigned only by the explicit, recorded legacy claim flow.
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN user_id TEXT", []);

    // Git account user isolation: add user_id column and backfill existing accounts to admin user
    let _ = conn.execute("ALTER TABLE git_accounts ADD COLUMN user_id TEXT", []);

    // Agent config user isolation
    let _ = conn.execute("ALTER TABLE agent_configs ADD COLUMN user_id TEXT", []);
    let _ = conn.execute(
        "UPDATE agent_configs SET user_id = (SELECT ga.user_id FROM git_accounts ga WHERE ga.id = agent_configs.git_account_id) WHERE user_id IS NULL",
        [],
    );

    // Remove built-in stubs
    let _ = conn.execute("DELETE FROM mcp_configs WHERE is_builtin = 1", []);

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS todos (
            id          TEXT PRIMARY KEY,
            project_path TEXT NOT NULL,
            text        TEXT NOT NULL,
            done        INTEGER NOT NULL DEFAULT 0,
            position    INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_todos_project_path ON todos(project_path);

        CREATE TABLE IF NOT EXISTS agent_configs (
            id              TEXT PRIMARY KEY,
            git_account_id  TEXT NOT NULL,
            repo_owner      TEXT NOT NULL,
            repo_name       TEXT NOT NULL,
            trigger_event   TEXT NOT NULL DEFAULT 'issues',
            trigger_label   TEXT,
            project_path    TEXT NOT NULL,
            model           TEXT NOT NULL DEFAULT 'claude-opus-4-7',
            permission_mode TEXT NOT NULL DEFAULT 'default',
            auto_pr         INTEGER NOT NULL DEFAULT 1,
            require_approval INTEGER NOT NULL DEFAULT 1,
            enabled         INTEGER NOT NULL DEFAULT 1,
            created_at      TEXT NOT NULL,
            FOREIGN KEY (git_account_id) REFERENCES git_accounts(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS agent_runs (
            id              TEXT PRIMARY KEY,
            agent_config_id TEXT NOT NULL,
            session_id      TEXT,
            status          TEXT NOT NULL DEFAULT 'pending',
            trigger_payload TEXT NOT NULL DEFAULT '{}',
            pr_url          TEXT,
            error           TEXT,
            created_at      TEXT NOT NULL,
            updated_at      TEXT NOT NULL,
            FOREIGN KEY (agent_config_id) REFERENCES agent_configs(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_agent_runs_config_id ON agent_runs(agent_config_id);

        CREATE TABLE IF NOT EXISTS compact_summaries (
            id          TEXT PRIMARY KEY,
            session_id  TEXT NOT NULL,
            summary     TEXT NOT NULL,
            tokens_before INTEGER NOT NULL DEFAULT 0,
            model       TEXT NOT NULL,
            created_at  TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );
        ",
    )
    .unwrap();

    // Plans user isolation
    let _ = conn.execute("ALTER TABLE plans ADD COLUMN user_id TEXT", []);
    let _ = conn.execute(
        "UPDATE plans
         SET active_item_id = (
             SELECT id FROM plan_items
             WHERE plan_items.plan_id = plans.id AND status = 'in_progress'
             ORDER BY position ASC LIMIT 1
         )
         WHERE active_item_id IS NULL",
        [],
    );
    let _ = conn.execute(
        "UPDATE plans
         SET execution_status = CASE
             WHEN COALESCE(finished, 0) = 1 THEN 'completed'
             WHEN EXISTS (SELECT 1 FROM plan_items WHERE plan_items.plan_id = plans.id AND status = 'blocked') THEN 'blocked'
             WHEN EXISTS (SELECT 1 FROM plan_items WHERE plan_items.plan_id = plans.id AND status = 'in_progress') THEN 'running'
             ELSE COALESCE(NULLIF(execution_status, ''), 'ready')
         END
         WHERE execution_status IS NULL OR execution_status = '' OR execution_status = 'ready'",
        [],
    );
    let _ = conn.execute(
        "UPDATE plans SET approval_status = 'approved' WHERE approval_status IS NULL OR approval_status = ''",
        [],
    );
    let _ = conn.execute(
        "UPDATE plans SET approval_requested_at = created_at WHERE approval_requested_at IS NULL",
        [],
    );
    let _ = conn.execute(
        "UPDATE plans SET approved_at = created_at WHERE approval_status = 'approved' AND approved_at IS NULL",
        [],
    );
    let _ = conn.execute(
        "UPDATE plans SET execution_updated_at = updated_at WHERE execution_updated_at IS NULL",
        [],
    );
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS plan_artifacts (
            id TEXT PRIMARY KEY,
            plan_id TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL DEFAULT 'plan',
            schema_version INTEGER NOT NULL DEFAULT 1,
            session_id TEXT,
            project_path TEXT,
            user_id TEXT,
            metadata TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (plan_id) REFERENCES plans(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_plan_artifacts_user_session ON plan_artifacts(user_id, session_id);
        CREATE INDEX IF NOT EXISTS idx_plan_artifacts_user_project ON plan_artifacts(user_id, project_path);
        INSERT OR IGNORE INTO plan_artifacts (
            id, plan_id, kind, schema_version, session_id, project_path, user_id, metadata, created_at, updated_at
        )
        SELECT 'plan-artifact-' || id, id, 'plan', 1, session_id, project_path, user_id, '{}', created_at, updated_at
        FROM plans;
        ",
    )
    .ok();

    // Todos user isolation
    let _ = conn.execute("ALTER TABLE todos ADD COLUMN user_id TEXT", []);

    // Prompts user isolation
    let _ = conn.execute("ALTER TABLE prompts ADD COLUMN user_id TEXT", []);

    // MCP configs user isolation
    let _ = conn.execute("ALTER TABLE mcp_configs ADD COLUMN user_id TEXT", []);

    // Issue lifecycle
    let _ = conn.execute(
        "ALTER TABLE agent_configs ADD COLUMN auto_close INTEGER NOT NULL DEFAULT 1",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE agent_configs ADD COLUMN comment_on_start INTEGER NOT NULL DEFAULT 1",
        [],
    );

    // Issue poller tracking
    let _ = conn.execute(
        "ALTER TABLE agent_configs ADD COLUMN last_polled_at TEXT",
        [],
    );
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS seen_agent_issues (
            agent_config_id TEXT NOT NULL,
            issue_number    INTEGER NOT NULL,
            created_at      TEXT NOT NULL,
            PRIMARY KEY (agent_config_id, issue_number),
            FOREIGN KEY (agent_config_id) REFERENCES agent_configs(id) ON DELETE CASCADE
        );",
    )
    .unwrap();

    // Agent run timestamps and priority
    let _ = conn.execute("ALTER TABLE agent_runs ADD COLUMN started_at TEXT", []);
    let _ = conn.execute("ALTER TABLE agent_runs ADD COLUMN finished_at TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE agent_runs ADD COLUMN priority INTEGER DEFAULT 0",
        [],
    );
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_output_lines (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id  TEXT NOT NULL,
            line    TEXT NOT NULL,
            ts      TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_agent_output_run_id ON agent_output_lines(run_id);",
    )
    .unwrap();

    // Plan items: add section column for phase/section grouping
    let _ = conn.execute("ALTER TABLE plan_items ADD COLUMN section TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE plan_items ADD COLUMN reasoning_floor TEXT NOT NULL DEFAULT 'medium'",
        [],
    );

    super::ownership::migrate(conn).expect("Failed to initialize ownership migration ledger");

    // Migration: drop FK constraints from user_settings and claude_credentials
    // so they work with both local users and flamel.dev SSO users.
    // SQLite can't drop constraints, so we recreate the tables.
    let has_fk: bool = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='user_settings'",
            [],
            |r| r.get::<_, String>(0),
        )
        .map(|sql| sql.contains("FOREIGN KEY"))
        .unwrap_or(false);
    if has_fk {
        conn.execute_batch(
            "
            CREATE TABLE user_settings_new (
                user_id TEXT PRIMARY KEY,
                git_name TEXT,
                git_email TEXT,
                git_signing_key TEXT,
                updated_at TEXT NOT NULL
            );
            INSERT INTO user_settings_new SELECT * FROM user_settings;
            DROP TABLE user_settings;
            ALTER TABLE user_settings_new RENAME TO user_settings;

            CREATE TABLE claude_credentials_new (
                user_id TEXT PRIMARY KEY,
                access_token TEXT NOT NULL,
                refresh_token TEXT,
                expires_at INTEGER NOT NULL,
                scopes TEXT NOT NULL DEFAULT 'openid user:inference user:profile',
                updated_at TEXT NOT NULL
            );
            INSERT INTO claude_credentials_new SELECT * FROM claude_credentials;
            DROP TABLE claude_credentials;
            ALTER TABLE claude_credentials_new RENAME TO claude_credentials;
            ",
        )
        .ok();
    }

    // Retain retired local identity tables as migration evidence. No active auth
    // path reads them; deleting them would lose the mapping needed to recover
    // ambiguous historical ownership safely.

    // ── Runners ──────────────────────────────────────────────────────────

    // Runners: add ssh_password and ssh_key columns for additional auth methods

    // Sessions: add runner_id for remote runner dispatch
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN runner_id TEXT", []);

    // ── Project Runner Pools ─────────────────────────────────────────────
    // Maps projects to their assigned runners. A project can have 0, 1, or N runners.
    // mode: 'manual' (user picks), 'auto' (server picks from pool), or absent (local only).

    // Project-level runner mode: 'local' (default), 'manual', 'auto'
    // Stored in app_settings as 'project_runner_mode:<project_path>' = mode

    // Repository Jobs supersede the legacy scheduler; its dispatcher is gone.
    // Keep historical schedules/results for recovery. An upgrade is not consent
    // to erase records; a separately approved archival migration can remove them.

    let _ = conn.execute(
        "ALTER TABLE sessions ADD COLUMN session_owner_kind TEXT",
        [],
    );
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN orchestration_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE sessions ADD COLUMN orchestration_run_id TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE sessions ADD COLUMN orchestration_role TEXT",
        [],
    );

    // Provider-agnostic OAuth credentials (Codex, etc.)
    // Claude credentials stay in claude_credentials for backward compat.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS provider_credentials (
            user_id    TEXT NOT NULL,
            provider   TEXT NOT NULL,
            access_token TEXT NOT NULL,
            refresh_token TEXT,
            id_token   TEXT,
            expires_at INTEGER NOT NULL,
            scopes     TEXT NOT NULL DEFAULT '',
            extra_json TEXT,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (user_id, provider)
        );",
    )
    .ok();

    let _ = conn.execute(
        "ALTER TABLE sessions ADD COLUMN provider TEXT DEFAULT 'claude'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE sessions ADD COLUMN continuity_status TEXT DEFAULT 'idle'",
        [],
    );
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN recovery_strategy TEXT", []);
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN active_request_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE sessions ADD COLUMN last_checkpoint_at TEXT",
        [],
    );
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN last_error TEXT", []);
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN parent_session_id TEXT", []);
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN active_plan_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE sessions ADD COLUMN requested_model TEXT DEFAULT 'auto'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE sessions ADD COLUMN requested_reasoning_effort TEXT DEFAULT 'auto'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE sessions ADD COLUMN resolved_reasoning_effort TEXT",
        [],
    );
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN routing_reason TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE user_settings ADD COLUMN preferred_provider TEXT DEFAULT 'codex'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE user_settings ADD COLUMN cross_provider_fallback INTEGER DEFAULT 1",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE user_settings ADD COLUMN codex_planning_reasoning_effort TEXT DEFAULT 'xhigh'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE user_settings ADD COLUMN claude_planning_reasoning_effort TEXT DEFAULT 'xhigh'",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_active_plan ON sessions(active_plan_id)",
        [],
    );

    // Capability Hub. Legacy mcp_configs remain as a rollback-compatible source,
    // while new connections can represent stdio and remote MCP transports.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS capability_catalog (
            id TEXT PRIMARY KEY,
            slug TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            category TEXT NOT NULL DEFAULT 'developer',
            source TEXT NOT NULL DEFAULT 'custom',
            publisher TEXT NOT NULL DEFAULT '',
            version TEXT NOT NULL DEFAULT '',
            transport TEXT NOT NULL DEFAULT 'stdio',
            command TEXT,
            args_json TEXT NOT NULL DEFAULT '[]',
            remote_url TEXT,
            auth_kind TEXT NOT NULL DEFAULT 'none',
            permissions_json TEXT NOT NULL DEFAULT '[]',
            toolsets_json TEXT NOT NULL DEFAULT '[]',
            admin_approved INTEGER NOT NULL DEFAULT 0,
            builtin INTEGER NOT NULL DEFAULT 0,
            metadata_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS capability_connections (
            id TEXT PRIMARY KEY,
            capability_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            project_path TEXT,
            name TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            config_json TEXT NOT NULL DEFAULT '{}',
            credential_ref TEXT,
            status TEXT NOT NULL DEFAULT 'configured',
            last_error TEXT,
            health_checked_at TEXT,
            last_success_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (capability_id) REFERENCES capability_catalog(id) ON DELETE CASCADE
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_capability_connection_scope
            ON capability_connections(capability_id, user_id, IFNULL(project_path, ''));
        CREATE INDEX IF NOT EXISTS idx_capability_connections_user
            ON capability_connections(user_id, project_path, enabled);
        CREATE TABLE IF NOT EXISTS capability_policies (
            id TEXT PRIMARY KEY,
            capability_id TEXT,
            scope_kind TEXT NOT NULL DEFAULT 'server',
            scope_id TEXT,
            decision TEXT NOT NULL DEFAULT 'allow',
            rules_json TEXT NOT NULL DEFAULT '{}',
            created_by TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (capability_id) REFERENCES capability_catalog(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS capability_invocations (
            id TEXT PRIMARY KEY,
            capability_id TEXT NOT NULL,
            connection_id TEXT,
            user_id TEXT NOT NULL,
            session_id TEXT,
            tool_name TEXT NOT NULL,
            status TEXT NOT NULL,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            error_code TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY (capability_id) REFERENCES capability_catalog(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_capability_invocations_lookup
            ON capability_invocations(user_id, capability_id, created_at);

        CREATE TABLE IF NOT EXISTS capability_credentials (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            capability_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            encrypted_value TEXT NOT NULL,
            metadata_json TEXT NOT NULL DEFAULT '{}',
            expires_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (capability_id) REFERENCES capability_catalog(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_capability_credentials_owner
            ON capability_credentials(user_id, capability_id);

        CREATE TABLE IF NOT EXISTS capability_tools (
            capability_id TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            input_schema_json TEXT NOT NULL DEFAULT '{}',
            output_schema_json TEXT,
            annotations_json TEXT NOT NULL DEFAULT '{}',
            enabled INTEGER NOT NULL DEFAULT 1,
            discovered_at TEXT NOT NULL,
            PRIMARY KEY (capability_id, name),
            FOREIGN KEY (capability_id) REFERENCES capability_catalog(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS capability_surfaces (
            capability_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            definition_json TEXT NOT NULL DEFAULT '{}',
            discovered_at TEXT NOT NULL,
            PRIMARY KEY (capability_id, kind, name),
            FOREIGN KEY (capability_id) REFERENCES capability_catalog(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS capability_tasks (
            id TEXT PRIMARY KEY,
            capability_id TEXT NOT NULL,
            connection_id TEXT,
            user_id TEXT NOT NULL,
            session_id TEXT,
            tool_name TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'queued',
            input_json TEXT NOT NULL DEFAULT '{}',
            output_json TEXT,
            error TEXT,
            progress REAL NOT NULL DEFAULT 0,
            cancel_requested INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (capability_id) REFERENCES capability_catalog(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_capability_tasks_owner
            ON capability_tasks(user_id, status, updated_at);

        CREATE TABLE IF NOT EXISTS capability_audit_events (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            capability_id TEXT,
            connection_id TEXT,
            session_id TEXT,
            event_type TEXT NOT NULL,
            decision TEXT NOT NULL DEFAULT 'allow',
            tool_name TEXT,
            summary TEXT NOT NULL DEFAULT '',
            metadata_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_capability_audit_lookup
            ON capability_audit_events(user_id, created_at);

        CREATE TABLE IF NOT EXISTS capability_health (
            capability_id TEXT NOT NULL,
            connection_id TEXT NOT NULL,
            process_id INTEGER,
            tool_count INTEGER NOT NULL DEFAULT 0,
            consecutive_failures INTEGER NOT NULL DEFAULT 0,
            restart_count INTEGER NOT NULL DEFAULT 0,
            circuit_state TEXT NOT NULL DEFAULT 'closed',
            latency_ms INTEGER,
            initialized_at TEXT,
            last_success_at TEXT,
            last_failure_at TEXT,
            last_error TEXT,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (capability_id, connection_id)
        );

        CREATE TABLE IF NOT EXISTS capability_marketplace (
            id TEXT PRIMARY KEY,
            slug TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            description TEXT NOT NULL,
            category TEXT NOT NULL,
            publisher TEXT NOT NULL,
            official INTEGER NOT NULL DEFAULT 0,
            homepage_url TEXT,
            manifest_url TEXT,
            transport TEXT NOT NULL,
            remote_url TEXT,
            auth_kind TEXT NOT NULL DEFAULT 'oauth2',
            permissions_json TEXT NOT NULL DEFAULT '[]',
            checksum TEXT,
            status TEXT NOT NULL DEFAULT 'available',
            metadata_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS capability_pack_versions (
            pack_id TEXT NOT NULL,
            version TEXT NOT NULL,
            manifest_json TEXT NOT NULL,
            manifest_sha256 TEXT NOT NULL,
            signature_b64 TEXT NOT NULL,
            signer_key_id TEXT NOT NULL,
            verified_at TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (pack_id, version)
        );
        CREATE TABLE IF NOT EXISTS capability_pack_installations (
            pack_id TEXT PRIMARY KEY,
            capability_id TEXT,
            active_version TEXT,
            previous_version TEXT,
            state TEXT NOT NULL DEFAULT 'installed',
            schedule_eligible INTEGER NOT NULL DEFAULT 0,
            schedules_enabled INTEGER NOT NULL DEFAULT 0,
            installed_by TEXT NOT NULL,
            installed_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (capability_id) REFERENCES capability_catalog(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_capability_pack_installations_capability
            ON capability_pack_installations(capability_id, state);

        CREATE TABLE IF NOT EXISTS project_index_files (
            project_path TEXT NOT NULL,
            relative_path TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            language TEXT NOT NULL DEFAULT '',
            size_bytes INTEGER NOT NULL DEFAULT 0,
            symbols_json TEXT NOT NULL DEFAULT '[]',
            imports_json TEXT NOT NULL DEFAULT '[]',
            embedding_json TEXT NOT NULL DEFAULT '[]',
            indexed_at TEXT NOT NULL,
            PRIMARY KEY (project_path, relative_path)
        );
        CREATE INDEX IF NOT EXISTS idx_project_index_language
            ON project_index_files(project_path, language);

        CREATE TABLE IF NOT EXISTS work_items (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            project_path TEXT NOT NULL,
            parent_id TEXT,
            kind TEXT NOT NULL DEFAULT 'task',
            title TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'backlog',
            priority TEXT NOT NULL DEFAULT 'normal',
            assignee_kind TEXT,
            assignee_id TEXT,
            milestone TEXT,
            release_status TEXT,
            plan_id TEXT,
            orchestration_run_id TEXT,
            labels_json TEXT NOT NULL DEFAULT '[]',
            position INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_work_items_project
            ON work_items(user_id, project_path, status, position);

        CREATE TABLE IF NOT EXISTS work_comments (
            id TEXT PRIMARY KEY,
            work_item_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (work_item_id) REFERENCES work_items(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS work_dependencies (
            work_item_id TEXT NOT NULL,
            depends_on_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (work_item_id, depends_on_id),
            FOREIGN KEY (work_item_id) REFERENCES work_items(id) ON DELETE CASCADE,
            FOREIGN KEY (depends_on_id) REFERENCES work_items(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS work_artifacts (
            id TEXT PRIMARY KEY,
            work_item_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            title TEXT NOT NULL,
            path TEXT,
            url TEXT,
            metadata_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            FOREIGN KEY (work_item_id) REFERENCES work_items(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS capability_skills (
            id TEXT PRIMARY KEY,
            slug TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            description TEXT NOT NULL,
            category TEXT NOT NULL,
            instructions TEXT NOT NULL,
            triggers_json TEXT NOT NULL DEFAULT '[]',
            enabled INTEGER NOT NULL DEFAULT 1,
            builtin INTEGER NOT NULL DEFAULT 0,
            pack_id TEXT,
            pack_version TEXT,
            current_version TEXT NOT NULL DEFAULT '1.0.0',
            content_hash TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS capability_skill_versions (
            skill_id TEXT NOT NULL,
            version TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT NOT NULL,
            category TEXT NOT NULL,
            instructions TEXT NOT NULL,
            triggers_json TEXT NOT NULL DEFAULT '[]',
            content_hash TEXT NOT NULL,
            required_capabilities_json TEXT NOT NULL DEFAULT '[]',
            allowed_operations_json TEXT NOT NULL DEFAULT '[]',
            provider_compat_json TEXT NOT NULL DEFAULT '[\"codex\",\"claude\"]',
            scope_kind TEXT NOT NULL DEFAULT 'global',
            owner_user_id TEXT,
            project_path TEXT,
            provenance_json TEXT NOT NULL DEFAULT '{}',
            review_state TEXT NOT NULL DEFAULT 'proposed',
            signed INTEGER NOT NULL DEFAULT 0,
            quarantined INTEGER NOT NULL DEFAULT 0,
            quarantine_reason TEXT,
            reviewed_by TEXT,
            reviewed_at TEXT,
            created_at TEXT NOT NULL,
            PRIMARY KEY (skill_id, version)
        );
        CREATE TABLE IF NOT EXISTS capability_skill_installations (
            id TEXT PRIMARY KEY,
            skill_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            project_path TEXT NOT NULL DEFAULT '',
            active_version TEXT NOT NULL,
            previous_version TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            provider_projection_enabled INTEGER NOT NULL DEFAULT 1,
            snapshot_revision TEXT NOT NULL,
            installed_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE (skill_id, user_id, project_path)
        );
        CREATE INDEX IF NOT EXISTS idx_capability_skill_installations_scope
            ON capability_skill_installations(user_id, project_path, enabled);
        CREATE INDEX IF NOT EXISTS idx_capability_skill_versions_review
            ON capability_skill_versions(review_state, quarantined, signed);

        INSERT OR IGNORE INTO capability_catalog (
            id, slug, title, description, category, source, transport, command,
            args_json, admin_approved, builtin, metadata_json, created_at, updated_at
        )
        SELECT 'legacy-' || id, name, name, 'Imported MCP server', 'developer',
               'legacy', 'stdio', command, COALESCE(args, '[]'), 1, 0,
               json_object('legacyMcpConfigId', id), datetime('now'), datetime('now')
        FROM mcp_configs;

        INSERT OR IGNORE INTO capability_connections (
            id, capability_id, user_id, project_path, name, enabled, config_json,
            status, created_at, updated_at
        )
        SELECT 'legacy-' || id, 'legacy-' || id, COALESCE(user_id, 'local'),
               project_path, name, enabled, '{}',
               CASE WHEN enabled = 1 THEN 'configured' ELSE 'disabled' END,
               datetime('now'), datetime('now')
        FROM mcp_configs;",
    )
    .ok();

    for statement in [
        "ALTER TABLE capability_connections ADD COLUMN restart_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE capability_connections ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE capability_connections ADD COLUMN circuit_state TEXT NOT NULL DEFAULT 'closed'",
        "ALTER TABLE capability_connections ADD COLUMN latency_ms INTEGER",
        "ALTER TABLE capability_connections ADD COLUMN tool_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE capability_skills ADD COLUMN pack_id TEXT",
        "ALTER TABLE capability_skills ADD COLUMN pack_version TEXT",
        "ALTER TABLE capability_skills ADD COLUMN current_version TEXT NOT NULL DEFAULT '1.0.0'",
        "ALTER TABLE capability_skills ADD COLUMN content_hash TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE project_index_files ADD COLUMN embedding_json TEXT NOT NULL DEFAULT '[]'",
    ] {
        let _ = conn.execute(statement, []);
    }

    // Goal Workers use normal provider sessions for durable streaming and audit.
    // Older builds did not classify those sessions, so they appeared as duplicate
    // chats. Backfill from the durable worker marker rather than relying on titles.
    conn.execute_batch(
        "UPDATE sessions
         SET session_owner_kind='goal_worker'
         WHERE session_owner_kind IS NULL
           AND EXISTS (
               SELECT 1 FROM session_messages
               WHERE session_messages.session_id=sessions.id
                 AND session_messages.message LIKE '%\"worker\":true%'
           );
         CREATE INDEX IF NOT EXISTS idx_sessions_owner_kind
             ON sessions(session_owner_kind);",
    )
    .expect("failed to classify internal Goal Worker sessions");

    // Older clients could create a real interactive session before learning its
    // server identity, leaving the durable title empty while the tab showed a
    // local project title. Repair those records from their first user message so
    // every client renders the same canonical identity after reconnecting.
    let untitled_messages = {
        let mut statement = conn
            .prepare(
                "SELECT sessions.id, session_messages.message
                 FROM sessions
                 JOIN session_messages ON session_messages.session_id=sessions.id
                 WHERE TRIM(COALESCE(sessions.title, ''))=''
                   AND COALESCE(sessions.session_owner_kind, 'interactive')='interactive'
                   AND session_messages.role='user'
                 ORDER BY sessions.id, session_messages.id",
            )
            .expect("failed to prepare untitled session migration");
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("failed to query untitled sessions")
            .filter_map(Result::ok)
            .collect::<Vec<_>>()
    };
    let mut titled_sessions = std::collections::HashSet::new();
    for (session_id, message) in untitled_messages {
        if titled_sessions.contains(&session_id) {
            continue;
        }
        let Some(title) = super::session_titles::title_from_persisted_user_message(&message) else {
            continue;
        };
        conn.execute(
            "UPDATE sessions SET title=?2 WHERE id=?1",
            rusqlite::params![&session_id, title],
        )
        .expect("failed to repair untitled session");
        titled_sessions.insert(session_id);
    }

    // The retired orchestration runtime is no longer started, but its reports,
    // artifacts, audit history and templates are user data. Preserve them just
    // like the linked session transcript rather than deleting on every startup.

    // Durable realtime delivery. The legacy chat stream remains available while
    // clients migrate, but reconnectable clients consume this ordered journal.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_events (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL UNIQUE,
            session_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            run_id TEXT,
            kind TEXT NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_session_events_user_sequence
             ON session_events(user_id, sequence);
         CREATE INDEX IF NOT EXISTS idx_session_events_session_sequence
             ON session_events(session_id, sequence);
         CREATE INDEX IF NOT EXISTS idx_session_events_run_kind_sequence
             ON session_events(run_id, kind, sequence);

         CREATE TABLE IF NOT EXISTS chat_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            state TEXT NOT NULL,
            request_json TEXT NOT NULL,
            attempt INTEGER NOT NULL DEFAULT 0,
            lease_owner TEXT,
            lease_expires_at TEXT,
            last_event_sequence INTEGER,
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_chat_runs_session_state
             ON chat_runs(session_id, state, created_at);
         CREATE INDEX IF NOT EXISTS idx_chat_runs_user_state
             ON chat_runs(user_id, state, created_at);

         CREATE TABLE IF NOT EXISTS input_requests (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            run_id TEXT,
            user_id TEXT NOT NULL,
            request_json TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending','answered','cancelled','consumed')),
            answer TEXT,
            delivery_kind TEXT
                CHECK (delivery_kind IS NULL OR delivery_kind IN ('live','recovery')),
            delivery_claim_owner TEXT,
            delivery_claimed_at TEXT,
            delivery_claim_expires_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            answered_at TEXT,
            cancelled_at TEXT,
            consumed_at TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (run_id) REFERENCES chat_runs(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_input_requests_session_status
             ON input_requests(user_id, session_id, status, created_at);
         CREATE INDEX IF NOT EXISTS idx_input_requests_run_status
             ON input_requests(run_id, status, created_at);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_input_requests_one_pending_run
             ON input_requests(run_id) WHERE run_id IS NOT NULL AND status='pending';",
    )
    .expect("failed to create durable realtime storage");

    // Durable context storage. `session_messages` remains the transcript used
    // by the UI; this ledger gives each message one stable source ID for
    // checkpoint retrieval without retaining transient streaming snapshots.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS context_events (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL UNIQUE,
            session_id TEXT NOT NULL,
            run_id TEXT,
            source_kind TEXT NOT NULL,
            source_id TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            metadata TEXT NOT NULL DEFAULT '{}',
            active INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            UNIQUE(session_id, source_kind, source_id, content_hash),
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_context_events_session_sequence
             ON context_events(session_id, sequence);
         CREATE INDEX IF NOT EXISTS idx_context_events_session_active_sequence
             ON context_events(session_id, active, sequence);

         DROP TRIGGER IF EXISTS context_events_fts_insert;
         DROP TRIGGER IF EXISTS context_events_fts_delete;

         CREATE TABLE IF NOT EXISTS context_checkpoints (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            status TEXT NOT NULL,
            summary TEXT NOT NULL,
            state_json TEXT NOT NULL,
            source_start_sequence INTEGER,
            source_end_sequence INTEGER,
            preserved_tail_messages INTEGER NOT NULL DEFAULT 0,
            tokens_before INTEGER NOT NULL DEFAULT 0,
            estimated_tokens_after INTEGER NOT NULL DEFAULT 0,
            model TEXT NOT NULL,
            reasoning_effort TEXT NOT NULL,
            validation_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            activated_at TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_context_checkpoints_session_created
             ON context_checkpoints(session_id, created_at DESC);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_context_checkpoints_one_active
             ON context_checkpoints(session_id) WHERE status='active';

         CREATE TABLE IF NOT EXISTS context_facts (
            id TEXT PRIMARY KEY,
            checkpoint_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            canonical_key TEXT NOT NULL,
            content TEXT NOT NULL,
            value_json TEXT,
            confidence REAL NOT NULL DEFAULT 1.0,
            sensitivity TEXT NOT NULL DEFAULT 'session',
            source_event_ids TEXT NOT NULL DEFAULT '[]',
            last_verified_at TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY (checkpoint_id) REFERENCES context_checkpoints(id) ON DELETE CASCADE,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_context_facts_session_kind
             ON context_facts(session_id, kind);",
    )
    .expect("failed to create lossless context storage");

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS runtime_catalog_state (
            id INTEGER PRIMARY KEY CHECK (id=1),
            channel TEXT NOT NULL DEFAULT 'stable',
            active_revision INTEGER,
            previous_revision INTEGER,
            highest_accepted_revision INTEGER NOT NULL DEFAULT 0,
            active_key_id TEXT,
            etag TEXT,
            trusted_revocations_json TEXT NOT NULL DEFAULT '[]',
            last_attempt_at TEXT,
            last_success_at TEXT,
            last_error TEXT,
            frozen INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
         );
         INSERT OR IGNORE INTO runtime_catalog_state
             (id,channel,highest_accepted_revision,frozen,updated_at)
         VALUES (1,'stable',0,0,datetime('now'));

         CREATE TABLE IF NOT EXISTS runtime_catalog_snapshots (
            revision INTEGER PRIMARY KEY,
            channel TEXT NOT NULL,
            manifest_jws TEXT NOT NULL,
            manifest_json TEXT NOT NULL,
            key_id TEXT NOT NULL,
            verified_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS runtime_catalog_artifacts (
            sha256 TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            slug TEXT NOT NULL,
            version TEXT NOT NULL,
            byte_size INTEGER NOT NULL,
            content BLOB NOT NULL,
            created_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS runtime_catalog_snapshot_artifacts (
            revision INTEGER NOT NULL,
            position INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            PRIMARY KEY (revision,position),
            UNIQUE (revision,sha256),
            FOREIGN KEY (revision) REFERENCES runtime_catalog_snapshots(revision) ON DELETE CASCADE,
            FOREIGN KEY (sha256) REFERENCES runtime_catalog_artifacts(sha256)
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_runtime_catalog_artifact_identity
             ON runtime_catalog_artifacts(kind,slug,version);
         CREATE INDEX IF NOT EXISTS idx_runtime_catalog_snapshot_artifact_hash
             ON runtime_catalog_snapshot_artifacts(sha256);",
    )
    .expect("failed to create Runtime Catalog last-known-good storage");
    let _ = conn.execute(
        "ALTER TABLE runtime_catalog_state
         ADD COLUMN trusted_revocations_json TEXT NOT NULL DEFAULT '[]'",
        [],
    );
}

#[cfg(test)]
mod tests {
    use super::run_migrations;
    use rusqlite::{Connection, params};

    #[test]
    fn migration_repairs_blank_interactive_session_titles() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn);
        conn.execute(
            "INSERT INTO sessions (id, project_path, title, created_at, updated_at)
             VALUES ('interactive', '/projects/replex', NULL, 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_messages (session_id, role, message, created_at)
             VALUES ('interactive', 'user', ?1, 'now')",
            params![r#"[{"type":"text","text":"Repair the Replex session identity"}]"#],
        )
        .unwrap();

        run_migrations(&conn);

        let title: String = conn
            .query_row(
                "SELECT title FROM sessions WHERE id='interactive'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "Repair the Replex session identity");
    }

    #[test]
    fn input_request_storage_migrates_additively_and_idempotently() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE legacy_user_state (id INTEGER PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO legacy_user_state (id,value) VALUES (1,'preserved');",
        )
        .unwrap();

        run_migrations(&conn);
        run_migrations(&conn);

        let preserved: String = conn
            .query_row(
                "SELECT value FROM legacy_user_state WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let columns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('input_requests')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let partial_unique: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_input_requests_one_pending_run'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(preserved, "preserved");
        assert_eq!(columns, 16);
        assert_eq!(partial_unique, 1);
    }

    #[test]
    fn runtime_catalog_storage_migrates_existing_databases_idempotently() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE legacy_user_state (id INTEGER PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO legacy_user_state (id,value) VALUES (1,'preserved');
             CREATE TABLE runtime_catalog_state (
                id INTEGER PRIMARY KEY CHECK (id=1),
                channel TEXT NOT NULL DEFAULT 'stable',
                active_revision INTEGER,
                previous_revision INTEGER,
                highest_accepted_revision INTEGER NOT NULL DEFAULT 0,
                active_key_id TEXT,
                etag TEXT,
                last_attempt_at TEXT,
                last_success_at TEXT,
                last_error TEXT,
                frozen INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL
             );
             INSERT INTO runtime_catalog_state
                (id,channel,highest_accepted_revision,frozen,updated_at)
             VALUES (1,'stable',0,0,'old');",
        )
        .unwrap();

        run_migrations(&conn);
        run_migrations(&conn);

        let preserved: String = conn
            .query_row(
                "SELECT value FROM legacy_user_state WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let state: (String, u64, i64, String) = conn
            .query_row(
                "SELECT channel,highest_accepted_revision,frozen,trusted_revocations_json
                 FROM runtime_catalog_state WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(preserved, "preserved");
        assert_eq!(state, ("stable".to_string(), 0, 0, "[]".to_string()));
    }
}
