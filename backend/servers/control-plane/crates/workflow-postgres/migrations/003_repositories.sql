-- Repositories: final table definitions for fresh installations.

CREATE TABLE public.workflow_repositories (
    id text NOT NULL,
    connection_id text NOT NULL,
    installation_id bigint NOT NULL,
    provider_repository_id bigint NOT NULL,
    full_name text NOT NULL,
    default_branch text NOT NULL,
    private boolean NOT NULL,
    connected_at_unix_ms bigint DEFAULT ((EXTRACT(epoch FROM clock_timestamp()) * (1000)::numeric))::bigint NOT NULL,
    CONSTRAINT workflow_repositories_default_branch_check CHECK (((octet_length(default_branch) >= 1) AND (octet_length(default_branch) <= 256))),
    CONSTRAINT workflow_repositories_full_name_check CHECK (((octet_length(full_name) >= 3) AND (octet_length(full_name) <= 256))),
    CONSTRAINT workflow_repositories_installation_id_check CHECK ((installation_id > 0)),
    CONSTRAINT workflow_repositories_provider_repository_id_check CHECK ((provider_repository_id > 0)),
    CONSTRAINT workflow_repositories_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_repositories_provider_repository_id_key UNIQUE (provider_repository_id),
    CONSTRAINT workflow_repositories_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES public.workflow_connections(id)
);

CREATE TABLE public.workflow_repository_association_receipts (
    idempotency_key text NOT NULL,
    command_id text NOT NULL,
    safe_request text NOT NULL,
    repository_id text NOT NULL,
    CONSTRAINT workflow_repository_association_receipts_command_id_key UNIQUE (command_id),
    CONSTRAINT workflow_repository_association_receipts_pkey PRIMARY KEY (idempotency_key),
    CONSTRAINT workflow_repository_association_receipts_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES public.workflow_repositories(id)
);

CREATE TABLE public.workflow_repository_inspections (
    id text NOT NULL,
    idempotency_key text NOT NULL,
    safe_request text NOT NULL,
    repository_id text NOT NULL,
    connection_id text NOT NULL,
    connection_revision bigint NOT NULL,
    credential_revision text NOT NULL,
    commit_sha text NOT NULL,
    state text NOT NULL,
    safe_reason text DEFAULT ''::text NOT NULL,
    result text DEFAULT ''::text NOT NULL,
    accepted_at_unix_ms bigint DEFAULT ((EXTRACT(epoch FROM clock_timestamp()) * (1000)::numeric))::bigint NOT NULL,
    updated_at_unix_ms bigint DEFAULT ((EXTRACT(epoch FROM clock_timestamp()) * (1000)::numeric))::bigint NOT NULL,
    CONSTRAINT workflow_repository_inspections_commit_sha_check CHECK ((commit_sha ~ '^[0-9a-f]{40}$'::text)),
    CONSTRAINT workflow_repository_inspections_connection_revision_check CHECK ((connection_revision > 0)),
    CONSTRAINT workflow_repository_inspections_state_check CHECK ((state = ANY (ARRAY['accepted'::text, 'completed'::text, 'failed'::text, 'superseded'::text]))),
    CONSTRAINT workflow_repository_inspections_idempotency_key_key UNIQUE (idempotency_key),
    CONSTRAINT workflow_repository_inspections_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_repository_inspections_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES public.workflow_connections(id),
    CONSTRAINT workflow_repository_inspections_credential_revision_fkey FOREIGN KEY (credential_revision) REFERENCES public.workflow_connection_credentials(revision_id),
    CONSTRAINT workflow_repository_inspections_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES public.workflow_repositories(id)
);

CREATE INDEX workflow_repository_inspections_repository_history ON public.workflow_repository_inspections USING btree (repository_id, id DESC);

