-- Invalid domain scope must not require forged domain rows or weaken foreign keys.
-- Conflicting identities retain an independent immutable digest-bound decision.
CREATE TABLE workflow_transport_rejections (
    report_source text NOT NULL CHECK (octet_length(report_source) <= 256),
    report_id text NOT NULL CHECK (octet_length(report_id) <= 128),
    report_digest text NOT NULL CHECK (octet_length(report_digest) = 71),
    exact_report_bytes bytea NOT NULL CHECK (octet_length(exact_report_bytes) <= 32768),
    decision_event_id text NOT NULL UNIQUE,
    decision_bytes bytea NOT NULL CHECK (octet_length(decision_bytes) <= 16384),
    executor_instance_id text NOT NULL CHECK (octet_length(executor_instance_id) <= 128),
    safe_reason text NOT NULL CHECK (safe_reason IN ('wrong_scope','event_conflict')),
    subject text NOT NULL CHECK (octet_length(subject) <= 256),
    state text NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','stored','transferred')),
    stream_sequence bigint NOT NULL DEFAULT 0 CHECK (stream_sequence >= 0),
    attempt_count bigint NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_attempt_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (report_source,report_id,report_digest)
);
CREATE INDEX workflow_transport_rejections_pending
    ON workflow_transport_rejections(next_attempt_at) WHERE state <> 'transferred';
