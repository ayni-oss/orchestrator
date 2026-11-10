-- Sessions and providers: final table definitions for fresh installations.

CREATE TABLE public.workflow_sessions (
    id text NOT NULL,
    generation bigint NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    state text NOT NULL,
    idle_timeout_seconds bigint NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    last_active_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    credential_binding text DEFAULT ''::text NOT NULL,
    CONSTRAINT workflow_sessions_generation_check CHECK ((generation > 0)),
    CONSTRAINT workflow_sessions_idle_timeout_seconds_check CHECK (((idle_timeout_seconds >= 30) AND (idle_timeout_seconds <= 86400))),
    CONSTRAINT workflow_sessions_state_check CHECK ((state = ANY (ARRAY['provisioning'::text, 'idle'::text, 'expired'::text, 'replaced'::text]))),
    CONSTRAINT workflow_sessions_conversation_id_generation_key UNIQUE (conversation_id, generation),
    CONSTRAINT workflow_sessions_pkey PRIMARY KEY (id, generation),
    CONSTRAINT workflow_sessions_workspace_id_project_id_conversation_id_fkey FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES public.workflow_conversations(workspace_id, project_id, id)
);

CREATE UNIQUE INDEX workflow_sessions_current ON public.workflow_sessions USING btree (conversation_id) WHERE (state = ANY (ARRAY['provisioning'::text, 'idle'::text]));

CREATE TABLE public.workflow_turn_sessions (
    turn_id text NOT NULL,
    session_id text NOT NULL,
    generation bigint NOT NULL,
    CONSTRAINT workflow_turn_sessions_pkey PRIMARY KEY (turn_id),
    CONSTRAINT workflow_turn_sessions_session_id_generation_fkey FOREIGN KEY (session_id, generation) REFERENCES public.workflow_sessions(id, generation),
    CONSTRAINT workflow_turn_sessions_turn_id_fkey FOREIGN KEY (turn_id) REFERENCES public.workflow_turns(id)
);

CREATE TABLE public.workflow_session_receipts (
    conversation_id text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    session_id text NOT NULL,
    generation bigint NOT NULL,
    CONSTRAINT workflow_session_receipts_pkey PRIMARY KEY (conversation_id, idempotency_key),
    CONSTRAINT workflow_session_receipts_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES public.workflow_conversations(id),
    CONSTRAINT workflow_session_receipts_session_id_generation_fkey FOREIGN KEY (session_id, generation) REFERENCES public.workflow_sessions(id, generation)
);

CREATE TABLE public.workflow_context_manifests (
    turn_id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    context_revision bigint NOT NULL,
    record_count bigint NOT NULL,
    digest text NOT NULL,
    CONSTRAINT workflow_context_manifests_context_revision_check CHECK ((context_revision > 0)),
    CONSTRAINT workflow_context_manifests_digest_check CHECK ((length(digest) = 71)),
    CONSTRAINT workflow_context_manifests_record_count_check CHECK ((record_count > 0)),
    CONSTRAINT workflow_context_manifests_pkey PRIMARY KEY (turn_id),
    CONSTRAINT workflow_context_manifests_turn_id_conversation_id_key UNIQUE (turn_id, conversation_id),
    CONSTRAINT workflow_context_manifests_turn_id_digest_key UNIQUE (turn_id, digest),
    CONSTRAINT workflow_context_manifests_turn_id_fkey FOREIGN KEY (turn_id) REFERENCES public.workflow_turns(id),
    CONSTRAINT workflow_context_manifests_workspace_id_project_id_convers_fkey FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES public.workflow_conversations(workspace_id, project_id, id)
);

CREATE TABLE public.workflow_context_manifest_messages (
    turn_id text NOT NULL,
    ordinal bigint NOT NULL,
    conversation_id text NOT NULL,
    message_id text NOT NULL,
    CONSTRAINT workflow_context_manifest_messages_ordinal_check CHECK ((ordinal > 0)),
    CONSTRAINT workflow_context_manifest_messages_pkey PRIMARY KEY (turn_id, ordinal),
    CONSTRAINT workflow_context_manifest_messages_turn_id_message_id_key UNIQUE (turn_id, message_id),
    CONSTRAINT workflow_context_manifest_messa_conversation_id_message_id_fkey FOREIGN KEY (conversation_id, message_id) REFERENCES public.workflow_messages(conversation_id, id),
    CONSTRAINT workflow_context_manifest_messages_turn_id_conversation_id_fkey FOREIGN KEY (turn_id, conversation_id) REFERENCES public.workflow_context_manifests(turn_id, conversation_id) DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE public.workflow_provider_calls (
    turn_id text NOT NULL,
    call_id text NOT NULL,
    state text NOT NULL,
    evidence bytea NOT NULL,
    safe_reason text DEFAULT ''::text NOT NULL,
    reservation_settled boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_provider_calls_evidence_check CHECK ((octet_length(evidence) <= 16384)),
    CONSTRAINT workflow_provider_calls_state_check CHECK ((state = ANY (ARRAY['sending'::text, 'completed'::text, 'failed'::text, 'uncertain'::text, 'cancelled'::text]))),
    CONSTRAINT workflow_provider_calls_call_id_key UNIQUE (call_id),
    CONSTRAINT workflow_provider_calls_pkey PRIMARY KEY (turn_id),
    CONSTRAINT workflow_provider_calls_turn_id_call_id_key UNIQUE (turn_id, call_id),
    CONSTRAINT workflow_provider_calls_turn_id_fkey FOREIGN KEY (turn_id) REFERENCES public.workflow_turns(id)
);

CREATE TABLE public.workflow_provider_uncertainty_receipts (
    conversation_id text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    command_id text NOT NULL,
    turn_id text NOT NULL,
    call_id text NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_provider_uncertainty_receipts_request_digest_check CHECK ((octet_length(request_digest) = 32)),
    CONSTRAINT workflow_provider_uncertainty_receipts_command_id_key UNIQUE (command_id),
    CONSTRAINT workflow_provider_uncertainty_receipts_pkey PRIMARY KEY (conversation_id, idempotency_key),
    CONSTRAINT workflow_provider_uncertainty_receipts_turn_id_key UNIQUE (turn_id),
    CONSTRAINT workflow_provider_uncertainty_receipts_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES public.workflow_conversations(id),
    CONSTRAINT workflow_provider_uncertainty_receipts_turn_id_fkey FOREIGN KEY (turn_id) REFERENCES public.workflow_turns(id)
);
