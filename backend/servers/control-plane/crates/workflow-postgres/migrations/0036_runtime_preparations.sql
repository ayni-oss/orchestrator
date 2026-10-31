-- Frozen operation inputs contain public metadata/source only; credential values stay in their existing authority store.
CREATE TABLE workflow_runtime_preparations (
 id text PRIMARY KEY,
 idempotency_key text NOT NULL UNIQUE,
 request_digest text NOT NULL,
 repository_id text NOT NULL REFERENCES workflow_repositories(id),
 inspection_id text NOT NULL REFERENCES workflow_repository_inspections(id),
 registry_connection_id text NOT NULL REFERENCES workflow_connections(id),
 signer_id text NOT NULL REFERENCES workflow_runtime_signers(id),
 platform text NOT NULL CHECK(platform IN ('linux/amd64','linux/arm64')),
 frozen text NOT NULL CHECK(octet_length(frozen) <= 41943040),
 frozen_input_digest text NOT NULL,
 state text NOT NULL CHECK(state IN ('pending','running','stopping','cancelling','cleanup_pending','ready','cancelled','superseded','unavailable','failed','blocked','publication_uncertain')),
 stage text NOT NULL CHECK(stage IN ('acquire_source','discover','pull_existing','build','freeze_candidate','certify','finalize','publish','pull_final','verify_content','verify_compatibility','cleanup')),
 safe_reason text NOT NULL DEFAULT '',
 terminal_state text NOT NULL DEFAULT '',
 generation bigint NOT NULL CHECK(generation > 0),
 allow_build boolean NOT NULL,
 allow_replace boolean NOT NULL,
 max_builds integer NOT NULL CHECK(max_builds BETWEEN 0 AND 10),
 builds_started integer NOT NULL DEFAULT 0 CHECK(builds_started >= 0 AND builds_started <= max_builds),
 deadline_unix_ms bigint NOT NULL CHECK(deadline_unix_ms > 0),
 evidence text NOT NULL DEFAULT '{}',
 accepted_at_unix_ms bigint NOT NULL DEFAULT (extract(epoch FROM clock_timestamp())*1000)::bigint,
 updated_at_unix_ms bigint NOT NULL DEFAULT (extract(epoch FROM clock_timestamp())*1000)::bigint
);
CREATE TABLE workflow_runtime_preparation_selections (
 repository_id text PRIMARY KEY REFERENCES workflow_repositories(id),
 operation_id text NOT NULL UNIQUE REFERENCES workflow_runtime_preparations(id)
);
CREATE TABLE workflow_runtime_preparation_attempts (
 id text PRIMARY KEY,
 operation_id text NOT NULL REFERENCES workflow_runtime_preparations(id),
 generation bigint NOT NULL CHECK(generation > 0),
 stage text NOT NULL,
 command_digest text NOT NULL,
 pod_uid text NOT NULL DEFAULT '',
 state text NOT NULL CHECK(state IN ('claimed','bound','reported','stopped')),
 deadline_unix_ms bigint NOT NULL,
 accepted_at_unix_ms bigint NOT NULL DEFAULT (extract(epoch FROM clock_timestamp())*1000)::bigint,
 report_digest text NOT NULL DEFAULT '',
 report text NOT NULL DEFAULT '',
 UNIQUE(operation_id,generation)
);
CREATE TABLE workflow_runtime_preparation_receipts (
 idempotency_key text PRIMARY KEY,
 command_id text NOT NULL UNIQUE,
 operation_id text NOT NULL REFERENCES workflow_runtime_preparations(id),
 kind text NOT NULL CHECK(kind IN ('start','cancel','retry')),
 request_digest text NOT NULL,
 result text NOT NULL
);
CREATE INDEX workflow_runtime_preparation_history ON workflow_runtime_preparations(repository_id,id DESC);
CREATE INDEX workflow_runtime_preparation_pending ON workflow_runtime_preparations(state,id);
REVOKE ALL ON workflow_runtime_preparations,workflow_runtime_preparation_selections,workflow_runtime_preparation_attempts,workflow_runtime_preparation_receipts FROM PUBLIC;
