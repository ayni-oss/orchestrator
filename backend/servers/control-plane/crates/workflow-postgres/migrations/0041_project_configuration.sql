CREATE TABLE workflow_project_configuration_heads (
 workspace_id text NOT NULL,
 project_id text PRIMARY KEY,
 revision_id text NOT NULL,
 FOREIGN KEY (workspace_id, project_id) REFERENCES workflow_projects(workspace_id,id),
 FOREIGN KEY (workspace_id, revision_id) REFERENCES workflow_agent_configurations(workspace_id,revision_id)
);
INSERT INTO workflow_project_configuration_heads
SELECT p.workspace_id,p.id,h.revision_id FROM workflow_projects p
JOIN workflow_agent_configuration_heads h ON h.workspace_id=p.workspace_id;

CREATE TABLE workflow_project_configuration_receipts (
 workspace_id text NOT NULL,
 project_id text NOT NULL,
 idempotency_key text NOT NULL,
 request_digest bytea NOT NULL CHECK(octet_length(request_digest)=32),
 revision_id text NOT NULL,
 PRIMARY KEY(project_id,idempotency_key),
 FOREIGN KEY(workspace_id,project_id) REFERENCES workflow_projects(workspace_id,id),
 FOREIGN KEY(workspace_id,revision_id) REFERENCES workflow_agent_configurations(workspace_id,revision_id)
);
REVOKE ALL ON workflow_project_configuration_heads,workflow_project_configuration_receipts FROM PUBLIC;

-- Old association receipts remain historical; workspace availability is authoritative.
ALTER TABLE workflow_project_repositories DROP CONSTRAINT project_repository_workspace_membership;

CREATE TABLE workflow_project_configuration_revisions (
 workspace_id text NOT NULL,
 project_id text NOT NULL,
 revision_id text NOT NULL,
 PRIMARY KEY(project_id,revision_id),
 FOREIGN KEY(workspace_id,project_id) REFERENCES workflow_projects(workspace_id,id),
 FOREIGN KEY(workspace_id,revision_id) REFERENCES workflow_agent_configurations(workspace_id,revision_id)
);
INSERT INTO workflow_project_configuration_revisions SELECT workspace_id,project_id,revision_id FROM workflow_project_configuration_heads;
INSERT INTO workflow_project_configuration_revisions
SELECT DISTINCT t.workspace_id,t.project_id,i.configuration_revision_id FROM workflow_turns t JOIN workflow_turn_inputs i ON i.turn_id=t.id ON CONFLICT DO NOTHING;
REVOKE ALL ON workflow_project_configuration_revisions FROM PUBLIC;
