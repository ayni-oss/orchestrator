-- Task feedback and recovery: final table definitions for fresh installations.

CREATE TABLE public.workflow_task_feedback (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    task_id text NOT NULL,
    attempt_id text NOT NULL,
    category text NOT NULL,
    safe_reason text NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_task_feedback_category_check CHECK ((category = ANY (ARRAY['execution'::text, 'quality'::text]))),
    CONSTRAINT workflow_task_feedback_safe_reason_check CHECK ((safe_reason = ANY (ARRAY['execution_failed'::text, 'execution_interrupted'::text, 'provider_uncertain'::text, 'execution_deadline_exceeded'::text, 'quality_failed'::text, 'quality_blocked'::text, 'quality_interrupted'::text]))),
    CONSTRAINT workflow_task_feedback_attempt_id_category_key UNIQUE (attempt_id, category),
    CONSTRAINT workflow_task_feedback_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_task_feedback_attempt_id_fkey FOREIGN KEY (attempt_id) REFERENCES public.workflow_task_runs(attempt_id),
    CONSTRAINT workflow_task_feedback_workspace_id_project_id_conversatio_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, task_id) REFERENCES public.workflow_tasks(workspace_id, project_id, conversation_id, id)
);

CREATE INDEX workflow_task_feedback_conversation ON public.workflow_task_feedback USING btree (workspace_id, project_id, conversation_id, id);

CREATE INDEX workflow_task_feedback_project ON public.workflow_task_feedback USING btree (workspace_id, project_id, id);

CREATE TABLE public.workflow_task_recovery (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    task_id text NOT NULL,
    attempt_id text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    kind text NOT NULL,
    question_id text,
    content bytea NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_task_recovery_check CHECK (((kind = 'answer'::text) = (question_id IS NOT NULL))),
    CONSTRAINT workflow_task_recovery_kind_check CHECK ((kind = ANY (ARRAY['proposal'::text, 'question'::text, 'answer'::text]))),
    CONSTRAINT workflow_task_recovery_attempt_id_idempotency_key_key UNIQUE (attempt_id, idempotency_key),
    CONSTRAINT workflow_task_recovery_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_task_recovery_question_id_key UNIQUE (question_id),
    CONSTRAINT workflow_task_recovery_attempt_id_fkey FOREIGN KEY (attempt_id) REFERENCES public.workflow_task_runs(attempt_id),
    CONSTRAINT workflow_task_recovery_question_id_fkey FOREIGN KEY (question_id) REFERENCES public.workflow_task_recovery(id),
    CONSTRAINT workflow_task_recovery_workspace_id_project_id_conversatio_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, task_id) REFERENCES public.workflow_tasks(workspace_id, project_id, conversation_id, id)
);

CREATE INDEX workflow_task_recovery_origin ON public.workflow_task_recovery USING btree (workspace_id, project_id, conversation_id, task_id, attempt_id, id);

CREATE TABLE public.workflow_task_recovery_confirmations (
    proposal_id text NOT NULL,
    parent_attempt_id text NOT NULL,
    attempt_id text NOT NULL,
    request_digest bytea NOT NULL,
    result bytea NOT NULL,
    CONSTRAINT workflow_task_recovery_confirmations_check CHECK ((parent_attempt_id <> attempt_id)),
    CONSTRAINT workflow_task_recovery_confirmations_attempt_id_key UNIQUE (attempt_id),
    CONSTRAINT workflow_task_recovery_confirmations_parent_attempt_id_key UNIQUE (parent_attempt_id),
    CONSTRAINT workflow_task_recovery_confirmations_pkey PRIMARY KEY (proposal_id),
    CONSTRAINT workflow_task_recovery_confirmations_attempt_id_fkey FOREIGN KEY (attempt_id) REFERENCES public.workflow_task_runs(attempt_id),
    CONSTRAINT workflow_task_recovery_confirmations_parent_attempt_id_fkey FOREIGN KEY (parent_attempt_id) REFERENCES public.workflow_task_runs(attempt_id),
    CONSTRAINT workflow_task_recovery_confirmations_proposal_id_fkey FOREIGN KEY (proposal_id) REFERENCES public.workflow_task_recovery(id)
);

CREATE TABLE public.workflow_task_recovery_confirmation_receipts (
    conversation_id text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    proposal_id text NOT NULL,
    result bytea NOT NULL,
    CONSTRAINT workflow_task_recovery_confirmation_recei_idempotency_key_check CHECK (((octet_length(idempotency_key) >= 1) AND (octet_length(idempotency_key) <= 128))),
    CONSTRAINT workflow_task_recovery_confirmation_receipts_pkey PRIMARY KEY (conversation_id, idempotency_key),
    CONSTRAINT workflow_task_recovery_confirmation_receip_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES public.workflow_conversations(id),
    CONSTRAINT workflow_task_recovery_confirmation_receipts_proposal_id_fkey FOREIGN KEY (proposal_id) REFERENCES public.workflow_task_recovery_confirmations(proposal_id)
);
