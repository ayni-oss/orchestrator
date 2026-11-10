-- Outcome assessments: final table definitions for fresh installations.

CREATE TABLE public.workflow_assessment_authorizations (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    idempotency_key text NOT NULL,
    configuration_revision_id text NOT NULL,
    max_assessments bigint NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_assessment_authorizations_max_assessments_check CHECK (((max_assessments >= 0) AND (max_assessments <= 10))),
    CONSTRAINT workflow_assessment_authoriza_conversation_id_idempotency_k_key UNIQUE (conversation_id, idempotency_key),
    CONSTRAINT workflow_assessment_authoriza_workspace_id_project_id_conve_key UNIQUE (workspace_id, project_id, conversation_id, id),
    CONSTRAINT workflow_assessment_authorizations_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_assessment_authoriza_workspace_id_configuration_r_fkey FOREIGN KEY (workspace_id, configuration_revision_id) REFERENCES public.workflow_agent_configurations(workspace_id, revision_id),
    CONSTRAINT workflow_assessment_authoriza_workspace_id_project_id_conv_fkey FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES public.workflow_conversations(workspace_id, project_id, id)
);

CREATE INDEX workflow_assessment_authorizations_origin ON public.workflow_assessment_authorizations USING btree (conversation_id, created_at DESC, id DESC);

CREATE TABLE public.workflow_assessment_scans (
    authorization_id text NOT NULL,
    observed_cursor bigint NOT NULL,
    CONSTRAINT workflow_assessment_scans_observed_cursor_check CHECK ((observed_cursor >= 0)),
    CONSTRAINT workflow_assessment_scans_pkey PRIMARY KEY (authorization_id, observed_cursor),
    CONSTRAINT workflow_assessment_scans_authorization_id_fkey FOREIGN KEY (authorization_id) REFERENCES public.workflow_assessment_authorizations(id)
);

CREATE TABLE public.workflow_outcome_assessments (
    id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    authorization_id text NOT NULL,
    snapshot_digest text NOT NULL,
    snapshot bytea NOT NULL,
    turn_id text NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT workflow_outcome_assessments_snapshot_check CHECK (((octet_length(snapshot) >= 1) AND (octet_length(snapshot) <= 131072))),
    CONSTRAINT workflow_outcome_assessments_authorization_id_snapshot_dige_key UNIQUE (authorization_id, snapshot_digest),
    CONSTRAINT workflow_outcome_assessments_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_outcome_assessments_turn_id_key UNIQUE (turn_id),
    CONSTRAINT workflow_outcome_assessments_turn_id_fkey FOREIGN KEY (turn_id) REFERENCES public.workflow_turns(id),
    CONSTRAINT workflow_outcome_assessments_workspace_id_project_id_conve_fkey FOREIGN KEY (workspace_id, project_id, conversation_id, authorization_id) REFERENCES public.workflow_assessment_authorizations(workspace_id, project_id, conversation_id, id)
);
