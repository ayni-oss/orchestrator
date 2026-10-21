-- Transport poison is separate from accepted workflow authority. Future versions
-- retain exact bytes until successful replay; malformed content retains digests only.
CREATE TABLE workflow_transport_quarantine (
    consumer_role text NOT NULL CHECK (consumer_role = 'application_report'),
    event_source text NOT NULL CHECK (octet_length(event_source) <= 256),
    event_id text NOT NULL CHECK (octet_length(event_id) <= 128),
    event_digest text NOT NULL CHECK (octet_length(event_digest) = 71),
    reason text NOT NULL CHECK (reason IN ('malformed', 'unsupported_version', 'event_conflict')),
    exact_bytes bytea CHECK (octet_length(exact_bytes) <= 32768),
    recorded_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (consumer_role, event_source, event_id, event_digest),
    CHECK ((reason = 'unsupported_version') = (exact_bytes IS NOT NULL))
);
CREATE INDEX workflow_transport_quarantine_retention
    ON workflow_transport_quarantine (recorded_at) WHERE exact_bytes IS NULL;

-- A terminal report can arrive before its signed ownership claim. It remains
-- non-authoritative until normal claim fencing and terminal verification succeed.
CREATE TABLE workflow_pending_reports (
    report_source text NOT NULL CHECK (octet_length(report_source) <= 256),
    report_id text NOT NULL CHECK (octet_length(report_id) <= 128),
    report_digest text NOT NULL CHECK (octet_length(report_digest) = 71),
    exact_bytes bytea NOT NULL CHECK (octet_length(exact_bytes) <= 32768),
    turn_id text NOT NULL REFERENCES workflow_turns(id),
    recorded_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (report_source, report_id)
);
CREATE INDEX workflow_pending_reports_turn ON workflow_pending_reports (turn_id);
