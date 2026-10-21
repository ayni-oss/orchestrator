-- One paid foreground call per admitted turn. Uncertainty retains its reservation.
CREATE TABLE workflow_provider_calls (
    turn_id text PRIMARY KEY REFERENCES workflow_turns(id),
    call_id text NOT NULL UNIQUE,
    state text NOT NULL CHECK (state IN ('sending','completed','failed','uncertain','cancelled')),
    evidence bytea NOT NULL CHECK (octet_length(evidence) <= 16384),
    safe_reason text NOT NULL DEFAULT '',
    reservation_settled boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE workflow_cancellation_receipts (
    conversation_id text NOT NULL REFERENCES workflow_conversations(id),
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL CHECK (octet_length(request_digest)=32),
    command_id text NOT NULL UNIQUE,
    turn_id text NOT NULL REFERENCES workflow_turns(id),
    state text NOT NULL,
    cancellation_requested boolean NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (conversation_id,idempotency_key)
);
