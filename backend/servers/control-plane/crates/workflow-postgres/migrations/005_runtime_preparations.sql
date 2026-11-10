-- Runtime preparations: final table definitions for fresh installations.

CREATE TABLE public.workflow_runtime_preparations (
    id text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest text NOT NULL,
    repository_id text NOT NULL,
    inspection_id text NOT NULL,
    registry_connection_id text NOT NULL,
    signer_id text NOT NULL,
    platform text NOT NULL,
    frozen text NOT NULL,
    frozen_input_digest text NOT NULL,
    state text NOT NULL,
    stage text NOT NULL,
    safe_reason text DEFAULT ''::text NOT NULL,
    terminal_state text DEFAULT ''::text NOT NULL,
    generation bigint NOT NULL,
    allow_build boolean NOT NULL,
    allow_replace boolean NOT NULL,
    max_builds integer NOT NULL,
    builds_started integer DEFAULT 0 NOT NULL,
    deadline_unix_ms bigint NOT NULL,
    evidence text DEFAULT '{}'::text NOT NULL,
    accepted_at_unix_ms bigint DEFAULT ((EXTRACT(epoch FROM clock_timestamp()) * (1000)::numeric))::bigint NOT NULL,
    updated_at_unix_ms bigint DEFAULT ((EXTRACT(epoch FROM clock_timestamp()) * (1000)::numeric))::bigint NOT NULL,
    CONSTRAINT workflow_runtime_preparations_check CHECK (((builds_started >= 0) AND (builds_started <= max_builds))),
    CONSTRAINT workflow_runtime_preparations_deadline_unix_ms_check CHECK ((deadline_unix_ms > 0)),
    CONSTRAINT workflow_runtime_preparations_frozen_check CHECK ((octet_length(frozen) <= 41943040)),
    CONSTRAINT workflow_runtime_preparations_generation_check CHECK ((generation > 0)),
    CONSTRAINT workflow_runtime_preparations_max_builds_check CHECK (((max_builds >= 0) AND (max_builds <= 10))),
    CONSTRAINT workflow_runtime_preparations_platform_check CHECK ((platform = ANY (ARRAY['linux/amd64'::text, 'linux/arm64'::text]))),
    CONSTRAINT workflow_runtime_preparations_stage_check CHECK ((stage = ANY (ARRAY['acquire_source'::text, 'discover'::text, 'pull_existing'::text, 'build'::text, 'freeze_candidate'::text, 'certify'::text, 'finalize'::text, 'publish'::text, 'pull_final'::text, 'verify_content'::text, 'verify_compatibility'::text, 'cleanup'::text]))),
    CONSTRAINT workflow_runtime_preparations_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'running'::text, 'stopping'::text, 'cancelling'::text, 'cleanup_pending'::text, 'ready'::text, 'cancelled'::text, 'superseded'::text, 'unavailable'::text, 'failed'::text, 'blocked'::text, 'publication_uncertain'::text]))),
    CONSTRAINT workflow_runtime_preparations_idempotency_key_key UNIQUE (idempotency_key),
    CONSTRAINT workflow_runtime_preparations_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_runtime_preparations_inspection_id_fkey FOREIGN KEY (inspection_id) REFERENCES public.workflow_repository_inspections(id),
    CONSTRAINT workflow_runtime_preparations_registry_connection_id_fkey FOREIGN KEY (registry_connection_id) REFERENCES public.workflow_connections(id),
    CONSTRAINT workflow_runtime_preparations_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES public.workflow_repositories(id),
    CONSTRAINT workflow_runtime_preparations_signer_id_fkey FOREIGN KEY (signer_id) REFERENCES public.workflow_runtime_signers(id)
);

CREATE INDEX workflow_runtime_preparation_history ON public.workflow_runtime_preparations USING btree (repository_id, id DESC);

CREATE INDEX workflow_runtime_preparation_pending ON public.workflow_runtime_preparations USING btree (state, id);

CREATE TABLE public.workflow_runtime_preparation_attempts (
    id text NOT NULL,
    operation_id text NOT NULL,
    generation bigint NOT NULL,
    stage text NOT NULL,
    command_digest text NOT NULL,
    pod_uid text DEFAULT ''::text NOT NULL,
    state text NOT NULL,
    deadline_unix_ms bigint NOT NULL,
    accepted_at_unix_ms bigint DEFAULT ((EXTRACT(epoch FROM clock_timestamp()) * (1000)::numeric))::bigint NOT NULL,
    report_digest text DEFAULT ''::text NOT NULL,
    report text DEFAULT ''::text NOT NULL,
    CONSTRAINT workflow_runtime_preparation_attempts_generation_check CHECK ((generation > 0)),
    CONSTRAINT workflow_runtime_preparation_attempts_state_check CHECK ((state = ANY (ARRAY['claimed'::text, 'bound'::text, 'reported'::text, 'stopped'::text]))),
    CONSTRAINT workflow_runtime_preparation_attemp_operation_id_generation_key UNIQUE (operation_id, generation),
    CONSTRAINT workflow_runtime_preparation_attempts_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_runtime_preparation_attempts_operation_id_fkey FOREIGN KEY (operation_id) REFERENCES public.workflow_runtime_preparations(id)
);

CREATE TABLE public.workflow_runtime_preparation_receipts (
    idempotency_key text NOT NULL,
    command_id text NOT NULL,
    operation_id text NOT NULL,
    kind text NOT NULL,
    request_digest text NOT NULL,
    result text NOT NULL,
    CONSTRAINT workflow_runtime_preparation_receipts_kind_check CHECK ((kind = ANY (ARRAY['start'::text, 'cancel'::text, 'retry'::text]))),
    CONSTRAINT workflow_runtime_preparation_receipts_command_id_key UNIQUE (command_id),
    CONSTRAINT workflow_runtime_preparation_receipts_pkey PRIMARY KEY (idempotency_key),
    CONSTRAINT workflow_runtime_preparation_receipts_operation_id_fkey FOREIGN KEY (operation_id) REFERENCES public.workflow_runtime_preparations(id)
);

CREATE TABLE public.workflow_runtime_preparation_selections (
    repository_id text NOT NULL,
    operation_id text NOT NULL,
    CONSTRAINT workflow_runtime_preparation_selections_operation_id_key UNIQUE (operation_id),
    CONSTRAINT workflow_runtime_preparation_selections_pkey PRIMARY KEY (repository_id),
    CONSTRAINT workflow_runtime_preparation_selections_operation_id_fkey FOREIGN KEY (operation_id) REFERENCES public.workflow_runtime_preparations(id),
    CONSTRAINT workflow_runtime_preparation_selections_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES public.workflow_repositories(id)
);
