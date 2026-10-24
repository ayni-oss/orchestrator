-- Assessment consent is immutable, scoped and tied to one reviewed configuration.
-- A zero allowance revokes future automatic admission without erasing paid calls.
CREATE TABLE workflow_assessment_authorizations (
 id text PRIMARY KEY,
 workspace_id text NOT NULL,
 project_id text NOT NULL,
 conversation_id text NOT NULL,
 idempotency_key text NOT NULL,
 configuration_revision_id text NOT NULL,
 max_assessments bigint NOT NULL CHECK(max_assessments BETWEEN 0 AND 10),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(conversation_id,idempotency_key),
 UNIQUE(workspace_id,project_id,conversation_id,id),
 FOREIGN KEY(workspace_id,project_id,conversation_id) REFERENCES workflow_conversations(workspace_id,project_id,id),
 FOREIGN KEY(workspace_id,configuration_revision_id) REFERENCES workflow_agent_configurations(workspace_id,revision_id)
);
CREATE INDEX workflow_assessment_authorizations_origin ON workflow_assessment_authorizations(conversation_id,created_at DESC,id DESC);

-- One coalesced snapshot is admitted once under each explicit allowance. The
-- normal turn owns all paid-call, cancellation, reservation and report recovery.
CREATE TABLE workflow_outcome_assessments (
 id text PRIMARY KEY,
 workspace_id text NOT NULL,
 project_id text NOT NULL,
 conversation_id text NOT NULL,
 authorization_id text NOT NULL,
 snapshot_digest text NOT NULL,
 snapshot bytea NOT NULL CHECK(octet_length(snapshot) BETWEEN 1 AND 131072),
 turn_id text NOT NULL UNIQUE REFERENCES workflow_turns(id),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(authorization_id,snapshot_digest),
 FOREIGN KEY(workspace_id,project_id,conversation_id,authorization_id) REFERENCES workflow_assessment_authorizations(workspace_id,project_id,conversation_id,id)
);

-- Append-only scan cursors let a bounded maintenance sweep advance past unchanged
-- origins without changing consent, consuming allowance or scheduling a model.
CREATE TABLE workflow_assessment_scans (
 authorization_id text NOT NULL REFERENCES workflow_assessment_authorizations(id),
 observed_cursor bigint NOT NULL CHECK(observed_cursor >= 0),
 PRIMARY KEY(authorization_id,observed_cursor)
);

-- Accepted outcome ordering is durable and does not depend on replay retention.
ALTER TABLE workflow_task_runs ADD COLUMN assessment_outcome_at timestamptz;
UPDATE workflow_task_runs SET assessment_outcome_at=created_at;
ALTER TABLE workflow_task_runs ALTER COLUMN assessment_outcome_at SET DEFAULT clock_timestamp();
ALTER TABLE workflow_task_runs ALTER COLUMN assessment_outcome_at SET NOT NULL;
