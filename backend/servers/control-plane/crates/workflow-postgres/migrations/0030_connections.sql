-- Credentials are deliberately separate from safe projections and receipts.
-- Hosting owns storage/backup encryption. Workers never receive database access.
CREATE TABLE workflow_connections (
    id text PRIMARY KEY,
    display_name text NOT NULL CHECK (octet_length(display_name) BETWEEN 1 AND 120),
    provider text NOT NULL CHECK (provider = 'github'),
    capability text NOT NULL CHECK (capability = 'source_control'),
    revision bigint NOT NULL CHECK (revision > 0),
    enabled boolean NOT NULL,
    app_id text NOT NULL,
    active_credential_revision text NOT NULL DEFAULT '',
    candidate_credential_revision text NOT NULL DEFAULT '',
    status text NOT NULL CHECK (status IN ('pending','ready','failed','disabled')),
    safe_reason text NOT NULL DEFAULT ''
);
CREATE TABLE workflow_connection_credentials (
    revision_id text PRIMARY KEY,
    connection_id text NOT NULL REFERENCES workflow_connections(id),
    value text NOT NULL CHECK (octet_length(value) BETWEEN 1 AND 32768)
);
REVOKE ALL ON workflow_connection_credentials FROM PUBLIC;
CREATE TABLE workflow_connection_operations (
    id text PRIMARY KEY,
    idempotency_key text NOT NULL UNIQUE,
    connection_id text NOT NULL REFERENCES workflow_connections(id),
    kind text NOT NULL CHECK (kind IN ('save','enable')),
    safe_request text NOT NULL,
    credential_revision text NOT NULL DEFAULT '',
    connection_revision bigint NOT NULL CHECK (connection_revision > 0),
    state text NOT NULL CHECK (state IN ('accepted','completed','failed','superseded')),
    safe_reason text NOT NULL DEFAULT '',
    result text NOT NULL,
    accepted_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX workflow_connection_operations_history ON workflow_connection_operations(connection_id,id);
