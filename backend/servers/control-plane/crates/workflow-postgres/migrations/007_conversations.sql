-- Conversations: final table definitions for fresh installations.

CREATE FUNCTION public.workflow_retain_execution_activity() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public', 'pg_temp'
    AS $$
BEGIN
    INSERT INTO public.workflow_execution_timeline
    SELECT a.id, a.workspace_id, a.project_id, a.conversation_id,
           a.conversation_revision, a.turn_id, a.stage, a.presentation_state,
           a.waiting_on, a.safe_reason, a.safe_detail, a.accepted_at,
           NEW.kind, NEW.affected_resource_ids
    FROM public.workflow_activities a
    WHERE a.id = NEW.activity_id
      AND a.workspace_id = NEW.workspace_id AND a.project_id = NEW.project_id
      AND a.conversation_id = NEW.conversation_id
      AND a.conversation_revision = NEW.conversation_revision;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'execution activity scope mismatch';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.workflow_ts_nanos(value timestamp with time zone) RETURNS bigint
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    RETURN (((EXTRACT(microseconds FROM value))::bigint % (1000000)::bigint) * 1000);

CREATE FUNCTION public.workflow_ts_seconds(value timestamp with time zone) RETURNS bigint
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    RETURN (floor(EXTRACT(epoch FROM value)))::bigint;

REVOKE ALL ON FUNCTION public.workflow_ts_seconds(timestamp with time zone),
    public.workflow_ts_nanos(timestamp with time zone),
    public.workflow_retain_execution_activity() FROM PUBLIC;

CREATE TABLE public.workflow_conversations (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    title text NOT NULL,
    revision bigint DEFAULT 1 NOT NULL,
    next_turn_sequence bigint DEFAULT 1 NOT NULL,
    next_history_sequence bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_conversations_next_history_sequence_check CHECK ((next_history_sequence > 0)),
    CONSTRAINT workflow_conversations_next_turn_sequence_check CHECK ((next_turn_sequence > 0)),
    CONSTRAINT workflow_conversations_revision_check CHECK ((revision > 0)),
    CONSTRAINT workflow_conversations_title_check CHECK ((((octet_length(title) >= 1) AND (octet_length(title) <= 256)) AND (title = btrim(title)))),
    CONSTRAINT workflow_conversations_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_conversations_workspace_id_project_id_id_key UNIQUE (workspace_id, project_id, id),
    CONSTRAINT workflow_conversations_workspace_id_project_id_fkey FOREIGN KEY (workspace_id, project_id) REFERENCES public.workflow_projects(workspace_id, id)
);

CREATE TABLE public.workflow_activities (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    conversation_revision bigint NOT NULL,
    turn_id text DEFAULT ''::text NOT NULL,
    stage text NOT NULL,
    presentation_state text NOT NULL,
    waiting_on text DEFAULT ''::text NOT NULL,
    safe_reason text DEFAULT ''::text NOT NULL,
    safe_detail text DEFAULT ''::text NOT NULL,
    accepted_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_activities_conversation_revision_check CHECK ((conversation_revision > 0)),
    CONSTRAINT workflow_activities_safe_detail_check CHECK ((octet_length(safe_detail) <= 8192)),
    CONSTRAINT workflow_activities_conversation_id_conversation_revision_key UNIQUE (conversation_id, conversation_revision),
    CONSTRAINT workflow_activities_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_activities_workspace_id_project_id_conversation_i_fkey FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES public.workflow_conversations(workspace_id, project_id, id)
);

