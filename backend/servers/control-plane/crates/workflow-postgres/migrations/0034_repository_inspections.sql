-- Frozen source inspection acceptance is independent of runtime preparation.
CREATE TABLE workflow_repository_inspections (
 id text PRIMARY KEY,
 idempotency_key text NOT NULL UNIQUE,
 safe_request text NOT NULL,
 repository_id text NOT NULL REFERENCES workflow_repositories(id),
 connection_id text NOT NULL REFERENCES workflow_connections(id),
 connection_revision bigint NOT NULL CHECK (connection_revision > 0),
 credential_revision text NOT NULL REFERENCES workflow_connection_credentials(revision_id),
 commit_sha text NOT NULL CHECK (commit_sha ~ '^[0-9a-f]{40}$'),
 state text NOT NULL CHECK (state IN ('accepted','completed','failed','superseded')),
 safe_reason text NOT NULL DEFAULT '',
 result text NOT NULL DEFAULT '',
 accepted_at_unix_ms bigint NOT NULL DEFAULT (extract(epoch FROM clock_timestamp())*1000)::bigint,
 updated_at_unix_ms bigint NOT NULL DEFAULT (extract(epoch FROM clock_timestamp())*1000)::bigint
);
CREATE INDEX workflow_repository_inspections_repository_history ON workflow_repository_inspections(repository_id,id DESC);
REVOKE ALL ON workflow_repository_inspections FROM PUBLIC;
