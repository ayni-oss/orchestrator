impl Store {
    #[allow(clippy::too_many_lines)] // One ordered SQLite schema migration is easier to audit atomically.
    pub(crate) fn migrate_development(&self) -> Result<(), StoreError> {
        self.connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS repository_bindings (
                id TEXT PRIMARY KEY, owner TEXT NOT NULL, repository TEXT NOT NULL,
                installation_id INTEGER NOT NULL, base_branch TEXT NOT NULL,
                lock_sha256 TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS repository_actions (
                action_key TEXT PRIMARY KEY,
                repository_id TEXT NOT NULL,
                binding_snapshot TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS application_list_snapshots (
                token TEXT PRIMARY KEY,
                method TEXT NOT NULL,
                filter TEXT NOT NULL,
                item_ids TEXT NOT NULL,
                item_values TEXT,
                item_offset INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS application_list_snapshots_created_idx
                ON application_list_snapshots(created_at);
            CREATE TABLE IF NOT EXISTS agent_profiles (
                id TEXT PRIMARY KEY, provider TEXT NOT NULL, model TEXT NOT NULL,
                secret_ref TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS development_tasks (
                id TEXT PRIMARY KEY, repository_id TEXT NOT NULL REFERENCES repository_bindings(id),
                profile_id TEXT NOT NULL REFERENCES agent_profiles(id),
                repository_snapshot TEXT NOT NULL, profile_snapshot TEXT NOT NULL,
                start_revision TEXT NOT NULL, request TEXT NOT NULL,
                acceptance_criteria TEXT NOT NULL, protected_paths TEXT NOT NULL DEFAULT '[]',
                allow_test_edits INTEGER NOT NULL DEFAULT 0, phase TEXT NOT NULL,
                preparation_id TEXT, retry_of TEXT, provided_input TEXT, requested_input TEXT,
                candidate_commit TEXT, validation_manifest TEXT,
                policy_or_test_edits INTEGER, scope_violations TEXT, draft_pr_url TEXT, failure TEXT,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                deadline_seconds INTEGER, deadline_at INTEGER,
                lifecycle_complete INTEGER NOT NULL DEFAULT 0,
                resource_version INTEGER NOT NULL DEFAULT 1
            );
            CREATE TABLE IF NOT EXISTS development_actions (
                action_key TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES development_tasks(id),
                action_kind TEXT NOT NULL, created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS development_lifecycle_events (
                event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL REFERENCES development_tasks(id),
                event_key TEXT NOT NULL,
                event_time INTEGER NOT NULL,
                stage TEXT NOT NULL,
                outcome TEXT NOT NULL,
                reason_code TEXT,
                UNIQUE(task_id, event_key)
            );
            CREATE INDEX IF NOT EXISTS development_lifecycle_task_idx
                ON development_lifecycle_events(task_id, event_id);
            CREATE TABLE IF NOT EXISTS development_model_usage_collection (
                task_id TEXT PRIMARY KEY REFERENCES development_tasks(id),
                complete INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS development_model_usage (
                task_id TEXT NOT NULL REFERENCES development_tasks(id),
                stage TEXT NOT NULL, call_id TEXT NOT NULL,
                provider TEXT NOT NULL, model TEXT NOT NULL, resolved_model TEXT,
                started_at_ms INTEGER NOT NULL, completed_at_ms INTEGER NOT NULL,
                latency_ms INTEGER NOT NULL, outcome TEXT NOT NULL,
                input_tokens INTEGER, output_tokens INTEGER, cached_input_tokens INTEGER,
                reasoning_tokens INTEGER, total_tokens INTEGER,
                estimated_cost_micros INTEGER, pricing_currency TEXT, pricing_version TEXT,
                priced_input_tokens INTEGER, priced_cached_input_tokens INTEGER,
                priced_output_tokens INTEGER, input_rate INTEGER, cached_input_rate INTEGER,
                output_rate INTEGER,
                PRIMARY KEY(task_id, stage, call_id)
            );
            CREATE INDEX IF NOT EXISTS development_model_usage_task_idx
                ON development_model_usage(task_id, stage, call_id);
            CREATE TABLE IF NOT EXISTS publication_actions (
                task_id TEXT PRIMARY KEY REFERENCES development_tasks(id),
                branch_name TEXT NOT NULL UNIQUE, candidate_commit TEXT NOT NULL,
                status TEXT NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL,
                pr_number INTEGER, pr_url TEXT,
                updated_at INTEGER NOT NULL
            );",
            )
            .map_err(StoreError::Database)?;
        let list_values_exist: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM pragma_table_info('application_list_snapshots')
                    WHERE name='item_values'
                )",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::Database)?;
        if !list_values_exist {
            self.connection
                .execute(
                    "ALTER TABLE application_list_snapshots ADD COLUMN item_values TEXT",
                    [],
                )
                .map_err(StoreError::Database)?;
        }
        for (name, definition) in [
            ("preparation_id", "TEXT"),
            ("retry_of", "TEXT"),
            ("provided_input", "TEXT"),
            ("requested_input", "TEXT"),
            ("deadline_seconds", "INTEGER"),
            ("deadline_at", "INTEGER"),
            ("protected_paths", "TEXT NOT NULL DEFAULT '[]'"),
            ("allow_test_edits", "INTEGER NOT NULL DEFAULT 0"),
            ("scope_violations", "TEXT"),
            ("lifecycle_complete", "INTEGER NOT NULL DEFAULT 0"),
            ("resource_version", "INTEGER NOT NULL DEFAULT 1"),
        ] {
            let exists: bool = self
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('development_tasks') WHERE name=?1)",
                    [name],
                    |row| row.get(0),
                )
                .map_err(StoreError::Database)?;
            if !exists {
                self.connection
                    .execute(
                        &format!("ALTER TABLE development_tasks ADD COLUMN {name} {definition}"),
                        [],
                    )
                    .map_err(StoreError::Database)?;
            }
        }
        // Before `provided_input` existed, follow-up text was appended to the
        // child request. Normalize those rows so the original request remains
        // frozen and durable action replays use the canonical shape.
        self.connection
            .execute(
                "UPDATE development_tasks AS child
                 SET provided_input = substr(
                         child.request,
                         length(parent.request || char(10) || char(10) || 'Maintainer input:' || char(10)) + 1
                     ),
                     request = parent.request
                 FROM development_tasks AS parent
                 WHERE child.retry_of = parent.id
                   AND child.provided_input IS NULL
                   AND substr(
                         child.request,
                         1,
                         length(parent.request || char(10) || char(10) || 'Maintainer input:' || char(10))
                       ) = parent.request || char(10) || char(10) || 'Maintainer input:' || char(10)",
                [],
            )
            .map_err(StoreError::Database)?;
        super::development_lifecycle::backfill_development_lifecycle(&self.connection)
    }

}
