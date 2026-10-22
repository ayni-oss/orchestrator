-- Immutable dispatch inputs; mutable execution authority is separate from preparation.
CREATE TABLE workflow_task_runs (
 attempt_id text PRIMARY KEY REFERENCES workflow_task_attempts(id),
 task_id text NOT NULL UNIQUE REFERENCES workflow_tasks(id),
 workspace_id text NOT NULL, project_id text NOT NULL, conversation_id text NOT NULL,
 command bytea NOT NULL CHECK(octet_length(command) BETWEEN 1 AND 65536),
 source_pack bytea NOT NULL CHECK(octet_length(source_pack) BETWEEN 1 AND 8388608),
 state text NOT NULL CHECK(state IN ('queued','claimed','reporting','cancelling','completed','failed','uncertain','cancelled','interrupted')),
 generation bigint NOT NULL DEFAULT 1 CHECK(generation > 0),
 pod_uid text NOT NULL DEFAULT '' CHECK(octet_length(pod_uid) <= 64),
 deadline_at timestamptz,
 provider_state text NOT NULL DEFAULT 'not_started' CHECK(provider_state IN ('not_started','sending','settled','uncertain')),
 request_digest text NOT NULL DEFAULT '' CHECK(octet_length(request_digest) <= 71),
 outcome bytea CHECK(octet_length(outcome) <= 131072),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(workspace_id,project_id,conversation_id,task_id) REFERENCES workflow_tasks(workspace_id,project_id,conversation_id,id)
);
-- A cancelled worker still occupies the global serial slot until deletion is confirmed.
CREATE UNIQUE INDEX workflow_task_runs_one_active ON workflow_task_runs ((true)) WHERE state IN ('claimed','reporting','cancelling');
CREATE TABLE workflow_task_run_receipts (
 command_id text PRIMARY KEY,
 conversation_id text NOT NULL REFERENCES workflow_conversations(id),
 method text NOT NULL CHECK(method IN ('StartTaskExecution','CancelTaskExecution')),
 idempotency_key text NOT NULL CHECK(octet_length(idempotency_key) BETWEEN 1 AND 128),
 request_digest bytea NOT NULL CHECK(octet_length(request_digest)=32),
 attempt_id text NOT NULL REFERENCES workflow_task_attempts(id),
 conversation_revision bigint NOT NULL,
 resume_cursor text NOT NULL,
 UNIQUE(conversation_id,method,idempotency_key)
);
