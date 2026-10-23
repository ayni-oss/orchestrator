-- Operator publication intent is immutable; external action authority is separate.
CREATE TABLE workflow_task_publications (
 publication_id text PRIMARY KEY,
 attempt_id text NOT NULL UNIQUE REFERENCES workflow_task_runs(attempt_id),
 task_id text NOT NULL REFERENCES workflow_tasks(id),
 workspace_id text NOT NULL, project_id text NOT NULL, conversation_id text NOT NULL,
 repository text NOT NULL,
 command bytea NOT NULL CHECK(octet_length(command) BETWEEN 1 AND 262144),
 state text NOT NULL CHECK(state IN ('queued','claimed','prepared','creating','uncertain','draft','blocked','cancelled')),
 generation bigint NOT NULL DEFAULT 1 CHECK(generation>0),
 pod_uid text NOT NULL DEFAULT '' CHECK(octet_length(pod_uid)<=64),
 lease_until timestamptz,
 commit_sha text NOT NULL DEFAULT '' CHECK(octet_length(commit_sha) IN (0,40)),
 tree_sha text NOT NULL DEFAULT '' CHECK(octet_length(tree_sha) IN (0,40)),
 outcome bytea CHECK(octet_length(outcome) BETWEEN 1 AND 8192),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(workspace_id,project_id,conversation_id,task_id) REFERENCES workflow_tasks(workspace_id,project_id,conversation_id,id)
);
CREATE UNIQUE INDEX workflow_task_publications_one_active ON workflow_task_publications(workspace_id,repository) WHERE state IN ('claimed','prepared','creating','uncertain');
CREATE TABLE workflow_task_publication_receipts (
 command_id text PRIMARY KEY,
 conversation_id text NOT NULL REFERENCES workflow_conversations(id),
 method text NOT NULL CHECK(method IN ('StartTaskPublication','CancelTaskPublication')),
 idempotency_key text NOT NULL CHECK(octet_length(idempotency_key) BETWEEN 1 AND 128),
 request_digest bytea NOT NULL CHECK(octet_length(request_digest)=32),
 publication_id text NOT NULL REFERENCES workflow_task_publications(publication_id),
 result bytea NOT NULL CHECK(octet_length(result) BETWEEN 1 AND 8192),
 UNIQUE(conversation_id,method,idempotency_key)
);
