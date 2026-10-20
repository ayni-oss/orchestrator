CREATE FUNCTION workflow_ts_seconds(value timestamptz)
RETURNS bigint
LANGUAGE sql
IMMUTABLE
PARALLEL SAFE
RETURN floor(extract(epoch FROM value))::bigint;

CREATE FUNCTION workflow_ts_nanos(value timestamptz)
RETURNS bigint
LANGUAGE sql
IMMUTABLE
PARALLEL SAFE
RETURN (extract(microseconds FROM value)::bigint % 1000000) * 1000;

CREATE TABLE workflow_workspaces (
    id text PRIMARY KEY,
    display_name text NOT NULL CHECK (
        octet_length(display_name) BETWEEN 1 AND 256
        AND display_name = btrim(display_name)
    ),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    resource_version bigint NOT NULL DEFAULT 1 CHECK (resource_version > 0)
);

CREATE TABLE workflow_projects (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workflow_workspaces(id),
    display_name text NOT NULL CHECK (
        octet_length(display_name) BETWEEN 1 AND 256
        AND display_name = btrim(display_name)
    ),
    goal text NOT NULL CHECK (octet_length(goal) <= 16384),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    resource_version bigint NOT NULL DEFAULT 1 CHECK (resource_version > 0),
    UNIQUE (workspace_id, id)
);

CREATE TABLE workflow_conversations (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    title text NOT NULL CHECK (
        octet_length(title) BETWEEN 1 AND 256
        AND title = btrim(title)
    ),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    next_turn_sequence bigint NOT NULL DEFAULT 1 CHECK (next_turn_sequence > 0),
    next_history_sequence bigint NOT NULL DEFAULT 1 CHECK (next_history_sequence > 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, project_id, id),
    FOREIGN KEY (workspace_id, project_id)
        REFERENCES workflow_projects(workspace_id, id)
);

CREATE TABLE workflow_command_receipts (
    command_id text PRIMARY KEY,
    principal text NOT NULL,
    method text NOT NULL,
    parent_scope text NOT NULL,
    idempotency_key text NOT NULL CHECK (octet_length(idempotency_key) BETWEEN 1 AND 128),
    request_digest bytea NOT NULL CHECK (octet_length(request_digest) = 32),
    kind text NOT NULL,
    workspace_id text NOT NULL DEFAULT '',
    project_id text NOT NULL DEFAULT '',
    conversation_id text NOT NULL DEFAULT '',
    message_id text NOT NULL DEFAULT '',
    turn_id text NOT NULL DEFAULT '',
    resource_id text NOT NULL DEFAULT '',
    accepted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    conversation_revision bigint NOT NULL DEFAULT 0 CHECK (conversation_revision >= 0),
    resume_cursor text NOT NULL DEFAULT '',
    presentation_state text NOT NULL,
    UNIQUE (principal, method, parent_scope, idempotency_key)
);

CREATE TABLE workflow_messages (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    turn_id text,
    role text NOT NULL CHECK (role IN ('user', 'assistant')),
    content text NOT NULL CHECK (octet_length(content) BETWEEN 1 AND 16384),
    sequence bigint NOT NULL CHECK (sequence > 0),
    accepted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (conversation_id, sequence),
    UNIQUE (conversation_id, id),
    FOREIGN KEY (workspace_id, project_id, conversation_id)
        REFERENCES workflow_conversations(workspace_id, project_id, id)
);

