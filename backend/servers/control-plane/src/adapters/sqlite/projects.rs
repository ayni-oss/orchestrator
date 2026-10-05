use super::{
    CleanStore, CleanStoreError, Message, PROJECT_SERVICE, Page, action_row, append_event,
    collect_bounded_payloads, encode_filter, get_payload, params, request_digest, resource_id,
    timestamp, unix_seconds, v1,
};

impl CleanStore {
    pub(super) fn project_task_repository(
        &self,
        request: &v1::DevelopmentTaskRequest,
    ) -> Result<v1::RepositoryConnection, CleanStoreError> {
        let id = self.project_member(&request.project_id, &request.repository_connection_id)?;
        self.repository_connection(&id)
    }

    pub(crate) fn project(&self, id: &str) -> Result<v1::Project, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM projects WHERE id=?1",
            id,
            "project",
        )
    }

    pub(crate) fn project_member(
        &self,
        id: &str,
        repository: &str,
    ) -> Result<String, CleanStoreError> {
        let project = self.project(id)?;
        if project.state != v1::ProjectState::Active as i32 {
            return Err(CleanStoreError::ProjectArchived);
        }
        // Legacy callers may omit the repository only for an unambiguous project.
        if repository.is_empty() && project.repository_connection_ids.len() == 1 {
            return Ok(project.repository_connection_ids[0].clone());
        }
        if !project
            .repository_connection_ids
            .iter()
            .any(|member| member == repository)
        {
            return Err(CleanStoreError::ParentMismatch);
        }
        Ok(repository.to_owned())
    }

    pub(super) fn replay_project(
        &self,
        operation: &str,
        key: &str,
        digest: &[u8],
    ) -> Result<Option<v1::Project>, CleanStoreError> {
        let Some(action) = action_row(&self.connection, operation, key)? else {
            return Ok(None);
        };
        if action.request_hash != digest {
            return Err(CleanStoreError::IdempotencyConflict);
        }
        self.project(&action.target_id.ok_or(CleanStoreError::CorruptState)?)
            .map(Some)
    }

    pub(crate) fn create_project(
        &mut self,
        key: &str,
        request: &v1::CreateProjectRequest,
    ) -> Result<v1::Project, CleanStoreError> {
        let digest = request_digest("CreateProject", request);
        if let Some(project) = self.replay_project("CreateProject", key, &digest)? {
            return Ok(project);
        }
        let repositories = if request.repository_connection_ids.is_empty() {
            vec![request.repository_connection_id.clone()]
        } else {
            if !request.repository_connection_id.is_empty() {
                return Err(CleanStoreError::ParentMismatch);
            }
            request.repository_connection_ids.clone()
        };
        api_validation_members(&repositories)?;
        for repository in &repositories {
            self.repository_connection(repository)?;
        }
        let now = unix_seconds()?;
        let project = v1::Project {
            id: resource_id("project"),
            display_name: request.display_name.clone(),
            state: v1::ProjectState::Active as i32,
            repository_connection_ids: repositories,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        self.save_project("CreateProject", key, request, &project, true)?;
        Ok(project)
    }

    pub(crate) fn update_project(
        &mut self,
        key: &str,
        request: &v1::UpdateProjectRequest,
    ) -> Result<v1::Project, CleanStoreError> {
        let digest = request_digest("UpdateProject", request);
        if let Some(project) = self.replay_project("UpdateProject", key, &digest)? {
            return Ok(project);
        }
        let mut project =
            self.editable_project(&request.project_id, request.expected_resource_version)?;
        project.display_name.clone_from(&request.display_name);
        if !request.repository_connection_ids.is_empty() {
            api_validation_members(&request.repository_connection_ids)?;
            for repository in &request.repository_connection_ids {
                self.repository_connection(repository)?;
            }
            project
                .repository_connection_ids
                .clone_from(&request.repository_connection_ids);
        }
        self.save_project("UpdateProject", key, request, &project, false)?;
        Ok(project)
    }

    pub(crate) fn archive_project(
        &mut self,
        key: &str,
        request: &v1::ArchiveProjectRequest,
    ) -> Result<v1::Project, CleanStoreError> {
        let digest = request_digest("ArchiveProject", request);
        if let Some(project) = self.replay_project("ArchiveProject", key, &digest)? {
            return Ok(project);
        }
        let mut project =
            self.editable_project(&request.project_id, request.expected_resource_version)?;
        project.state = v1::ProjectState::Archived as i32;
        self.save_project("ArchiveProject", key, request, &project, false)?;
        Ok(project)
    }

    pub(super) fn editable_project(
        &self,
        id: &str,
        version: u64,
    ) -> Result<v1::Project, CleanStoreError> {
        let mut project = self.project(id)?;
        if project.resource_version != version {
            return Err(CleanStoreError::StaleVersion);
        }
        if project.state != v1::ProjectState::Active as i32 {
            return Err(CleanStoreError::ProjectArchived);
        }
        project.resource_version = project
            .resource_version
            .checked_add(1)
            .ok_or(CleanStoreError::BoundExceeded)?;
        project.updated_at = Some(timestamp(unix_seconds()?));
        Ok(project)
    }

    pub(super) fn save_project(
        &mut self,
        operation: &str,
        key: &str,
        request: &impl Message,
        project: &v1::Project,
        create: bool,
    ) -> Result<(), CleanStoreError> {
        let now = unix_seconds()?;
        let digest = request_digest(operation, request);
        let repository = project
            .repository_connection_ids
            .first()
            .ok_or(CleanStoreError::CorruptState)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            PROJECT_SERVICE,
            "project",
            &project.id,
            project.resource_version,
            Some(repository),
            Some(&project.id),
            project,
        )?;
        let version =
            i64::try_from(project.resource_version).map_err(|_| CleanStoreError::BoundExceeded)?;
        let cursor = i64::try_from(cursor).map_err(|_| CleanStoreError::BoundExceeded)?;
        if create {
            transaction.execute("INSERT INTO projects(id,state,payload,resource_version,event_cursor) VALUES (?1,?2,?3,?4,?5)",
                params![project.id, project.state, project.encode_to_vec(), version, cursor]).map_err(CleanStoreError::Database)?;
        } else {
            let changed = transaction.execute("UPDATE projects SET state=?2,payload=?3,resource_version=?4,event_cursor=?5 WHERE id=?1 AND resource_version=?6",
                params![project.id, project.state, project.encode_to_vec(), version, cursor, version-1]).map_err(CleanStoreError::Database)?;
            if changed != 1 {
                return Err(CleanStoreError::StaleVersion);
            }
            transaction
                .execute(
                    "UPDATE project_repositories SET active=?2 WHERE project_id=?1",
                    params![project.id, 0],
                )
                .map_err(CleanStoreError::Database)?;
        }
        for member in &project.repository_connection_ids {
            transaction.execute("INSERT INTO project_repositories(project_id,repository_connection_id,active) VALUES (?1,?2,?3) ON CONFLICT(project_id,repository_connection_id) DO UPDATE SET active=excluded.active",
                params![project.id, member, i32::from(project.state == v1::ProjectState::Active as i32)]).map_err(CleanStoreError::Database)?;
        }
        transaction.execute("INSERT INTO durable_actions(operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at) VALUES (?1,?2,?3,?4,'completed','project',?5,?6,?6)",
            params![operation,key,digest,request.encode_to_vec(),project.id,now]).map_err(CleanStoreError::Database)?;
        transaction.commit().map_err(CleanStoreError::Database)
    }

    pub(crate) fn list_projects(
        &mut self,
        state: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::Project>, CleanStoreError> {
        let filter = encode_filter(&[&state.map_or_else(String::new, |v| v.to_string())]);
        self.page_messages(
            "ListProjects",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM projects WHERE (?1 IS NULL OR state=?1) ORDER BY id",
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
}

fn api_validation_members(repositories: &[String]) -> Result<(), CleanStoreError> {
    super::api_validation::project_members(repositories)
        .map_err(|_| CleanStoreError::ParentMismatch)
}
