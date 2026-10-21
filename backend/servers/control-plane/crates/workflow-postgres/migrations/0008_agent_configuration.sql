-- Immutable configuration identities are data revisions within the v1 interface.
CREATE TABLE workflow_agent_configurations (
    workspace_id text NOT NULL REFERENCES workflow_workspaces(id),
    revision_id text NOT NULL,
    instructions text NOT NULL CHECK (octet_length(instructions) <= 16384),
    provider text NOT NULL CHECK (provider IN ('scripted','openai')),
    model text NOT NULL CHECK (octet_length(model) BETWEEN 1 AND 256),
    max_input_tokens bigint NOT NULL CHECK (max_input_tokens BETWEEN 1 AND 1000000),
    max_output_tokens bigint NOT NULL CHECK (max_output_tokens BETWEEN 1 AND 100000),
    max_total_tokens bigint NOT NULL CHECK (max_total_tokens BETWEEN 2 AND 1100000 AND max_total_tokens >= max_input_tokens + max_output_tokens),
    idle_timeout_seconds bigint NOT NULL CHECK (idle_timeout_seconds BETWEEN 30 AND 86400),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (workspace_id, revision_id)
);
CREATE TABLE workflow_agent_configuration_heads (
    workspace_id text PRIMARY KEY REFERENCES workflow_workspaces(id),
    revision_id text NOT NULL,
    FOREIGN KEY (workspace_id, revision_id) REFERENCES workflow_agent_configurations(workspace_id,revision_id)
);
CREATE TABLE workflow_configuration_receipts (
    workspace_id text NOT NULL REFERENCES workflow_workspaces(id),
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL CHECK (octet_length(request_digest) = 32),
    revision_id text NOT NULL,
    PRIMARY KEY (workspace_id,idempotency_key),
    FOREIGN KEY (workspace_id,revision_id) REFERENCES workflow_agent_configurations(workspace_id,revision_id)
);
ALTER TABLE workflow_turns ADD UNIQUE (workspace_id,id);
CREATE TABLE workflow_turn_inputs (
    turn_id text PRIMARY KEY,
    workspace_id text NOT NULL,
    configuration_revision_id text NOT NULL,
    FOREIGN KEY (workspace_id, turn_id) REFERENCES workflow_turns(workspace_id,id),
    FOREIGN KEY (workspace_id,configuration_revision_id) REFERENCES workflow_agent_configurations(workspace_id,revision_id)
);
INSERT INTO workflow_agent_configurations
(workspace_id,revision_id,instructions,provider,model,max_input_tokens,max_output_tokens,max_total_tokens,idle_timeout_seconds)
SELECT id,id,'','scripted','scripted',8192,2048,10240,300 FROM workflow_workspaces;
INSERT INTO workflow_agent_configuration_heads (workspace_id,revision_id) SELECT id,id FROM workflow_workspaces;
-- Already admitted turns retain their original execution identity and receive no rewritten input.

CREATE TABLE workflow_context_manifests (
    turn_id text PRIMARY KEY REFERENCES workflow_turns(id),
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    context_revision bigint NOT NULL CHECK (context_revision > 0),
    record_count bigint NOT NULL CHECK (record_count > 0),
    digest text NOT NULL CHECK (length(digest) = 71),
    UNIQUE (turn_id,conversation_id),
    FOREIGN KEY (workspace_id, project_id, conversation_id) REFERENCES workflow_conversations(workspace_id,project_id,id)
);
CREATE TABLE workflow_context_manifest_messages (
    turn_id text NOT NULL,
    ordinal bigint NOT NULL CHECK (ordinal > 0),
    conversation_id text NOT NULL,
    message_id text NOT NULL,
    PRIMARY KEY (turn_id,ordinal),
    UNIQUE (turn_id,message_id),
    FOREIGN KEY (turn_id,conversation_id) REFERENCES workflow_context_manifests(turn_id,conversation_id) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (conversation_id,message_id) REFERENCES workflow_messages(conversation_id,id)
);
