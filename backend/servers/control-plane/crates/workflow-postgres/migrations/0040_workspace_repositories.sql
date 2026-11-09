CREATE TABLE workflow_workspace_repositories (
 workspace_id text NOT NULL REFERENCES workflow_workspaces(id),
 repository_id text NOT NULL REFERENCES workflow_repositories(id),
 PRIMARY KEY(workspace_id, repository_id)
);

-- Preserve the repository access already assigned to projects.
INSERT INTO workflow_workspace_repositories(workspace_id, repository_id)
SELECT DISTINCT workspace_id, repository_id FROM workflow_project_repositories;

ALTER TABLE workflow_project_repositories ADD CONSTRAINT project_repository_workspace_membership
 FOREIGN KEY(workspace_id, repository_id)
 REFERENCES workflow_workspace_repositories(workspace_id, repository_id);

CREATE INDEX workflow_project_repositories_workspace_repository_idx
 ON workflow_project_repositories(workspace_id, repository_id);

REVOKE ALL ON workflow_workspace_repositories FROM PUBLIC;