CREATE TABLE public.workflow_updates (
    cursor bigint GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.workflow_updates_cursor_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
) NOT NULL,
    id text NOT NULL,
    activity_id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    conversation_revision bigint NOT NULL,
    kind text NOT NULL,
    affected_resource_ids text[] NOT NULL,
    presentation_state text NOT NULL,
    waiting_on text DEFAULT ''::text NOT NULL,
    safe_reason text DEFAULT ''::text NOT NULL,
    accepted_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_updates_conversation_revision_check CHECK ((conversation_revision > 0)),
    CONSTRAINT workflow_updates_conversation_id_conversation_revision_key UNIQUE (conversation_id, conversation_revision),
    CONSTRAINT workflow_updates_id_key UNIQUE (id),
    CONSTRAINT workflow_updates_pkey PRIMARY KEY (cursor),
    CONSTRAINT workflow_updates_activity_id_fkey FOREIGN KEY (activity_id) REFERENCES public.workflow_activities(id),
    CONSTRAINT workflow_updates_workspace_id_project_id_conversation_id_fkey FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES public.workflow_conversations(workspace_id, project_id, id)
);

CREATE INDEX workflow_updates_replay ON public.workflow_updates USING btree (conversation_id, cursor);

CREATE INDEX workflow_updates_retention ON public.workflow_updates USING btree (accepted_at, conversation_id, cursor);

CREATE TRIGGER workflow_retain_execution_activity AFTER INSERT ON public.workflow_updates FOR EACH ROW EXECUTE FUNCTION public.workflow_retain_execution_activity();

CREATE TABLE public.workflow_execution_timeline (
    activity_id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    conversation_revision bigint NOT NULL,
    turn_id text NOT NULL,
    stage text NOT NULL,
    presentation_state text NOT NULL,
    waiting_on text NOT NULL,
    safe_reason text NOT NULL,
    safe_detail text NOT NULL,
    accepted_at timestamp with time zone NOT NULL,
    kind text NOT NULL,
    affected_resource_ids text[] NOT NULL,
    CONSTRAINT workflow_execution_timeline_conversation_revision_check CHECK ((conversation_revision > 0)),
    CONSTRAINT workflow_execution_timeline_safe_detail_check CHECK ((octet_length(safe_detail) <= 8192)),
    CONSTRAINT workflow_execution_timeline_conversation_id_conversation_re_key UNIQUE (conversation_id, conversation_revision),
    CONSTRAINT workflow_execution_timeline_pkey PRIMARY KEY (activity_id),
    CONSTRAINT workflow_execution_timeline_workspace_id_project_id_conver_fkey FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES public.workflow_conversations(workspace_id, project_id, id)
);

CREATE INDEX workflow_execution_timeline_resources ON public.workflow_execution_timeline USING gin (affected_resource_ids);

CREATE TABLE public.workflow_command_receipts (
    command_id text NOT NULL,
    principal text NOT NULL,
    method text NOT NULL,
    parent_scope text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    kind text NOT NULL,
    workspace_id text DEFAULT ''::text NOT NULL,
    project_id text DEFAULT ''::text NOT NULL,
    conversation_id text DEFAULT ''::text NOT NULL,
    message_id text DEFAULT ''::text NOT NULL,
    turn_id text DEFAULT ''::text NOT NULL,
    resource_id text DEFAULT ''::text NOT NULL,
    accepted_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    conversation_revision bigint DEFAULT 0 NOT NULL,
    resume_cursor text DEFAULT ''::text NOT NULL,
    presentation_state text NOT NULL,
    traceparent text DEFAULT COALESCE(current_setting('ayni.traceparent'::text, true), ''::text) NOT NULL,
    CONSTRAINT workflow_command_receipts_conversation_revision_check CHECK ((conversation_revision >= 0)),
    CONSTRAINT workflow_command_receipts_idempotency_key_check CHECK (((octet_length(idempotency_key) >= 1) AND (octet_length(idempotency_key) <= 128))),
    CONSTRAINT workflow_command_receipts_request_digest_check CHECK ((octet_length(request_digest) = 32)),
    CONSTRAINT workflow_command_receipts_pkey PRIMARY KEY (command_id),
    CONSTRAINT workflow_command_receipts_principal_method_parent_scope_ide_key UNIQUE (principal, method, parent_scope, idempotency_key)
);

