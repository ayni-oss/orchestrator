-- Hierarchy: final table definitions for fresh installations.

CREATE TABLE public.workflow_workspaces (
    id text NOT NULL,
    display_name text NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    updated_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    resource_version bigint DEFAULT 1 NOT NULL,
    CONSTRAINT workflow_workspaces_display_name_check CHECK ((((octet_length(display_name) >= 1) AND (octet_length(display_name) <= 256)) AND (display_name = btrim(display_name)))),
    CONSTRAINT workflow_workspaces_resource_version_check CHECK ((resource_version > 0)),
    CONSTRAINT workflow_workspaces_pkey PRIMARY KEY (id)
);

CREATE TABLE public.workflow_projects (
    id text NOT NULL,
    workspace_id text NOT NULL,
    display_name text NOT NULL,
    goal text NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    updated_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    resource_version bigint DEFAULT 1 NOT NULL,
    CONSTRAINT workflow_projects_display_name_check CHECK ((((octet_length(display_name) >= 1) AND (octet_length(display_name) <= 256)) AND (display_name = btrim(display_name)))),
    CONSTRAINT workflow_projects_goal_check CHECK ((octet_length(goal) <= 16384)),
    CONSTRAINT workflow_projects_resource_version_check CHECK ((resource_version > 0)),
    CONSTRAINT workflow_projects_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_projects_workspace_id_id_key UNIQUE (workspace_id, id),
    CONSTRAINT workflow_projects_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES public.workflow_workspaces(id)
);

CREATE TABLE public.workflow_hierarchy_rename_receipts (
    idempotency_key text NOT NULL,
    command_id text NOT NULL,
    safe_request text NOT NULL,
    workspace_id text NOT NULL,
    project_id text,
    CONSTRAINT workflow_hierarchy_rename_receipts_command_id_key UNIQUE (command_id),
    CONSTRAINT workflow_hierarchy_rename_receipts_pkey PRIMARY KEY (idempotency_key),
    CONSTRAINT workflow_hierarchy_rename_receipts_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES public.workflow_workspaces(id),
    CONSTRAINT workflow_hierarchy_rename_receipts_workspace_id_project_id_fkey FOREIGN KEY (workspace_id, project_id) REFERENCES public.workflow_projects(workspace_id, id)
);
