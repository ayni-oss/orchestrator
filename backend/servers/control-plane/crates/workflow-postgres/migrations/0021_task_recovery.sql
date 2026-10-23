-- Review records grant no execution, provider or publication authority.
CREATE TABLE workflow_task_recovery (
 id text PRIMARY KEY,
 workspace_id text NOT NULL,
 project_id text NOT NULL,
 conversation_id text NOT NULL,
 task_id text NOT NULL,
 attempt_id text NOT NULL REFERENCES workflow_task_runs(attempt_id),
 idempotency_key text NOT NULL,
 request_digest bytea NOT NULL,
 kind text NOT NULL CHECK(kind IN ('proposal','question','answer')),
 question_id text REFERENCES workflow_task_recovery(id),
 content bytea NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(attempt_id,idempotency_key),
 UNIQUE(question_id),
 CHECK ((kind='answer') = (question_id IS NOT NULL)),
 FOREIGN KEY(workspace_id,project_id,conversation_id,task_id) REFERENCES workflow_tasks(workspace_id,project_id,conversation_id,id)
);
CREATE INDEX workflow_task_recovery_origin ON workflow_task_recovery(workspace_id,project_id,conversation_id,task_id,attempt_id,id);
