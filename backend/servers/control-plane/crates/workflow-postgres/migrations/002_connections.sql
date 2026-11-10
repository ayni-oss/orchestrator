-- Connections: final table definitions for fresh installations.

CREATE TABLE public.workflow_connections (
    id text NOT NULL,
    display_name text NOT NULL,
    provider text NOT NULL,
    capability text NOT NULL,
    revision bigint NOT NULL,
    enabled boolean NOT NULL,
    app_id text NOT NULL,
    active_credential_revision text DEFAULT ''::text NOT NULL,
    candidate_credential_revision text DEFAULT ''::text NOT NULL,
    status text NOT NULL,
    safe_reason text DEFAULT ''::text NOT NULL,
    settings text DEFAULT '{}'::text NOT NULL,
    models text DEFAULT '[]'::text NOT NULL,
    CONSTRAINT workflow_connections_display_name_check CHECK (((octet_length(display_name) >= 1) AND (octet_length(display_name) <= 120))),
    CONSTRAINT workflow_connections_provider_capability_check CHECK ((((provider = 'github'::text) AND (capability = ANY (ARRAY['source_control'::text, 'container_registry'::text]))) OR ((provider = 'openai'::text) AND (capability = 'llm'::text)))),
    CONSTRAINT workflow_connections_revision_check CHECK ((revision > 0)),
    CONSTRAINT workflow_connections_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'ready'::text, 'failed'::text, 'disabled'::text]))),
    CONSTRAINT workflow_connections_pkey PRIMARY KEY (id)
);

CREATE TABLE public.workflow_connection_credentials (
    revision_id text NOT NULL,
    connection_id text NOT NULL,
    value text NOT NULL,
    CONSTRAINT workflow_connection_credentials_value_check CHECK (((octet_length(value) >= 1) AND (octet_length(value) <= 32768))),
    CONSTRAINT workflow_connection_credentials_pkey PRIMARY KEY (revision_id),
    CONSTRAINT workflow_connection_credentials_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES public.workflow_connections(id)
);

CREATE TABLE public.workflow_connection_operations (
    id text NOT NULL,
    idempotency_key text NOT NULL,
    connection_id text NOT NULL,
    kind text NOT NULL,
    safe_request text NOT NULL,
    credential_revision text DEFAULT ''::text NOT NULL,
    connection_revision bigint NOT NULL,
    state text NOT NULL,
    safe_reason text DEFAULT ''::text NOT NULL,
    result text NOT NULL,
    accepted_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_connection_operations_connection_revision_check CHECK ((connection_revision > 0)),
    CONSTRAINT workflow_connection_operations_kind_check CHECK ((kind = ANY (ARRAY['save'::text, 'enable'::text]))),
    CONSTRAINT workflow_connection_operations_state_check CHECK ((state = ANY (ARRAY['accepted'::text, 'completed'::text, 'failed'::text, 'superseded'::text]))),
    CONSTRAINT workflow_connection_operations_idempotency_key_key UNIQUE (idempotency_key),
    CONSTRAINT workflow_connection_operations_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_connection_operations_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES public.workflow_connections(id)
);

CREATE INDEX workflow_connection_operations_history ON public.workflow_connection_operations USING btree (connection_id, id);
