//! Workspace ownership is independent of repository runtime identity.
use super::{
    CleanStore, CleanStoreError, Connection, Message, PROJECT_SERVICE, Page, action_row,
    collect_bounded_payloads, decode, encode_filter, get_payload, params, request_digest,
    resource_id, timestamp, unix_seconds, v1,
};

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(CleanStoreError::Database)?;
    transaction.execute_batch("CREATE TABLE workspaces(id TEXT PRIMARY KEY,state INTEGER NOT NULL,payload BLOB NOT NULL,resource_version INTEGER NOT NULL);").map_err(CleanStoreError::Database)?;
    let projects = {
        let mut statement = transaction
            .prepare("SELECT payload FROM projects ORDER BY id")
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        collect_bounded_payloads(&mut rows)?
    };
    for payload in projects {
        let mut project: v1::Project = decode(&payload)?;
        project.workspace_id = format!("workspace-{}", project.id);
        let workspace = v1::Workspace {
            id: project.workspace_id.clone(),
            display_name: project.display_name.clone(),
            state: project.state,
            repository_connection_ids: project.repository_connection_ids.clone(),
            created_at: project.created_at,
            updated_at: project.updated_at,
            resource_version: 1,
            agent_defaults: project.agent_defaults.take(),
            planner_idle_timeout_seconds: super::project_sessions::DEFAULT_IDLE_SECONDS,
        };
        transaction
            .execute(
                "INSERT INTO workspaces(id,state,payload,resource_version) VALUES(?1,?2,?3,?4)",
                params![workspace.id, workspace.state, workspace.encode_to_vec(), 1],
            )
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "UPDATE projects SET payload=?2 WHERE id=?1",
                params![project.id, project.encode_to_vec()],
            )
            .map_err(CleanStoreError::Database)?;
    }
    transaction
        .execute(
            "UPDATE clean_schema SET version=20 WHERE component='control_plane'",
            [],
        )
        .map_err(CleanStoreError::Database)?;
    transaction.commit().map_err(CleanStoreError::Database)
}

