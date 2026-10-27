-- Rename receipts outlive subsequent edits and are immutable to the runtime role.
CREATE TABLE workflow_hierarchy_rename_receipts (
 idempotency_key text PRIMARY KEY,
 command_id text NOT NULL UNIQUE,
 safe_request text NOT NULL,
 workspace_id text NOT NULL REFERENCES workflow_workspaces(id),
 project_id text,
 FOREIGN KEY(workspace_id,project_id) REFERENCES workflow_projects(workspace_id,id)
);
REVOKE ALL ON workflow_hierarchy_rename_receipts FROM PUBLIC;
