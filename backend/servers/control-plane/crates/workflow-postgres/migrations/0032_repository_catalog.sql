-- Shared provider identity is independent of workspace/project ownership.
CREATE TABLE workflow_repositories (
 id text PRIMARY KEY,
 connection_id text NOT NULL REFERENCES workflow_connections(id),
 installation_id bigint NOT NULL CHECK (installation_id > 0),
 provider_repository_id bigint NOT NULL UNIQUE CHECK (provider_repository_id > 0),
 full_name text NOT NULL CHECK (octet_length(full_name) BETWEEN 3 AND 256),
 default_branch text NOT NULL CHECK (octet_length(default_branch) BETWEEN 1 AND 256),
 private boolean NOT NULL,
 connected_at_unix_ms bigint NOT NULL DEFAULT (extract(epoch FROM clock_timestamp())*1000)::bigint
);
CREATE TABLE workflow_repository_operations (
 id text PRIMARY KEY,
 idempotency_key text NOT NULL UNIQUE,
 safe_request text NOT NULL,
 connection_id text NOT NULL REFERENCES workflow_connections(id),
 connection_revision bigint NOT NULL CHECK (connection_revision > 0),
 credential_revision text NOT NULL REFERENCES workflow_connection_credentials(revision_id),
 installation_id bigint NOT NULL CHECK (installation_id > 0),
 provider_repository_id bigint NOT NULL CHECK (provider_repository_id > 0),
 state text NOT NULL CHECK (state IN ('accepted','completed','failed','superseded')),
 safe_reason text NOT NULL DEFAULT '',
 result text NOT NULL DEFAULT '',
 accepted_at_unix_ms bigint NOT NULL DEFAULT (extract(epoch FROM clock_timestamp())*1000)::bigint,
 updated_at_unix_ms bigint NOT NULL DEFAULT (extract(epoch FROM clock_timestamp())*1000)::bigint
);
CREATE TABLE workflow_project_repositories (
 workspace_id text NOT NULL REFERENCES workflow_workspaces(id),
 project_id text NOT NULL,
 repository_id text NOT NULL REFERENCES workflow_repositories(id),
 FOREIGN KEY(workspace_id,project_id) REFERENCES workflow_projects(workspace_id,id),
 PRIMARY KEY(workspace_id,project_id,repository_id)
);
CREATE TABLE workflow_repository_association_receipts (
 idempotency_key text PRIMARY KEY,
 command_id text NOT NULL UNIQUE,
 safe_request text NOT NULL,
 repository_id text NOT NULL REFERENCES workflow_repositories(id)
);
REVOKE ALL ON workflow_repositories, workflow_repository_operations,
 workflow_project_repositories, workflow_repository_association_receipts FROM PUBLIC;