CREATE TABLE workflow_turns (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    input_message_id text NOT NULL,
    sequence bigint NOT NULL CHECK (sequence > 0),
    execution_binding_revision text NOT NULL,
    context_revision bigint NOT NULL DEFAULT 0 CHECK (context_revision >= 0),
    assignment_id text NOT NULL DEFAULT '',
    executor_generation bigint NOT NULL DEFAULT 0 CHECK (executor_generation >= 0),
    scheduling_state text NOT NULL CHECK (
        scheduling_state IN ('waiting_for_predecessor', 'eligible')
    ),
    dispatch_state text NOT NULL CHECK (
        dispatch_state IN ('not_eligible', 'pending', 'stored', 'transferred', 'degraded')
    ),
    execution_state text NOT NULL CHECK (
        execution_state IN ('not_started', 'processing', 'completed', 'failed')
    ),
    response_message_id text,
    failure_reason text NOT NULL DEFAULT '',
    accepted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (conversation_id, sequence),
    UNIQUE (conversation_id, id),
    UNIQUE (conversation_id, input_message_id),
    FOREIGN KEY (workspace_id, project_id, conversation_id)
        REFERENCES workflow_conversations(workspace_id, project_id, id),
    FOREIGN KEY (conversation_id, input_message_id)
        REFERENCES workflow_messages(conversation_id, id)
);

ALTER TABLE workflow_messages
    ADD CONSTRAINT workflow_messages_turn_fk
    FOREIGN KEY (conversation_id, turn_id)
    REFERENCES workflow_turns(conversation_id, id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE workflow_activities (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    conversation_revision bigint NOT NULL CHECK (conversation_revision > 0),
    turn_id text NOT NULL DEFAULT '',
    stage text NOT NULL,
    presentation_state text NOT NULL,
    waiting_on text NOT NULL DEFAULT '',
    safe_reason text NOT NULL DEFAULT '',
    safe_detail text NOT NULL DEFAULT '' CHECK (octet_length(safe_detail) <= 8192),
    accepted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (conversation_id, conversation_revision),
    FOREIGN KEY (workspace_id, project_id, conversation_id)
        REFERENCES workflow_conversations(workspace_id, project_id, id)
);

CREATE TABLE workflow_updates (
    cursor bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    id text NOT NULL UNIQUE,
    activity_id text NOT NULL REFERENCES workflow_activities(id),
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    conversation_revision bigint NOT NULL CHECK (conversation_revision > 0),
    kind text NOT NULL,
    affected_resource_ids text[] NOT NULL,
    presentation_state text NOT NULL,
    waiting_on text NOT NULL DEFAULT '',
    safe_reason text NOT NULL DEFAULT '',
    accepted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (conversation_id, conversation_revision),
    FOREIGN KEY (workspace_id, project_id, conversation_id)
        REFERENCES workflow_conversations(workspace_id, project_id, id)
);

CREATE INDEX workflow_updates_replay
    ON workflow_updates (conversation_id, cursor);

CREATE TABLE workflow_dispatch_intents (
    turn_id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    eligible boolean NOT NULL,
    command_event_id text NOT NULL DEFAULT '',
    exact_command_bytes bytea,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (conversation_id, turn_id)
        REFERENCES workflow_turns(conversation_id, id),
    CHECK (
        (eligible AND command_event_id <> '' AND exact_command_bytes IS NOT NULL)
        OR (NOT eligible AND command_event_id = '' AND exact_command_bytes IS NULL)
    )
);

CREATE TABLE workflow_outbox (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    turn_id text NOT NULL,
    subject text NOT NULL,
    event_bytes bytea NOT NULL CHECK (octet_length(event_bytes) <= 262144),
    state text NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'stored', 'transferred')),
    attempt_count bigint NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    last_safe_reason text NOT NULL DEFAULT '',
    next_attempt_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (conversation_id, turn_id)
        REFERENCES workflow_turns(conversation_id, id)
);

CREATE INDEX workflow_outbox_pending
    ON workflow_outbox (next_attempt_at, created_at)
    WHERE state = 'pending';

CREATE TABLE workflow_history_snapshots (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    boundary_sequence bigint NOT NULL CHECK (boundary_sequence >= 0),
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (workspace_id, project_id, conversation_id)
        REFERENCES workflow_conversations(workspace_id, project_id, id)
);

CREATE INDEX workflow_history_snapshots_expiry
    ON workflow_history_snapshots (expires_at);