CREATE TABLE public.workflow_repository_operations (
    id text NOT NULL,
    idempotency_key text NOT NULL,
    safe_request text NOT NULL,
    connection_id text NOT NULL,
    connection_revision bigint NOT NULL,
    credential_revision text NOT NULL,
    installation_id bigint NOT NULL,
    provider_repository_id bigint NOT NULL,
    state text NOT NULL,
    safe_reason text DEFAULT ''::text NOT NULL,
    result text DEFAULT ''::text NOT NULL,
    accepted_at_unix_ms bigint DEFAULT ((EXTRACT(epoch FROM clock_timestamp()) * (1000)::numeric))::bigint NOT NULL,
    updated_at_unix_ms bigint DEFAULT ((EXTRACT(epoch FROM clock_timestamp()) * (1000)::numeric))::bigint NOT NULL,
    CONSTRAINT workflow_repository_operations_connection_revision_check CHECK ((connection_revision > 0)),
    CONSTRAINT workflow_repository_operations_installation_id_check CHECK ((installation_id > 0)),
    CONSTRAINT workflow_repository_operations_provider_repository_id_check CHECK ((provider_repository_id > 0)),
    CONSTRAINT workflow_repository_operations_state_check CHECK ((state = ANY (ARRAY['accepted'::text, 'completed'::text, 'failed'::text, 'superseded'::text]))),
    CONSTRAINT workflow_repository_operations_idempotency_key_key UNIQUE (idempotency_key),
    CONSTRAINT workflow_repository_operations_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_repository_operations_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES public.workflow_connections(id),
    CONSTRAINT workflow_repository_operations_credential_revision_fkey FOREIGN KEY (credential_revision) REFERENCES public.workflow_connection_credentials(revision_id)
);

CREATE TABLE public.workflow_repository_runtime_runs (
    id text NOT NULL,
    idempotency_key text NOT NULL,
    repository_id text NOT NULL,
    safe_request text NOT NULL,
    receipt text NOT NULL,
    state text NOT NULL,
    progress text NOT NULL,
    claim_token text DEFAULT ''::text NOT NULL,
    lease_until timestamp with time zone DEFAULT '-infinity'::timestamp with time zone NOT NULL,
    CONSTRAINT workflow_repository_runtime_runs_check CHECK ((((progress)::jsonb ->> 'state'::text) = state)),
    CONSTRAINT workflow_repository_runtime_runs_progress_check CHECK ((octet_length(progress) <= 32768)),
    CONSTRAINT workflow_repository_runtime_runs_state_check CHECK ((state = ANY (ARRAY['resolving'::text, 'inspecting'::text, 'admitting'::text, 'completed'::text, 'blocked'::text]))),
    CONSTRAINT workflow_repository_runtime_runs_idempotency_key_key UNIQUE (idempotency_key),
    CONSTRAINT workflow_repository_runtime_runs_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_repository_runtime_runs_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES public.workflow_repositories(id)
);

CREATE UNIQUE INDEX workflow_repository_runtime_active ON public.workflow_repository_runtime_runs USING btree (repository_id) WHERE (state = ANY (ARRAY['resolving'::text, 'inspecting'::text, 'admitting'::text]));

CREATE INDEX workflow_repository_runtime_queue ON public.workflow_repository_runtime_runs USING btree (lease_until, id) WHERE (state = ANY (ARRAY['resolving'::text, 'inspecting'::text, 'admitting'::text]));

CREATE TABLE public.workflow_workspace_repositories (
    workspace_id text NOT NULL,
    repository_id text NOT NULL,
    CONSTRAINT workflow_workspace_repositories_pkey PRIMARY KEY (workspace_id, repository_id),
    CONSTRAINT workflow_workspace_repositories_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES public.workflow_repositories(id),
    CONSTRAINT workflow_workspace_repositories_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES public.workflow_workspaces(id)
);

CREATE TABLE public.workflow_project_repositories (
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    repository_id text NOT NULL,
    CONSTRAINT workflow_project_repositories_pkey PRIMARY KEY (workspace_id, project_id, repository_id),
    CONSTRAINT workflow_project_repositories_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES public.workflow_repositories(id),
    CONSTRAINT workflow_project_repositories_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES public.workflow_workspaces(id),
    CONSTRAINT workflow_project_repositories_workspace_id_project_id_fkey FOREIGN KEY (workspace_id, project_id) REFERENCES public.workflow_projects(workspace_id, id)
);

CREATE INDEX workflow_project_repositories_workspace_repository_idx ON public.workflow_project_repositories USING btree (workspace_id, repository_id);
