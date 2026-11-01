-- Reviewed one-shot repository runtime intent. No credential values or automatic renewal of grants.
CREATE TABLE workflow_repository_runtime_runs (
 id text PRIMARY KEY,
 idempotency_key text NOT NULL UNIQUE,
 repository_id text NOT NULL REFERENCES workflow_repositories(id),
 safe_request text NOT NULL,
 receipt text NOT NULL,
 state text NOT NULL CHECK (state IN ('resolving','inspecting','admitting','completed','blocked')),
 progress text NOT NULL CHECK (octet_length(progress)<=32768),
 claim_token text NOT NULL DEFAULT '',
 lease_until timestamptz NOT NULL DEFAULT '-infinity',
 CHECK (progress::jsonb->>'state'=state)
);
CREATE UNIQUE INDEX workflow_repository_runtime_active ON workflow_repository_runtime_runs(repository_id)
 WHERE state IN ('resolving','inspecting','admitting');
CREATE INDEX workflow_repository_runtime_queue ON workflow_repository_runtime_runs(lease_until,id)
 WHERE state IN ('resolving','inspecting','admitting');
REVOKE ALL ON workflow_repository_runtime_runs FROM PUBLIC;
