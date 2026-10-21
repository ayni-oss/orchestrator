-- Session generations are application identities within the v1 contract.
CREATE TABLE workflow_sessions (
    id text NOT NULL,
    generation bigint NOT NULL CHECK (generation > 0),
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    state text NOT NULL CHECK (state IN ('provisioning', 'idle', 'expired', 'replaced')),
    idle_timeout_seconds bigint NOT NULL CHECK (idle_timeout_seconds BETWEEN 30 AND 86400),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    last_active_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (id, generation),
    UNIQUE (conversation_id, generation),
    FOREIGN KEY (workspace_id, project_id, conversation_id)
        REFERENCES workflow_conversations (workspace_id, project_id, id)
);
CREATE UNIQUE INDEX workflow_sessions_current ON workflow_sessions(conversation_id)
    WHERE state IN ('provisioning', 'idle');
CREATE TABLE workflow_turn_sessions (
    turn_id text PRIMARY KEY REFERENCES workflow_turns(id),
    session_id text NOT NULL,
    generation bigint NOT NULL,
    FOREIGN KEY (session_id, generation) REFERENCES workflow_sessions(id, generation)
);
CREATE TABLE workflow_session_receipts (
    conversation_id text NOT NULL REFERENCES workflow_conversations(id),
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    session_id text NOT NULL,
    generation bigint NOT NULL,
    PRIMARY KEY (conversation_id, idempotency_key),
    FOREIGN KEY (session_id, generation) REFERENCES workflow_sessions(id, generation)
);
