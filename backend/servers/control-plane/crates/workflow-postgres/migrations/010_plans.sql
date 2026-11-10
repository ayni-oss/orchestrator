-- Plans: final table definitions for fresh installations.

CREATE TABLE public.workflow_plans (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    current_version_id text NOT NULL,
    follows_plan_id text,
    CONSTRAINT workflow_plans_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_plans_workspace_id_project_id_conversation_id_id_c_key UNIQUE (workspace_id, project_id, conversation_id, id, current_version_id),
    CONSTRAINT workflow_plans_workspace_id_project_id_conversation_id_id_key UNIQUE (workspace_id, project_id, conversation_id, id),
    CONSTRAINT workflow_plans_workspace_id_project_id_conversation_id_fkey FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES public.workflow_conversations(workspace_id, project_id, id),
    CONSTRAINT workflow_plans_workspace_id_project_id_conversation_id_fol_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, follows_plan_id) REFERENCES public.workflow_plans(workspace_id, project_id, conversation_id, id)
);

CREATE TABLE public.workflow_plan_versions (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    plan_id text NOT NULL,
    ordinal bigint NOT NULL,
    predecessor_id text,
    content bytea NOT NULL,
    content_digest text NOT NULL,
    title text NOT NULL,
    author text NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_plan_versions_author_check CHECK ((author = ANY (ARRAY['local-operator'::text, 'agent'::text]))),
    CONSTRAINT workflow_plan_versions_check CHECK (((ordinal = 1) = (predecessor_id IS NULL))),
    CONSTRAINT workflow_plan_versions_content_check CHECK (((octet_length(content) >= 1) AND (octet_length(content) <= 32768))),
    CONSTRAINT workflow_plan_versions_content_digest_check CHECK ((content_digest ~ '^sha256:[0-9a-f]{64}$'::text)),
    CONSTRAINT workflow_plan_versions_ordinal_check CHECK (((ordinal >= 1) AND (ordinal <= 64))),
    CONSTRAINT workflow_plan_versions_title_check CHECK (((octet_length(title) >= 1) AND (octet_length(title) <= 256))),
    CONSTRAINT workflow_plan_versions_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_plan_versions_plan_id_ordinal_key UNIQUE (plan_id, ordinal),
    CONSTRAINT workflow_plan_versions_workspace_id_project_id_conversatio_key1 UNIQUE (workspace_id, project_id, conversation_id, plan_id, id, content_digest),
    CONSTRAINT workflow_plan_versions_workspace_id_project_id_conversation_key UNIQUE (workspace_id, project_id, conversation_id, plan_id, id),
    CONSTRAINT workflow_plan_versions_workspace_id_project_id_conversati_fkey1 FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, predecessor_id) REFERENCES public.workflow_plan_versions(workspace_id, project_id, conversation_id, plan_id, id),
    CONSTRAINT workflow_plan_versions_workspace_id_project_id_conversatio_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id) REFERENCES public.workflow_plans(workspace_id, project_id, conversation_id, id)
);

CREATE TABLE public.workflow_plan_receipts (
    conversation_id text NOT NULL,
    method text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    command_id text NOT NULL,
    version_id text NOT NULL,
    conversation_revision bigint NOT NULL,
    resume_cursor text NOT NULL,
    CONSTRAINT workflow_plan_receipts_conversation_revision_check CHECK ((conversation_revision > 0)),
    CONSTRAINT workflow_plan_receipts_idempotency_key_check CHECK (((octet_length(idempotency_key) >= 1) AND (octet_length(idempotency_key) <= 128))),
    CONSTRAINT workflow_plan_receipts_method_check CHECK ((method = ANY (ARRAY['CreatePlan'::text, 'RevisePlan'::text]))),
    CONSTRAINT workflow_plan_receipts_request_digest_check CHECK ((octet_length(request_digest) = 32)),
    CONSTRAINT workflow_plan_receipts_command_id_key UNIQUE (command_id),
    CONSTRAINT workflow_plan_receipts_pkey PRIMARY KEY (conversation_id, method, idempotency_key),
    CONSTRAINT workflow_plan_receipts_version_id_key UNIQUE (version_id),
    CONSTRAINT workflow_plan_receipts_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES public.workflow_conversations(id),
    CONSTRAINT workflow_plan_receipts_version_id_fkey FOREIGN KEY (version_id) REFERENCES public.workflow_plan_versions(id)
);

