-- Task publication: final table definitions for fresh installations.

CREATE TABLE public.workflow_task_publications (
    publication_id text NOT NULL,
    attempt_id text NOT NULL,
    task_id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    repository text NOT NULL,
    command bytea NOT NULL,
    state text NOT NULL,
    generation bigint DEFAULT 1 NOT NULL,
    pod_uid text DEFAULT ''::text NOT NULL,
    lease_until timestamp with time zone,
    commit_sha text DEFAULT ''::text NOT NULL,
    tree_sha text DEFAULT ''::text NOT NULL,
    outcome bytea,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_task_publications_command_check CHECK (((octet_length(command) >= 1) AND (octet_length(command) <= 262144))),
    CONSTRAINT workflow_task_publications_commit_sha_check CHECK ((octet_length(commit_sha) = ANY (ARRAY[0, 40]))),
    CONSTRAINT workflow_task_publications_generation_check CHECK ((generation > 0)),
    CONSTRAINT workflow_task_publications_outcome_check CHECK (((octet_length(outcome) >= 1) AND (octet_length(outcome) <= 8192))),
    CONSTRAINT workflow_task_publications_pod_uid_check CHECK ((octet_length(pod_uid) <= 64)),
    CONSTRAINT workflow_task_publications_state_check CHECK ((state = ANY (ARRAY['queued'::text, 'claimed'::text, 'prepared'::text, 'creating'::text, 'uncertain'::text, 'draft'::text, 'blocked'::text, 'cancelled'::text]))),
    CONSTRAINT workflow_task_publications_tree_sha_check CHECK ((octet_length(tree_sha) = ANY (ARRAY[0, 40]))),
    CONSTRAINT workflow_task_publications_attempt_id_key UNIQUE (attempt_id),
    CONSTRAINT workflow_task_publications_pkey PRIMARY KEY (publication_id),
    CONSTRAINT workflow_task_publications_attempt_id_fkey FOREIGN KEY (attempt_id) REFERENCES public.workflow_task_runs(attempt_id),
    CONSTRAINT workflow_task_publications_task_id_fkey FOREIGN KEY (task_id) REFERENCES public.workflow_tasks(id),
    CONSTRAINT workflow_task_publications_workspace_id_project_id_convers_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, task_id) REFERENCES public.workflow_tasks(workspace_id, project_id, conversation_id, id)
);

CREATE UNIQUE INDEX workflow_task_publications_one_active ON public.workflow_task_publications USING btree (workspace_id, repository) WHERE (state = ANY (ARRAY['claimed'::text, 'prepared'::text, 'creating'::text, 'uncertain'::text]));

CREATE TABLE public.workflow_task_publication_receipts (
    command_id text NOT NULL,
    conversation_id text NOT NULL,
    method text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    publication_id text NOT NULL,
    result bytea NOT NULL,
    CONSTRAINT workflow_task_publication_receipts_idempotency_key_check CHECK (((octet_length(idempotency_key) >= 1) AND (octet_length(idempotency_key) <= 128))),
    CONSTRAINT workflow_task_publication_receipts_method_check CHECK ((method = ANY (ARRAY['StartTaskPublication'::text, 'CancelTaskPublication'::text]))),
    CONSTRAINT workflow_task_publication_receipts_request_digest_check CHECK ((octet_length(request_digest) = 32)),
    CONSTRAINT workflow_task_publication_receipts_result_check CHECK (((octet_length(result) >= 1) AND (octet_length(result) <= 8192))),
    CONSTRAINT workflow_task_publication_rec_conversation_id_method_idempo_key UNIQUE (conversation_id, method, idempotency_key),
    CONSTRAINT workflow_task_publication_receipts_pkey PRIMARY KEY (command_id),
    CONSTRAINT workflow_task_publication_receipts_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES public.workflow_conversations(id),
    CONSTRAINT workflow_task_publication_receipts_publication_id_fkey FOREIGN KEY (publication_id) REFERENCES public.workflow_task_publications(publication_id)
);

CREATE TABLE public.workflow_task_publication_reports (
    publication_id text NOT NULL,
    ordinal smallint NOT NULL,
    report_digest text NOT NULL,
    outcome bytea NOT NULL,
    generation bigint NOT NULL,
    pod_uid text NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_task_publication_reports_generation_check CHECK ((generation > 0)),
    CONSTRAINT workflow_task_publication_reports_ordinal_check CHECK (((ordinal >= 1) AND (ordinal <= 2))),
    CONSTRAINT workflow_task_publication_reports_outcome_check CHECK (((octet_length(outcome) >= 1) AND (octet_length(outcome) <= 8192))),
    CONSTRAINT workflow_task_publication_reports_pod_uid_check CHECK ((octet_length(pod_uid) <= 64)),
    CONSTRAINT workflow_task_publication_reports_report_digest_check CHECK ((octet_length(report_digest) = 71)),
    CONSTRAINT workflow_task_publication_repo_publication_id_report_digest_key UNIQUE (publication_id, report_digest),
    CONSTRAINT workflow_task_publication_reports_pkey PRIMARY KEY (publication_id, ordinal),
    CONSTRAINT workflow_task_publication_reports_publication_id_fkey FOREIGN KEY (publication_id) REFERENCES public.workflow_task_publications(publication_id)
);
