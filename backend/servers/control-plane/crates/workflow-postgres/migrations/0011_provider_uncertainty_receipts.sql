-- Explicit operator retirement never rewrites uncertain call evidence or usage.
CREATE TABLE workflow_provider_uncertainty_receipts (
    conversation_id text NOT NULL REFERENCES workflow_conversations(id),
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL CHECK (octet_length(request_digest)=32),
    command_id text NOT NULL UNIQUE,
    turn_id text NOT NULL UNIQUE REFERENCES workflow_turns(id),
    call_id text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (conversation_id,idempotency_key)
);
