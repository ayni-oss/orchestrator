-- Feedback belongs to the exact admitted origin and is immutable across retries.
CREATE TABLE workflow_task_feedback (
 id text PRIMARY KEY,
 workspace_id text NOT NULL,
 project_id text NOT NULL,
 conversation_id text NOT NULL,
 task_id text NOT NULL,
 attempt_id text NOT NULL REFERENCES workflow_task_runs(attempt_id),
 category text NOT NULL CHECK(category IN ('execution','quality')),
 safe_reason text NOT NULL CHECK(safe_reason IN ('execution_failed','execution_interrupted','provider_uncertain','execution_deadline_exceeded','quality_failed','quality_blocked','quality_interrupted')),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(attempt_id,category),
 FOREIGN KEY(workspace_id,project_id,conversation_id,task_id) REFERENCES workflow_tasks(workspace_id,project_id,conversation_id,id)
);
CREATE INDEX workflow_task_feedback_project ON workflow_task_feedback(workspace_id,project_id,id);
CREATE INDEX workflow_task_feedback_conversation ON workflow_task_feedback(workspace_id,project_id,conversation_id,id);
