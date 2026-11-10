-- Task execution: final table definitions for fresh installations.

CREATE TABLE public.workflow_tasks (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    plan_id text NOT NULL,
    version_id text NOT NULL,
    content_digest text NOT NULL,
    approval_id text NOT NULL,
    proposed_task_key text NOT NULL,
    specification bytea NOT NULL,
    state text NOT NULL,
    execution_blockers text[] NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_tasks_execution_blockers_check CHECK (((cardinality(execution_blockers) >= 3) AND (cardinality(execution_blockers) <= 19))),
    CONSTRAINT workflow_tasks_proposed_task_key_check CHECK ((proposed_task_key ~ '^[A-Za-z0-9_-]{1,64}$'::text)),
    CONSTRAINT workflow_tasks_specification_check CHECK (((octet_length(specification) >= 1) AND (octet_length(specification) <= 32768))),
    CONSTRAINT workflow_tasks_state_check CHECK ((state = 'awaiting_execution'::text)),
    CONSTRAINT workflow_tasks_approval_id_proposed_task_key_key UNIQUE (approval_id, proposed_task_key),
    CONSTRAINT workflow_tasks_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_tasks_version_id_proposed_task_key_key UNIQUE (version_id, proposed_task_key),
    CONSTRAINT workflow_tasks_workspace_id_project_id_conversation_id_id_key UNIQUE (workspace_id, project_id, conversation_id, id),
    CONSTRAINT workflow_tasks_workspace_id_project_id_conversation_id_plan_key UNIQUE (workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, approval_id, id),
    CONSTRAINT workflow_tasks_workspace_id_project_id_conversation_id_pla_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, approval_id) REFERENCES public.workflow_plan_approvals(workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, id)
);

CREATE INDEX workflow_tasks_conversation_id_idx ON public.workflow_tasks USING btree (conversation_id, id);

CREATE TABLE public.workflow_task_preparations (
    command_id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    task_id text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    result bytea NOT NULL,
    CONSTRAINT workflow_task_preparations_idempotency_key_check CHECK (((octet_length(idempotency_key) >= 1) AND (octet_length(idempotency_key) <= 128))),
    CONSTRAINT workflow_task_preparations_request_digest_check CHECK ((octet_length(request_digest) = 32)),
    CONSTRAINT workflow_task_preparations_result_check CHECK (((octet_length(result) >= 1) AND (octet_length(result) <= 16384))),
    CONSTRAINT workflow_task_preparations_conversation_id_idempotency_key_key UNIQUE (conversation_id, idempotency_key),
    CONSTRAINT workflow_task_preparations_pkey PRIMARY KEY (command_id),
    CONSTRAINT workflow_task_preparations_workspace_id_project_id_conversa_key UNIQUE (workspace_id, project_id, conversation_id, task_id, command_id),
    CONSTRAINT workflow_task_preparations_workspace_id_project_id_convers_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, task_id) REFERENCES public.workflow_tasks(workspace_id, project_id, conversation_id, id)
);

CREATE INDEX workflow_task_preparations_task_idx ON public.workflow_task_preparations USING btree (task_id, command_id);

CREATE TABLE public.workflow_task_attempts (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    plan_id text NOT NULL,
    version_id text NOT NULL,
    content_digest text NOT NULL,
    approval_id text NOT NULL,
    task_id text NOT NULL,
    preparation_command_id text NOT NULL,
    configuration_revision_id text NOT NULL,
    inputs bytea NOT NULL,
    state text NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_task_attempts_inputs_check CHECK (((octet_length(inputs) >= 1) AND (octet_length(inputs) <= 4096))),
    CONSTRAINT workflow_task_attempts_state_check CHECK ((state = 'prepared'::text)),
    CONSTRAINT workflow_task_attempts_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_task_attempts_workspace_id_configuration_revision_fkey FOREIGN KEY (workspace_id, configuration_revision_id) REFERENCES public.workflow_agent_configurations(workspace_id, revision_id),
    CONSTRAINT workflow_task_attempts_workspace_id_project_id_conversati_fkey1 FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, approval_id, task_id) REFERENCES public.workflow_tasks(workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, approval_id, id),
    CONSTRAINT workflow_task_attempts_workspace_id_project_id_conversatio_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, task_id, preparation_command_id) REFERENCES public.workflow_task_preparations(workspace_id, project_id, conversation_id, task_id, command_id)
);

