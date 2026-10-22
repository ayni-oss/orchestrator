-- One retained operator authorization and the complete frozen admitted task set.
ALTER TABLE workflow_plan_versions ADD UNIQUE
    (workspace_id, project_id, conversation_id, plan_id, id, content_digest);
ALTER TABLE workflow_plans ADD UNIQUE
    (workspace_id, project_id, conversation_id, id, current_version_id);

CREATE TABLE workflow_plan_approvals (
    id text PRIMARY KEY,
    command_id text NOT NULL UNIQUE,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    plan_id text NOT NULL UNIQUE,
    version_id text NOT NULL UNIQUE,
    content_digest text NOT NULL,
    operator text NOT NULL CHECK (operator = 'local-operator'),
    idempotency_key text NOT NULL CHECK (octet_length(idempotency_key) BETWEEN 1 AND 128),
    request_digest bytea NOT NULL CHECK (octet_length(request_digest) = 32),
    task_ids text[] NOT NULL CHECK (cardinality(task_ids) BETWEEN 1 AND 32),
    conversation_revision bigint NOT NULL CHECK (conversation_revision > 0),
    resume_cursor text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (conversation_id, idempotency_key),
    UNIQUE (workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, id),
    FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, version_id, content_digest)
        REFERENCES workflow_plan_versions(workspace_id, project_id, conversation_id, plan_id, id, content_digest),
    FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, version_id)
        REFERENCES workflow_plans(workspace_id, project_id, conversation_id, id, current_version_id)
);
CREATE TABLE workflow_tasks (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    plan_id text NOT NULL,
    version_id text NOT NULL,
    content_digest text NOT NULL,
    approval_id text NOT NULL,
    proposed_task_key text NOT NULL CHECK (proposed_task_key ~ '^[A-Za-z0-9_-]{1,64}$'),
    specification bytea NOT NULL CHECK (octet_length(specification) BETWEEN 1 AND 32768),
    state text NOT NULL CHECK (state = 'awaiting_execution'),
    execution_blockers text[] NOT NULL CHECK (cardinality(execution_blockers) BETWEEN 3 AND 19),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (version_id, proposed_task_key),
    UNIQUE (approval_id, proposed_task_key),
    FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, approval_id)
        REFERENCES workflow_plan_approvals(workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, id)
);
CREATE INDEX workflow_tasks_conversation_id_idx ON workflow_tasks(conversation_id, id);