impl CleanStore {
    pub(crate) fn workspace(&self, id: &str) -> Result<v1::Workspace, CleanStoreError> {
        let mut workspace: v1::Workspace = get_payload(
            &self.connection,
            "SELECT payload FROM workspaces WHERE id=?1",
            id,
            "workspace",
        )?;
        if workspace.planner_idle_timeout_seconds == 0 {
            workspace.planner_idle_timeout_seconds = super::project_sessions::DEFAULT_IDLE_SECONDS;
        }
        super::project_sessions::idle_timeout(workspace.planner_idle_timeout_seconds)?;
        Ok(workspace)
    }
    pub(crate) fn active_workspace(&self, id: &str) -> Result<v1::Workspace, CleanStoreError> {
        let workspace = self.workspace(id)?;
        if workspace.state != v1::WorkspaceState::Active as i32 {
            return Err(CleanStoreError::ProjectArchived);
        }
        Ok(workspace)
    }
    pub(crate) fn effective_project_defaults(
        &self,
        project: &v1::Project,
    ) -> Result<Option<v1::ProjectAgentDefaults>, CleanStoreError> {
        let mut defaults = if project.workspace_id.is_empty() {
            None
        } else {
            self.workspace(&project.workspace_id)?.agent_defaults
        };
        let workspace_defaults = defaults.clone();
        defaults.get_or_insert_default();
        if let Some(overrides) = &project.agent_defaults {
            let effective = defaults.get_or_insert_default();
            if !overrides.project_connection_id.is_empty() {
                effective
                    .project_connection_id
                    .clone_from(&overrides.project_connection_id);
                effective.project_model.clone_from(&overrides.project_model);
            }
            if !overrides.coding_connection_id.is_empty() {
                effective
                    .coding_connection_id
                    .clone_from(&overrides.coding_connection_id);
                effective.coding_model.clone_from(&overrides.coding_model);
            }
        }
        if let Some(effective) = defaults.as_mut() {
            self.merge_agent_profiles(
                workspace_defaults.as_ref(),
                project.agent_defaults.as_ref(),
                effective,
            )?;
        }
        Ok(defaults)
    }
    pub(crate) fn effective_workspace_defaults(
        &self,
        workspace: &v1::Workspace,
    ) -> Result<v1::ProjectAgentDefaults, CleanStoreError> {
        let mut defaults = workspace.agent_defaults.clone().unwrap_or_default();
        self.merge_agent_profiles(workspace.agent_defaults.as_ref(), None, &mut defaults)?;
        Ok(defaults)
    }
    pub(crate) fn replay_workspace(
        &self,
        operation: &str,
        key: &str,
        request: &impl Message,
    ) -> Result<Option<v1::Workspace>, CleanStoreError> {
        let Some(action) = action_row(&self.connection, operation, key)? else {
            return Ok(None);
        };
        if action.request_hash != request_digest(operation, request) {
            return Err(CleanStoreError::IdempotencyConflict);
        }
        self.workspace(&action.target_id.ok_or(CleanStoreError::CorruptState)?)
            .map(Some)
    }
    pub(super) fn validate_agent_defaults(
        &self,
        defaults: Option<&v1::ProjectAgentDefaults>,
    ) -> Result<(), CleanStoreError> {
        if let Some(defaults) = defaults {
            crate::agent_profiles::selection(
                defaults.planner_profile.as_ref(),
                &ayni_orchestrator_runtime::agent_profile::Role::Planner,
            )
            .map_err(|_| CleanStoreError::InvalidTransition)?;
            crate::agent_profiles::selection(
                defaults.worker_profile.as_ref(),
                &ayni_orchestrator_runtime::agent_profile::Role::Worker,
            )
            .map_err(|_| CleanStoreError::InvalidTransition)?;
            for (id, model) in [
                (&defaults.project_connection_id, &defaults.project_model),
                (&defaults.coding_connection_id, &defaults.coding_model),
            ] {
                if id.is_empty() && model.is_empty() {
                    continue;
                }
                let selected = self.get_connection(id)?;
                if selected.kind != "openai"
                    || !selected.enabled
                    || selected.status != "ready"
                    || model.is_empty()
                    || model.len() > 128
                    || model.chars().any(char::is_control)
                {
                    return Err(CleanStoreError::InvalidTransition);
                }
            }
        }
        Ok(())
    }
    pub(crate) fn create_workspace(
        &mut self,
        key: &str,
        request: &v1::CreateWorkspaceRequest,
    ) -> Result<v1::Workspace, CleanStoreError> {
        if let Some(workspace) = self.replay_workspace("CreateWorkspace", key, request)? {
            return Ok(workspace);
        }
        self.validate_workspace_members(&request.repository_connection_ids)?;
        self.validate_agent_defaults(request.agent_defaults.as_ref())?;
        super::agent_profiles::validate_profile_scopes(
            request.agent_defaults.as_ref(),
            &request.repository_connection_ids,
        )?;
        let now = timestamp(unix_seconds()?);
        let workspace = v1::Workspace {
            id: resource_id("workspace"),
            display_name: request.display_name.clone(),
            state: v1::WorkspaceState::Active as i32,
            repository_connection_ids: request.repository_connection_ids.clone(),
            created_at: Some(now),
            updated_at: Some(now),
            resource_version: 1,
            agent_defaults: request.agent_defaults.clone(),
            planner_idle_timeout_seconds: super::project_sessions::idle_timeout(
                request
                    .planner_idle_timeout_seconds
                    .unwrap_or(super::project_sessions::DEFAULT_IDLE_SECONDS),
            )?,
        };
        self.save_workspace("CreateWorkspace", key, request, &workspace, true)?;
        Ok(workspace)
    }
    fn validate_workspace_members(&self, members: &[String]) -> Result<(), CleanStoreError> {
        super::api_validation::project_members(members)
            .map_err(|_| CleanStoreError::ParentMismatch)?;
        for id in members {
            self.repository_connection(id)?;
        }
        Ok(())
    }
    fn editable_workspace(&self, id: &str, version: u64) -> Result<v1::Workspace, CleanStoreError> {
        let mut workspace = self.active_workspace(id)?;
        if workspace.resource_version != version {
            return Err(CleanStoreError::StaleVersion);
        }
        workspace.resource_version = workspace
            .resource_version
            .checked_add(1)
            .ok_or(CleanStoreError::BoundExceeded)?;
        workspace.updated_at = Some(timestamp(unix_seconds()?));
        Ok(workspace)
    }
    pub(crate) fn update_workspace(
        &mut self,
        key: &str,
        request: &v1::UpdateWorkspaceRequest,
    ) -> Result<v1::Workspace, CleanStoreError> {
        if let Some(workspace) = self.replay_workspace("UpdateWorkspace", key, request)? {
            return Ok(workspace);
        }
        let mut workspace =
            self.editable_workspace(&request.workspace_id, request.expected_resource_version)?;
        workspace.display_name.clone_from(&request.display_name);
        if !request.repository_connection_ids.is_empty() {
            self.validate_workspace_members(&request.repository_connection_ids)?;
            workspace
                .repository_connection_ids
                .clone_from(&request.repository_connection_ids);
        }
        if request.agent_defaults.is_some() {
            super::agent_profiles::validate_profile_scopes(
                request.agent_defaults.as_ref(),
                &workspace.repository_connection_ids,
            )?;
            self.validate_agent_defaults(request.agent_defaults.as_ref())?;
            workspace.agent_defaults.clone_from(&request.agent_defaults);
        }
        if let Some(timeout) = request.planner_idle_timeout_seconds {
            workspace.planner_idle_timeout_seconds =
                super::project_sessions::idle_timeout(timeout)?;
        }
        self.save_workspace("UpdateWorkspace", key, request, &workspace, false)?;
        Ok(workspace)
    }
    pub(crate) fn archive_workspace(
        &mut self,
        key: &str,
        request: &v1::ArchiveWorkspaceRequest,
    ) -> Result<v1::Workspace, CleanStoreError> {
        if let Some(workspace) = self.replay_workspace("ArchiveWorkspace", key, request)? {
            return Ok(workspace);
        }
        let mut workspace =
            self.editable_workspace(&request.workspace_id, request.expected_resource_version)?;
        workspace.state = v1::WorkspaceState::Archived as i32;
        self.save_workspace("ArchiveWorkspace", key, request, &workspace, false)?;
        Ok(workspace)
    }
    fn save_workspace(
        &mut self,
        operation: &str,
        key: &str,
        request: &impl Message,
        workspace: &v1::Workspace,
        create: bool,
    ) -> Result<(), CleanStoreError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let version = i64::try_from(workspace.resource_version)
            .map_err(|_| CleanStoreError::BoundExceeded)?;
        if create {transaction.execute("INSERT INTO workspaces(id,state,payload,resource_version) VALUES(?1,?2,?3,?4)",params![workspace.id,workspace.state,workspace.encode_to_vec(),version]).map_err(CleanStoreError::Database)?;}
        else if transaction.execute("UPDATE workspaces SET state=?2,payload=?3,resource_version=?4 WHERE id=?1 AND resource_version=?5",params![workspace.id,workspace.state,workspace.encode_to_vec(),version,version-1]).map_err(CleanStoreError::Database)? != 1 {return Err(CleanStoreError::StaleVersion);}
        let now = unix_seconds()?;
        transaction.execute("INSERT INTO durable_actions(operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at) VALUES(?1,?2,?3,?4,'completed','workspace',?5,?6,?6)",params![operation,key,request_digest(operation,request),request.encode_to_vec(),workspace.id,now]).map_err(CleanStoreError::Database)?;
        transaction.commit().map_err(CleanStoreError::Database)
    }
    pub(crate) fn list_workspaces(
        &mut self,
        state: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::Workspace>, CleanStoreError> {
        let filter = encode_filter(&[&state.map_or_else(String::new, |value| value.to_string())]);
        self.page_messages(
            "ListWorkspaces",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM workspaces WHERE (?1 IS NULL OR state=?1) ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query([state])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            PROJECT_SERVICE,
        )
    }
    pub(super) fn validate_workspaces(&self) -> Result<(), CleanStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT id,state,resource_version,payload FROM workspaces")
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
            let id: String = row.get(0).map_err(CleanStoreError::Database)?;
            let state: i32 = row.get(1).map_err(CleanStoreError::Database)?;
            let version: i64 = row.get(2).map_err(CleanStoreError::Database)?;
            let payload: Vec<u8> = row.get(3).map_err(CleanStoreError::Database)?;
            let workspace: v1::Workspace = decode(&payload)?;
            // Reuse the durable project shape validation without requiring live
            // credentials: archived/frozen history survives provider changes.
            let shape = v1::Project {
                id: workspace.id.clone(),
                display_name: workspace.display_name.clone(),
                state: workspace.state,
                repository_connection_ids: workspace.repository_connection_ids.clone(),
                created_at: workspace.created_at,
                updated_at: workspace.updated_at,
                resource_version: workspace.resource_version,
                agent_defaults: workspace.agent_defaults.clone(),
                ..Default::default()
            };
            if !matches!(workspace.planner_idle_timeout_seconds, 300 | 86400)
                || !super::valid_project(&shape)
                || workspace.id != id
                || workspace.state != state
                || i64::try_from(workspace.resource_version).ok() != Some(version)
            {
                return Err(CleanStoreError::CorruptState);
            }
            for member in &workspace.repository_connection_ids {
                self.repository_connection(member)?;
            }
        }
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM projects")
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
            let payload: Vec<u8> = row.get(0).map_err(CleanStoreError::Database)?;
            let project: v1::Project = decode(&payload)?;
            if super::api_validation::project_goal(&project.goal).is_err() {
                return Err(CleanStoreError::CorruptState);
            }
            self.workspace(&project.workspace_id)?;
        }
        Ok(())
    }
    pub(super) fn workspace_repository_in_use(&self, id: &str) -> Result<bool, CleanStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM workspaces")
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
            let payload: Vec<u8> = row.get(0).map_err(CleanStoreError::Database)?;
            let workspace: v1::Workspace = decode(&payload)?;
            if workspace
                .repository_connection_ids
                .iter()
                .any(|member| member == id)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