CREATE TABLE public.workflow_task_runs (
    attempt_id text NOT NULL,
    task_id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    command bytea NOT NULL,
    source_pack bytea NOT NULL,
    state text NOT NULL,
    generation bigint DEFAULT 1 NOT NULL,
    pod_uid text DEFAULT ''::text NOT NULL,
    deadline_at timestamp with time zone,
    provider_state text DEFAULT 'not_started'::text NOT NULL,
    request_digest text DEFAULT ''::text NOT NULL,
    outcome bytea,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    reconciled_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    source_reclaim_after timestamp with time zone,
    source_retired_at timestamp with time zone,
    source_retired_bytes bigint DEFAULT 0 NOT NULL,
    assessment_outcome_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_task_runs_command_check CHECK (((octet_length(command) >= 1) AND (octet_length(command) <= 2097152))),
    CONSTRAINT workflow_task_runs_generation_check CHECK ((generation > 0)),
    CONSTRAINT workflow_task_runs_outcome_check CHECK ((octet_length(outcome) <= 131072)),
    CONSTRAINT workflow_task_runs_pod_uid_check CHECK ((octet_length(pod_uid) <= 64)),
    CONSTRAINT workflow_task_runs_provider_state_check CHECK ((provider_state = ANY (ARRAY['not_started'::text, 'sending'::text, 'settled'::text, 'uncertain'::text]))),
    CONSTRAINT workflow_task_runs_request_digest_check CHECK ((octet_length(request_digest) <= 71)),
    CONSTRAINT workflow_task_runs_source_pack_check CHECK ((((source_retired_at IS NULL) AND ((octet_length(source_pack) >= 1) AND (octet_length(source_pack) <= 8388608)) AND (source_retired_bytes = 0)) OR ((source_retired_at IS NOT NULL) AND (octet_length(source_pack) = 0) AND (source_retired_bytes > 0)))),
    CONSTRAINT workflow_task_runs_source_retired_bytes_check CHECK ((source_retired_bytes >= 0)),
    CONSTRAINT workflow_task_runs_state_check CHECK ((state = ANY (ARRAY['queued'::text, 'claimed'::text, 'reporting'::text, 'cancelling'::text, 'completed'::text, 'failed'::text, 'uncertain'::text, 'cancelled'::text, 'interrupted'::text]))),
    CONSTRAINT workflow_task_runs_pkey PRIMARY KEY (attempt_id),
    CONSTRAINT workflow_task_runs_attempt_id_fkey FOREIGN KEY (attempt_id) REFERENCES public.workflow_task_attempts(id),
    CONSTRAINT workflow_task_runs_task_id_fkey FOREIGN KEY (task_id) REFERENCES public.workflow_tasks(id),
    CONSTRAINT workflow_task_runs_workspace_id_project_id_conversation_id_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, task_id) REFERENCES public.workflow_tasks(workspace_id, project_id, conversation_id, id)
);

CREATE INDEX workflow_task_runs_reconciliation ON public.workflow_task_runs USING btree (reconciled_at, attempt_id) WHERE (state = ANY (ARRAY['claimed'::text, 'reporting'::text, 'cancelling'::text]));

CREATE INDEX workflow_task_runs_source_retention_idx ON public.workflow_task_runs USING btree (source_reclaim_after, attempt_id) WHERE (source_retired_at IS NULL);

CREATE UNIQUE INDEX workflow_task_runs_task_pending ON public.workflow_task_runs USING btree (task_id) WHERE (state = ANY (ARRAY['queued'::text, 'claimed'::text, 'reporting'::text, 'cancelling'::text]));

CREATE TABLE public.workflow_task_run_receipts (
    command_id text NOT NULL,
    conversation_id text NOT NULL,
    method text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    attempt_id text NOT NULL,
    conversation_revision bigint NOT NULL,
    resume_cursor text NOT NULL,
    CONSTRAINT workflow_task_run_receipts_idempotency_key_check CHECK (((octet_length(idempotency_key) >= 1) AND (octet_length(idempotency_key) <= 128))),
    CONSTRAINT workflow_task_run_receipts_method_check CHECK ((method = ANY (ARRAY['StartTaskExecution'::text, 'CancelTaskExecution'::text]))),
    CONSTRAINT workflow_task_run_receipts_request_digest_check CHECK ((octet_length(request_digest) = 32)),
    CONSTRAINT workflow_task_run_receipts_conversation_id_method_idempoten_key UNIQUE (conversation_id, method, idempotency_key),
    CONSTRAINT workflow_task_run_receipts_pkey PRIMARY KEY (command_id),
    CONSTRAINT workflow_task_run_receipts_attempt_id_fkey FOREIGN KEY (attempt_id) REFERENCES public.workflow_task_attempts(id),
    CONSTRAINT workflow_task_run_receipts_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES public.workflow_conversations(id)
);

CREATE TABLE public.workflow_task_quality (
    attempt_id text NOT NULL,
    outcome bytea NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_task_quality_outcome_check CHECK (((octet_length(outcome) >= 1) AND (octet_length(outcome) <= 2097152))),
    CONSTRAINT workflow_task_quality_pkey PRIMARY KEY (attempt_id),
    CONSTRAINT workflow_task_quality_attempt_id_fkey FOREIGN KEY (attempt_id) REFERENCES public.workflow_task_runs(attempt_id)
);
