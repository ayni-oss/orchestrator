CREATE TABLE workflow_plans (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    current_version_id text NOT NULL,
    follows_plan_id text,
    UNIQUE (workspace_id, project_id, conversation_id, id),
    FOREIGN KEY (workspace_id, project_id, conversation_id)
        REFERENCES workflow_conversations(workspace_id, project_id, id),
    FOREIGN KEY (workspace_id, project_id, conversation_id, follows_plan_id)
        REFERENCES workflow_plans(workspace_id, project_id, conversation_id, id)
);
CREATE TABLE workflow_plan_versions (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    plan_id text NOT NULL,
    ordinal bigint NOT NULL CHECK (ordinal BETWEEN 1 AND 64),
    predecessor_id text,
    content bytea NOT NULL CHECK (octet_length(content) BETWEEN 1 AND 32768),
    content_digest text NOT NULL CHECK (content_digest ~ '^sha256:[0-9a-f]{64}$'),
    title text NOT NULL CHECK (octet_length(title) BETWEEN 1 AND 256),
    author text NOT NULL CHECK (author = 'local-operator'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (plan_id, ordinal),
    UNIQUE (workspace_id, project_id, conversation_id, plan_id, id),
    FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id)
        REFERENCES workflow_plans(workspace_id, project_id, conversation_id, id),
    FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, predecessor_id)
        REFERENCES workflow_plan_versions(workspace_id, project_id, conversation_id, plan_id, id),
    CHECK ((ordinal = 1) = (predecessor_id IS NULL))
);
ALTER TABLE workflow_plans ADD CONSTRAINT workflow_plans_current_version_fk
    FOREIGN KEY (workspace_id, project_id, conversation_id, id, current_version_id)
    REFERENCES workflow_plan_versions(workspace_id, project_id, conversation_id, plan_id, id)
    DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE workflow_plan_receipts (
    conversation_id text NOT NULL REFERENCES workflow_conversations(id),
    method text NOT NULL CHECK (method IN ('CreatePlan', 'RevisePlan')),
    idempotency_key text NOT NULL CHECK (octet_length(idempotency_key) BETWEEN 1 AND 128),
    request_digest bytea NOT NULL CHECK (octet_length(request_digest) = 32),
    command_id text NOT NULL UNIQUE,
    version_id text NOT NULL UNIQUE REFERENCES workflow_plan_versions(id),
    conversation_revision bigint NOT NULL CHECK (conversation_revision > 0),
    resume_cursor text NOT NULL,
    PRIMARY KEY (conversation_id, method, idempotency_key)
);
