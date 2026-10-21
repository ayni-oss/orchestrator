-- Immutable executor observations and their authoritative application decisions.
-- Exact report bytes remain available for duplicate/conflict detection even after
-- the corresponding targeted decision has been broker-stored.
CREATE TABLE workflow_report_receipts (
    report_source text NOT NULL,
    report_id text NOT NULL,
    report_digest text NOT NULL CHECK (octet_length(report_digest) = 71),
    exact_report_bytes bytea NOT NULL CHECK (octet_length(exact_report_bytes) <= 32768),
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    turn_id text NOT NULL,
    assignment_id text NOT NULL,
    executor_generation bigint NOT NULL CHECK (executor_generation > 0),
    report_sequence bigint NOT NULL CHECK (report_sequence IN (1, 2)),
    executor_instance_id text NOT NULL,
    decision text NOT NULL CHECK (decision IN ('accepted', 'rejected')),
    safe_reason text NOT NULL DEFAULT '',
    decision_event_id text NOT NULL UNIQUE,
    decision_bytes bytea NOT NULL CHECK (octet_length(decision_bytes) <= 16384),
    accepted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (report_source, report_id),
    FOREIGN KEY (workspace_id, project_id, conversation_id)
        REFERENCES workflow_conversations(workspace_id, project_id, id),
    FOREIGN KEY (conversation_id, turn_id)
        REFERENCES workflow_turns(conversation_id, id)
);

CREATE INDEX workflow_report_receipts_turn
    ON workflow_report_receipts (conversation_id, turn_id, report_sequence);

CREATE UNIQUE INDEX workflow_report_receipts_accepted_sequence
    ON workflow_report_receipts (turn_id, assignment_id, executor_generation, report_sequence)
    WHERE decision = 'accepted';
