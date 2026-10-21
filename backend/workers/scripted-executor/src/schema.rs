pub const SCHEMA: &str = r"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS executor_identity (
    singleton integer PRIMARY KEY CHECK (singleton = 1),
    instance_id text NOT NULL,
    signing_seed blob NOT NULL CHECK (length(signing_seed) = 32)
);

CREATE TABLE IF NOT EXISTS commands (
    source text NOT NULL,
    event_id text NOT NULL,
    digest text NOT NULL,
    event_bytes blob,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    turn_id text NOT NULL,
    assignment_id text NOT NULL,
    generation integer NOT NULL CHECK (generation > 0),
    execution_binding_revision text NOT NULL,
    context_revision integer NOT NULL CHECK (context_revision > 0),
    signing_public_key blob CHECK (signing_public_key IS NULL OR length(signing_public_key) = 32),
    terminal_reserved_bytes integer NOT NULL CHECK (terminal_reserved_bytes >= 0),
    terminal_reserved_records integer NOT NULL CHECK (terminal_reserved_records IN (0, 1)),
    state text NOT NULL CHECK (state IN ('claim_pending', 'accepted', 'terminal_pending', 'rejected', 'completed')),
    PRIMARY KEY (source, event_id),
    UNIQUE (turn_id, assignment_id, generation)
);

CREATE TABLE IF NOT EXISTS reports (
    source text NOT NULL,
    event_id text NOT NULL,
    digest text NOT NULL,
    event_bytes blob,
    reserved_bytes integer NOT NULL CHECK (reserved_bytes >= 0),
    command_source text NOT NULL,
    command_event_id text NOT NULL,
    sequence integer NOT NULL CHECK (sequence IN (1, 2)),
    decision text CHECK (decision IN ('accepted', 'rejected')),
    decision_digest text,
    decision_bytes blob,
    decision_consumer_sequence integer CHECK (decision_consumer_sequence > 0),
    decision_acked integer NOT NULL DEFAULT 0 CHECK (decision_acked IN (0, 1)),
    resolved_at_seconds integer CHECK (resolved_at_seconds IS NULL OR resolved_at_seconds >= 0),
    PRIMARY KEY (source, event_id),
    UNIQUE (command_source, command_event_id, sequence),
    FOREIGN KEY (command_source, command_event_id) REFERENCES commands(source, event_id)
);

CREATE TABLE IF NOT EXISTS transport_quarantine (
    consumer_role text NOT NULL CHECK (consumer_role IN ('executor_command','executor_decision')),
    event_source text NOT NULL CHECK (length(event_source) <= 256),
    event_id text NOT NULL CHECK (length(event_id) <= 128),
    event_digest text NOT NULL CHECK (length(event_digest) = 71),
    reason text NOT NULL CHECK (reason IN ('malformed','unsupported_version','event_conflict')),
    exact_bytes blob CHECK (length(exact_bytes) <= 262144),
    consumer_sequence integer NOT NULL CHECK (consumer_sequence > 0),
    recorded_at_seconds integer NOT NULL CHECK (recorded_at_seconds >= 0),
    PRIMARY KEY (consumer_role,event_source,event_id,event_digest),
    CHECK ((reason='unsupported_version') = (exact_bytes IS NOT NULL))
);
";
