-- Transport: final table definitions for fresh installations.

CREATE TABLE public.workflow_dispatch_intents (
    turn_id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    eligible boolean NOT NULL,
    command_event_id text DEFAULT ''::text NOT NULL,
    exact_command_bytes bytea,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    updated_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_dispatch_intents_check CHECK (((eligible AND (command_event_id <> ''::text) AND (exact_command_bytes IS NOT NULL)) OR ((NOT eligible) AND (command_event_id = ''::text) AND (exact_command_bytes IS NULL)))),
    CONSTRAINT workflow_dispatch_intents_pkey PRIMARY KEY (turn_id),
    CONSTRAINT workflow_dispatch_intents_conversation_id_turn_id_fkey FOREIGN KEY (conversation_id, turn_id) REFERENCES public.workflow_turns(conversation_id, id)
);

CREATE TABLE public.workflow_outbox (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    turn_id text NOT NULL,
    subject text NOT NULL,
    event_bytes bytea NOT NULL,
    state text DEFAULT 'pending'::text NOT NULL,
    attempt_count bigint DEFAULT 0 NOT NULL,
    last_safe_reason text DEFAULT ''::text NOT NULL,
    next_attempt_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    updated_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    stream_sequence bigint DEFAULT 0 NOT NULL,
    CONSTRAINT workflow_outbox_attempt_count_check CHECK ((attempt_count >= 0)),
    CONSTRAINT workflow_outbox_event_bytes_check CHECK ((octet_length(event_bytes) <= 262144)),
    CONSTRAINT workflow_outbox_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'stored'::text, 'transferred'::text]))),
    CONSTRAINT workflow_outbox_stream_sequence_check CHECK ((stream_sequence >= 0)),
    CONSTRAINT workflow_outbox_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_outbox_conversation_id_turn_id_fkey FOREIGN KEY (conversation_id, turn_id) REFERENCES public.workflow_turns(conversation_id, id)
);

CREATE INDEX workflow_outbox_pending ON public.workflow_outbox USING btree (next_attempt_at, created_at) WHERE (state = 'pending'::text);

CREATE TABLE public.workflow_report_receipts (
    report_source text NOT NULL,
    report_id text NOT NULL,
    report_digest text NOT NULL,
    exact_report_bytes bytea NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    turn_id text NOT NULL,
    assignment_id text NOT NULL,
    executor_generation bigint NOT NULL,
    report_sequence bigint NOT NULL,
    executor_instance_id text NOT NULL,
    decision text NOT NULL,
    safe_reason text DEFAULT ''::text NOT NULL,
    decision_event_id text NOT NULL,
    decision_bytes bytea NOT NULL,
    accepted_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_report_receipts_decision_bytes_check CHECK ((octet_length(decision_bytes) <= 16384)),
    CONSTRAINT workflow_report_receipts_decision_check CHECK ((decision = ANY (ARRAY['accepted'::text, 'rejected'::text]))),
    CONSTRAINT workflow_report_receipts_exact_report_bytes_check CHECK ((octet_length(exact_report_bytes) <= 131072)),
    CONSTRAINT workflow_report_receipts_executor_generation_check CHECK ((executor_generation > 0)),
    CONSTRAINT workflow_report_receipts_report_digest_check CHECK ((octet_length(report_digest) = 71)),
    CONSTRAINT workflow_report_receipts_report_sequence_check CHECK ((report_sequence = ANY (ARRAY[(1)::bigint, (2)::bigint]))),
    CONSTRAINT workflow_report_receipts_decision_event_id_key UNIQUE (decision_event_id),
    CONSTRAINT workflow_report_receipts_pkey PRIMARY KEY (report_source, report_id),
    CONSTRAINT workflow_report_receipts_report_source_report_id_report_dig_key UNIQUE (report_source, report_id, report_digest, workspace_id, project_id, conversation_id, turn_id, assignment_id, executor_generation),
    CONSTRAINT workflow_report_receipts_conversation_id_turn_id_fkey FOREIGN KEY (conversation_id, turn_id) REFERENCES public.workflow_turns(conversation_id, id),
    CONSTRAINT workflow_report_receipts_workspace_id_project_id_conversat_fkey FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES public.workflow_conversations(workspace_id, project_id, id)
);

CREATE UNIQUE INDEX workflow_report_receipts_accepted_sequence ON public.workflow_report_receipts USING btree (turn_id, assignment_id, executor_generation, report_sequence) WHERE (decision = 'accepted'::text);

CREATE INDEX workflow_report_receipts_turn ON public.workflow_report_receipts USING btree (conversation_id, turn_id, report_sequence);

CREATE TABLE public.workflow_pending_reports (
    report_source text NOT NULL,
    report_id text NOT NULL,
    report_digest text NOT NULL,
    exact_bytes bytea NOT NULL,
    turn_id text NOT NULL,
    recorded_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_pending_reports_exact_bytes_check CHECK ((octet_length(exact_bytes) <= 131072)),
    CONSTRAINT workflow_pending_reports_report_digest_check CHECK ((octet_length(report_digest) = 71)),
    CONSTRAINT workflow_pending_reports_report_id_check CHECK ((octet_length(report_id) <= 128)),
    CONSTRAINT workflow_pending_reports_report_source_check CHECK ((octet_length(report_source) <= 256)),
    CONSTRAINT workflow_pending_reports_pkey PRIMARY KEY (report_source, report_id),
    CONSTRAINT workflow_pending_reports_turn_id_fkey FOREIGN KEY (turn_id) REFERENCES public.workflow_turns(id)
);