CREATE INDEX workflow_command_receipts_turn_trace ON public.workflow_command_receipts USING btree (conversation_id, turn_id) WHERE (kind = 'send_message'::text);

CREATE TABLE public.workflow_history_snapshots (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    boundary_sequence bigint NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_history_snapshots_boundary_sequence_check CHECK ((boundary_sequence >= 0)),
    CONSTRAINT workflow_history_snapshots_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_history_snapshots_workspace_id_project_id_convers_fkey FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES public.workflow_conversations(workspace_id, project_id, id)
);

CREATE INDEX workflow_history_snapshots_expiry ON public.workflow_history_snapshots USING btree (expires_at);

CREATE TABLE public.workflow_watch_watermarks (
    conversation_id text NOT NULL,
    pruned_through_cursor bigint DEFAULT 0 NOT NULL,
    CONSTRAINT workflow_watch_watermarks_pruned_through_cursor_check CHECK ((pruned_through_cursor >= 0)),
    CONSTRAINT workflow_watch_watermarks_pkey PRIMARY KEY (conversation_id),
    CONSTRAINT workflow_watch_watermarks_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES public.workflow_conversations(id)
);

CREATE TABLE public.workflow_messages (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    turn_id text,
    role text NOT NULL,
    content text NOT NULL,
    sequence bigint NOT NULL,
    accepted_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_messages_content_check CHECK (((octet_length(content) >= 1) AND (octet_length(content) <= 16384))),
    CONSTRAINT workflow_messages_role_check CHECK ((role = ANY (ARRAY['user'::text, 'assistant'::text]))),
    CONSTRAINT workflow_messages_sequence_check CHECK ((sequence > 0)),
    CONSTRAINT workflow_messages_conversation_id_id_key UNIQUE (conversation_id, id),
    CONSTRAINT workflow_messages_conversation_id_sequence_key UNIQUE (conversation_id, sequence),
    CONSTRAINT workflow_messages_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_messages_workspace_id_project_id_conversation_id_fkey FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES public.workflow_conversations(workspace_id, project_id, id)
);

CREATE TABLE public.workflow_turns (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    input_message_id text NOT NULL,
    sequence bigint NOT NULL,
    execution_binding_revision text NOT NULL,
    context_revision bigint DEFAULT 0 NOT NULL,
    assignment_id text DEFAULT ''::text NOT NULL,
    executor_generation bigint DEFAULT 0 NOT NULL,
    scheduling_state text NOT NULL,
    dispatch_state text NOT NULL,
    execution_state text NOT NULL,
    response_message_id text,
    failure_reason text DEFAULT ''::text NOT NULL,
    accepted_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    updated_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_turns_context_revision_check CHECK ((context_revision >= 0)),
    CONSTRAINT workflow_turns_dispatch_state_check CHECK ((dispatch_state = ANY (ARRAY['not_eligible'::text, 'pending'::text, 'stored'::text, 'transferred'::text, 'degraded'::text]))),
    CONSTRAINT workflow_turns_execution_state_check CHECK ((execution_state = ANY (ARRAY['not_started'::text, 'processing'::text, 'completed'::text, 'failed'::text]))),
    CONSTRAINT workflow_turns_executor_generation_check CHECK ((executor_generation >= 0)),
    CONSTRAINT workflow_turns_scheduling_state_check CHECK ((scheduling_state = ANY (ARRAY['waiting_for_predecessor'::text, 'eligible'::text]))),
    CONSTRAINT workflow_turns_sequence_check CHECK ((sequence > 0)),
    CONSTRAINT workflow_turns_conversation_id_id_key UNIQUE (conversation_id, id),
    CONSTRAINT workflow_turns_conversation_id_input_message_id_key UNIQUE (conversation_id, input_message_id),
    CONSTRAINT workflow_turns_conversation_id_sequence_key UNIQUE (conversation_id, sequence),
    CONSTRAINT workflow_turns_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_turns_workspace_id_id_key UNIQUE (workspace_id, id),
    CONSTRAINT workflow_turns_conversation_id_input_message_id_fkey FOREIGN KEY (conversation_id, input_message_id) REFERENCES public.workflow_messages(conversation_id, id),
    CONSTRAINT workflow_turns_workspace_id_project_id_conversation_id_fkey FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES public.workflow_conversations(workspace_id, project_id, id)
);

