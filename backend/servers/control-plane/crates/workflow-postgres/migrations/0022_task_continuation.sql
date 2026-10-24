-- Each confirmed recovery is a new linked execution segment; old evidence is immutable.
ALTER TABLE workflow_task_attempts DROP CONSTRAINT workflow_task_attempts_task_id_key;
ALTER TABLE workflow_task_attempts DROP CONSTRAINT workflow_task_attempts_preparation_command_id_key;
ALTER TABLE workflow_task_runs DROP CONSTRAINT workflow_task_runs_task_id_key;
ALTER TABLE workflow_task_runs DROP CONSTRAINT workflow_task_runs_command_check;
ALTER TABLE workflow_task_runs ADD CONSTRAINT workflow_task_runs_command_check CHECK(octet_length(command) BETWEEN 1 AND 262144);
CREATE UNIQUE INDEX workflow_task_runs_task_pending ON workflow_task_runs(task_id) WHERE state IN ('queued','claimed','reporting','cancelling');
CREATE TABLE workflow_task_recovery_confirmations (
 proposal_id text PRIMARY KEY REFERENCES workflow_task_recovery(id),
 parent_attempt_id text NOT NULL UNIQUE REFERENCES workflow_task_runs(attempt_id),
 attempt_id text NOT NULL UNIQUE REFERENCES workflow_task_runs(attempt_id),
 request_digest bytea NOT NULL,
 result bytea NOT NULL,
 CHECK(parent_attempt_id<>attempt_id)
);
CREATE TABLE workflow_task_recovery_confirmation_receipts (
 conversation_id text NOT NULL REFERENCES workflow_conversations(id),
 idempotency_key text NOT NULL CHECK(octet_length(idempotency_key) BETWEEN 1 AND 128),
 request_digest bytea NOT NULL,
 proposal_id text NOT NULL REFERENCES workflow_task_recovery_confirmations(proposal_id),
 result bytea NOT NULL,
 PRIMARY KEY(conversation_id,idempotency_key)
);