CREATE INDEX workflow_pending_reports_turn ON public.workflow_pending_reports USING btree (turn_id);

CREATE TABLE public.workflow_transport_quarantine (
    consumer_role text NOT NULL,
    event_source text NOT NULL,
    event_id text NOT NULL,
    event_digest text NOT NULL,
    reason text NOT NULL,
    exact_bytes bytea,
    recorded_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_transport_quarantine_check CHECK (((reason = 'unsupported_version'::text) = (exact_bytes IS NOT NULL))),
    CONSTRAINT workflow_transport_quarantine_consumer_role_check CHECK ((consumer_role = 'application_report'::text)),
    CONSTRAINT workflow_transport_quarantine_event_digest_check CHECK ((octet_length(event_digest) = 71)),
    CONSTRAINT workflow_transport_quarantine_event_id_check CHECK ((octet_length(event_id) <= 128)),
    CONSTRAINT workflow_transport_quarantine_event_source_check CHECK ((octet_length(event_source) <= 256)),
    CONSTRAINT workflow_transport_quarantine_exact_bytes_check CHECK ((octet_length(exact_bytes) <= 131072)),
    CONSTRAINT workflow_transport_quarantine_reason_check CHECK ((reason = ANY (ARRAY['malformed'::text, 'unsupported_version'::text, 'event_conflict'::text]))),
    CONSTRAINT workflow_transport_quarantine_pkey PRIMARY KEY (consumer_role, event_source, event_id, event_digest)
);

CREATE INDEX workflow_transport_quarantine_retention ON public.workflow_transport_quarantine USING btree (recorded_at) WHERE (exact_bytes IS NULL);

CREATE TABLE public.workflow_transport_rejections (
    report_source text NOT NULL,
    report_id text NOT NULL,
    report_digest text NOT NULL,
    exact_report_bytes bytea NOT NULL,
    decision_event_id text NOT NULL,
    decision_bytes bytea NOT NULL,
    executor_instance_id text NOT NULL,
    safe_reason text NOT NULL,
    subject text NOT NULL,
    state text DEFAULT 'pending'::text NOT NULL,
    stream_sequence bigint DEFAULT 0 NOT NULL,
    attempt_count bigint DEFAULT 0 NOT NULL,
    next_attempt_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    updated_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_transport_rejections_attempt_count_check CHECK ((attempt_count >= 0)),
    CONSTRAINT workflow_transport_rejections_decision_bytes_check CHECK ((octet_length(decision_bytes) <= 16384)),
    CONSTRAINT workflow_transport_rejections_exact_report_bytes_check CHECK ((octet_length(exact_report_bytes) <= 131072)),
    CONSTRAINT workflow_transport_rejections_executor_instance_id_check CHECK ((octet_length(executor_instance_id) <= 128)),
    CONSTRAINT workflow_transport_rejections_report_digest_check CHECK ((octet_length(report_digest) = 71)),
    CONSTRAINT workflow_transport_rejections_report_id_check CHECK ((octet_length(report_id) <= 128)),
    CONSTRAINT workflow_transport_rejections_report_source_check CHECK ((octet_length(report_source) <= 256)),
    CONSTRAINT workflow_transport_rejections_safe_reason_check CHECK ((safe_reason = ANY (ARRAY['wrong_scope'::text, 'event_conflict'::text]))),
    CONSTRAINT workflow_transport_rejections_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'stored'::text, 'transferred'::text]))),
    CONSTRAINT workflow_transport_rejections_stream_sequence_check CHECK ((stream_sequence >= 0)),
    CONSTRAINT workflow_transport_rejections_subject_check CHECK ((octet_length(subject) <= 256)),
    CONSTRAINT workflow_transport_rejections_decision_event_id_key UNIQUE (decision_event_id),
    CONSTRAINT workflow_transport_rejections_pkey PRIMARY KEY (report_source, report_id, report_digest)
);

CREATE INDEX workflow_transport_rejections_pending ON public.workflow_transport_rejections USING btree (next_attempt_at) WHERE (state <> 'transferred'::text);

CREATE TABLE public.workflow_broker_capacity (
    lane text NOT NULL,
    retained_messages bigint NOT NULL,
    retained_bytes bigint NOT NULL,
    observed_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_broker_capacity_lane_check CHECK ((lane = ANY (ARRAY['command'::text, 'report'::text, 'decision'::text]))),
    CONSTRAINT workflow_broker_capacity_retained_bytes_check CHECK ((retained_bytes >= 0)),
    CONSTRAINT workflow_broker_capacity_retained_messages_check CHECK ((retained_messages >= 0)),
    CONSTRAINT workflow_broker_capacity_pkey PRIMARY KEY (lane)
);