CREATE TABLE public.workflow_plan_proposals (
    version_id text NOT NULL,
    plan_id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    source_turn_id text NOT NULL,
    configuration_revision_id text NOT NULL,
    context_digest text NOT NULL,
    provider_call_id text NOT NULL,
    provider_response_id text NOT NULL,
    assignment_id text NOT NULL,
    executor_generation bigint NOT NULL,
    report_source text NOT NULL,
    report_id text NOT NULL,
    report_digest text NOT NULL,
    invocation_id text NOT NULL,
    assumptions text[] NOT NULL,
    evidence text[] NOT NULL,
    CONSTRAINT workflow_plan_proposals_assumptions_check CHECK ((cardinality(assumptions) <= 16)),
    CONSTRAINT workflow_plan_proposals_evidence_check CHECK ((cardinality(evidence) <= 16)),
    CONSTRAINT workflow_plan_proposals_executor_generation_check CHECK ((executor_generation > 0)),
    CONSTRAINT workflow_plan_proposals_invocation_id_check CHECK (((octet_length(invocation_id) >= 1) AND (octet_length(invocation_id) <= 128))),
    CONSTRAINT workflow_plan_proposals_provider_response_id_check CHECK (((octet_length(provider_response_id) >= 1) AND (octet_length(provider_response_id) <= 256))),
    CONSTRAINT workflow_plan_proposals_pkey PRIMARY KEY (version_id),
    CONSTRAINT workflow_plan_proposals_report_source_report_id_key UNIQUE (report_source, report_id),
    CONSTRAINT workflow_plan_proposals_source_turn_id_key UNIQUE (source_turn_id),
    CONSTRAINT workflow_plan_proposals_report_source_report_id_report_dig_fkey FOREIGN KEY (report_source, report_id, report_digest, workspace_id, project_id, conversation_id, source_turn_id, assignment_id, executor_generation) REFERENCES public.workflow_report_receipts(report_source, report_id, report_digest, workspace_id, project_id, conversation_id, turn_id, assignment_id, executor_generation),
    CONSTRAINT workflow_plan_proposals_source_turn_id_context_digest_fkey FOREIGN KEY (source_turn_id, context_digest) REFERENCES public.workflow_context_manifests(turn_id, digest),
    CONSTRAINT workflow_plan_proposals_source_turn_id_provider_call_id_fkey FOREIGN KEY (source_turn_id, provider_call_id) REFERENCES public.workflow_provider_calls(turn_id, call_id),
    CONSTRAINT workflow_plan_proposals_workspace_id_configuration_revisio_fkey FOREIGN KEY (workspace_id, configuration_revision_id) REFERENCES public.workflow_agent_configurations(workspace_id, revision_id),
    CONSTRAINT workflow_plan_proposals_workspace_id_project_id_conversati_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, version_id) REFERENCES public.workflow_plan_versions(workspace_id, project_id, conversation_id, plan_id, id)
);

CREATE TABLE public.workflow_plan_approvals (
    id text NOT NULL,
    command_id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    plan_id text NOT NULL,
    version_id text NOT NULL,
    content_digest text NOT NULL,
    operator text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    task_ids text[] NOT NULL,
    conversation_revision bigint NOT NULL,
    resume_cursor text NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_plan_approvals_conversation_revision_check CHECK ((conversation_revision > 0)),
    CONSTRAINT workflow_plan_approvals_idempotency_key_check CHECK (((octet_length(idempotency_key) >= 1) AND (octet_length(idempotency_key) <= 128))),
    CONSTRAINT workflow_plan_approvals_operator_check CHECK ((operator = 'local-operator'::text)),
    CONSTRAINT workflow_plan_approvals_request_digest_check CHECK ((octet_length(request_digest) = 32)),
    CONSTRAINT workflow_plan_approvals_task_ids_check CHECK (((cardinality(task_ids) >= 1) AND (cardinality(task_ids) <= 32))),
    CONSTRAINT workflow_plan_approvals_command_id_key UNIQUE (command_id),
    CONSTRAINT workflow_plan_approvals_conversation_id_idempotency_key_key UNIQUE (conversation_id, idempotency_key),
    CONSTRAINT workflow_plan_approvals_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_plan_approvals_plan_id_key UNIQUE (plan_id),
    CONSTRAINT workflow_plan_approvals_version_id_key UNIQUE (version_id),
    CONSTRAINT workflow_plan_approvals_workspace_id_project_id_conversatio_key UNIQUE (workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, id),
    CONSTRAINT workflow_plan_approvals_workspace_id_project_id_conversat_fkey1 FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, version_id) REFERENCES public.workflow_plans(workspace_id, project_id, conversation_id, id, current_version_id),
    CONSTRAINT workflow_plan_approvals_workspace_id_project_id_conversati_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, version_id, content_digest) REFERENCES public.workflow_plan_versions(workspace_id, project_id, conversation_id, plan_id, id, content_digest)
);

-- Complete cyclic references after both tables exist. These constraints
-- belong to this layer and are installed in the same migration transaction.

ALTER TABLE public.workflow_plans
    ADD CONSTRAINT workflow_plans_current_version_fk FOREIGN KEY (workspace_id, project_id, conversation_id, id, current_version_id) REFERENCES public.workflow_plan_versions(workspace_id, project_id, conversation_id, plan_id, id) DEFERRABLE INITIALLY DEFERRED;
