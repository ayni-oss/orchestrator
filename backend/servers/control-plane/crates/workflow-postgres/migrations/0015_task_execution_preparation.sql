-- Preparation retains operator evidence; no coding dispatch is authorized here.
ALTER TABLE workflow_tasks ADD UNIQUE (workspace_id, project_id, conversation_id, id);
ALTER TABLE workflow_tasks ADD UNIQUE
    (workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, approval_id, id);
CREATE TABLE workflow_task_preparations (
    command_id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    task_id text NOT NULL,
    FOREIGN KEY (workspace_id, project_id, conversation_id, task_id)
        REFERENCES workflow_tasks(workspace_id, project_id, conversation_id, id),
    idempotency_key text NOT NULL CHECK (octet_length(idempotency_key) BETWEEN 1 AND 128),
    request_digest bytea NOT NULL CHECK (octet_length(request_digest) = 32),
    result bytea NOT NULL CHECK (octet_length(result) BETWEEN 1 AND 16384),
    UNIQUE (workspace_id, project_id, conversation_id, task_id, command_id),
    UNIQUE (conversation_id, idempotency_key)
);
CREATE INDEX workflow_task_preparations_task_idx ON workflow_task_preparations(task_id, command_id);
CREATE TABLE workflow_task_attempts (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    plan_id text NOT NULL,
    version_id text NOT NULL,
    content_digest text NOT NULL,
    approval_id text NOT NULL,
    task_id text NOT NULL UNIQUE,
    preparation_command_id text NOT NULL UNIQUE,
    FOREIGN KEY (workspace_id, project_id, conversation_id, task_id, preparation_command_id)
        REFERENCES workflow_task_preparations(workspace_id, project_id, conversation_id, task_id, command_id),
    configuration_revision_id text NOT NULL,
    FOREIGN KEY (workspace_id, configuration_revision_id) REFERENCES workflow_agent_configurations(workspace_id, revision_id),
    inputs bytea NOT NULL CHECK (octet_length(inputs) BETWEEN 1 AND 4096),
    state text NOT NULL CHECK (state = 'prepared'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, approval_id, task_id)
        REFERENCES workflow_tasks(workspace_id, project_id, conversation_id, plan_id, version_id, content_digest, approval_id, id)
);
