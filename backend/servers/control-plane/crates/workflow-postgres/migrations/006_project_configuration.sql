-- Project configuration: final table definitions for fresh installations.

CREATE TABLE public.workflow_agent_configurations (
    workspace_id text NOT NULL,
    revision_id text NOT NULL,
    instructions text NOT NULL,
    provider text NOT NULL,
    model text NOT NULL,
    max_input_tokens bigint NOT NULL,
    max_output_tokens bigint NOT NULL,
    max_total_tokens bigint NOT NULL,
    idle_timeout_seconds bigint NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    connection_id text DEFAULT ''::text NOT NULL,
    CONSTRAINT workflow_agent_configurations_check CHECK ((((max_total_tokens >= 2) AND (max_total_tokens <= 1100000)) AND (max_total_tokens >= (max_input_tokens + max_output_tokens)))),
    CONSTRAINT workflow_agent_configurations_idle_timeout_seconds_check CHECK (((idle_timeout_seconds >= 30) AND (idle_timeout_seconds <= 86400))),
    CONSTRAINT workflow_agent_configurations_instructions_check CHECK ((octet_length(instructions) <= 16384)),
    CONSTRAINT workflow_agent_configurations_max_input_tokens_check CHECK (((max_input_tokens >= 1) AND (max_input_tokens <= 1000000))),
    CONSTRAINT workflow_agent_configurations_max_output_tokens_check CHECK (((max_output_tokens >= 1) AND (max_output_tokens <= 100000))),
    CONSTRAINT workflow_agent_configurations_model_check CHECK (((octet_length(model) >= 1) AND (octet_length(model) <= 256))),
    CONSTRAINT workflow_agent_configurations_provider_check CHECK ((provider = ANY (ARRAY['scripted'::text, 'openai'::text, 'anthropic'::text]))),
    CONSTRAINT workflow_agent_configurations_pkey PRIMARY KEY (workspace_id, revision_id),
    CONSTRAINT workflow_agent_configurations_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES public.workflow_workspaces(id)
);

CREATE TABLE public.workflow_project_configuration_revisions (
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    revision_id text NOT NULL,
    CONSTRAINT workflow_project_configuration_revisions_pkey PRIMARY KEY (project_id, revision_id),
    CONSTRAINT workflow_project_configuration_r_workspace_id_revision_id_fkey1 FOREIGN KEY (workspace_id, revision_id) REFERENCES public.workflow_agent_configurations(workspace_id, revision_id),
    CONSTRAINT workflow_project_configuration_rev_workspace_id_project_id_fkey FOREIGN KEY (workspace_id, project_id) REFERENCES public.workflow_projects(workspace_id, id),
    CONSTRAINT project_configuration_revision_owner UNIQUE (workspace_id, revision_id)
);

CREATE TABLE public.workflow_project_configuration_heads (
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    revision_id text NOT NULL,
    CONSTRAINT workflow_project_configuration_heads_pkey PRIMARY KEY (project_id),
    CONSTRAINT workflow_project_configuration_he_workspace_id_revision_id_fkey FOREIGN KEY (workspace_id, revision_id) REFERENCES public.workflow_agent_configurations(workspace_id, revision_id),
    CONSTRAINT workflow_project_configuration_hea_workspace_id_project_id_fkey FOREIGN KEY (workspace_id, project_id) REFERENCES public.workflow_projects(workspace_id, id),
    CONSTRAINT project_configuration_head_owner FOREIGN KEY (project_id, revision_id)
    REFERENCES public.workflow_project_configuration_revisions(project_id, revision_id)
    DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE public.workflow_project_configuration_receipts (
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    revision_id text NOT NULL,
    CONSTRAINT workflow_project_configuration_receipts_request_digest_check CHECK ((octet_length(request_digest) = 32)),
    CONSTRAINT workflow_project_configuration_receipts_pkey PRIMARY KEY (project_id, idempotency_key),
    CONSTRAINT workflow_project_configuration_re_workspace_id_revision_id_fkey FOREIGN KEY (workspace_id, revision_id) REFERENCES public.workflow_agent_configurations(workspace_id, revision_id),
    CONSTRAINT workflow_project_configuration_rec_workspace_id_project_id_fkey FOREIGN KEY (workspace_id, project_id) REFERENCES public.workflow_projects(workspace_id, id),
    CONSTRAINT project_configuration_receipt_owner FOREIGN KEY (project_id, revision_id)
    REFERENCES public.workflow_project_configuration_revisions(project_id, revision_id)
);