CREATE TABLE public.workflow_execution_progress (
    turn_id text NOT NULL,
    session_id text NOT NULL,
    generation bigint NOT NULL,
    sequence bigint NOT NULL,
    round integer NOT NULL,
    call_id text NOT NULL,
    stage text NOT NULL,
    tool_name text NOT NULL,
    tool_invocation_id text NOT NULL,
    provisional_text text NOT NULL,
    stage_observed_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    observed_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_execution_progress_call_id_check CHECK ((octet_length(call_id) <= 128)),
    CONSTRAINT workflow_execution_progress_generation_check CHECK ((generation > 0)),
    CONSTRAINT workflow_execution_progress_provisional_text_check CHECK ((octet_length(provisional_text) <= 16384)),
    CONSTRAINT workflow_execution_progress_round_check CHECK (((round >= 0) AND (round <= 8))),
    CONSTRAINT workflow_execution_progress_sequence_check CHECK ((sequence > 0)),
    CONSTRAINT workflow_execution_progress_stage_check CHECK ((stage = ANY (ARRAY['preparation'::text, 'model'::text, 'tool'::text, 'scripted'::text]))),
    CONSTRAINT workflow_execution_progress_tool_invocation_id_check CHECK ((octet_length(tool_invocation_id) <= 128)),
    CONSTRAINT workflow_execution_progress_tool_name_check CHECK ((octet_length(tool_name) <= 128)),
    CONSTRAINT workflow_execution_progress_pkey PRIMARY KEY (turn_id),
    CONSTRAINT workflow_execution_progress_turn_id_fkey FOREIGN KEY (turn_id) REFERENCES public.workflow_turns(id)
);

CREATE TABLE public.workflow_cancellation_receipts (
    conversation_id text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    command_id text NOT NULL,
    turn_id text NOT NULL,
    state text NOT NULL,
    cancellation_requested boolean NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_cancellation_receipts_request_digest_check CHECK ((octet_length(request_digest) = 32)),
    CONSTRAINT workflow_cancellation_receipts_command_id_key UNIQUE (command_id),
    CONSTRAINT workflow_cancellation_receipts_pkey PRIMARY KEY (conversation_id, idempotency_key),
    CONSTRAINT workflow_cancellation_receipts_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES public.workflow_conversations(id),
    CONSTRAINT workflow_cancellation_receipts_turn_id_fkey FOREIGN KEY (turn_id) REFERENCES public.workflow_turns(id)
);

CREATE TABLE public.workflow_turn_inputs (
    turn_id text NOT NULL,
    workspace_id text NOT NULL,
    configuration_revision_id text NOT NULL,
    credential_revision text DEFAULT ''::text NOT NULL,
    CONSTRAINT workflow_turn_inputs_pkey PRIMARY KEY (turn_id),
    CONSTRAINT workflow_turn_inputs_workspace_id_configuration_revision_i_fkey FOREIGN KEY (workspace_id, configuration_revision_id) REFERENCES public.workflow_agent_configurations(workspace_id, revision_id),
    CONSTRAINT workflow_turn_inputs_workspace_id_turn_id_fkey FOREIGN KEY (workspace_id, turn_id) REFERENCES public.workflow_turns(workspace_id, id)
);

-- Complete cyclic references after both tables exist. These constraints
-- belong to this layer and are installed in the same migration transaction.

ALTER TABLE public.workflow_messages
    ADD CONSTRAINT workflow_messages_turn_fk FOREIGN KEY (conversation_id, turn_id) REFERENCES public.workflow_turns(conversation_id, id) DEFERRABLE INITIALLY DEFERRED;
