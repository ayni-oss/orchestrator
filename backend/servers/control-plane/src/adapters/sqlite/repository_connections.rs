use super::{
    CleanStore, CleanStoreError, Message, OptionalExtension, Page, REPOSITORY_SERVICE, action_row,
    append_event, bounded_failure, decode, encode_filter, get_payload, map_constraint, params,
    provider_key, query_payloads, request_digest, resource_id, timestamp, unix_seconds, v1,
    valid_repository_transition,
};

impl CleanStore {
    pub(crate) fn create_repository_connection(
        &mut self,
        key: &str,
        request: &v1::CreateRepositoryConnectionRequest,
    ) -> Result<v1::RepositoryConnection, CleanStoreError> {
        let digest = request_digest("CreateRepositoryConnection", request);
        if let Some(existing) = action_row(&self.connection, "CreateRepositoryConnection", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            let id = existing.target_id.ok_or(CleanStoreError::CorruptState)?;
            return self.repository_connection(&id);
        }
        let provider = request
            .provider
            .clone()
            .ok_or(CleanStoreError::CorruptState)?;
        let provider_key = provider_key(&provider)?;
        let now = unix_seconds()?;
        let id = resource_id("repository");
        let mut resource = v1::RepositoryConnection {
            id: id.clone(),
            provider: Some(provider),
            display_name: request.display_name.clone(),
            default_branch: request.default_branch.clone(),
            status: v1::RepositoryConnectionStatus::Connecting as i32,
            failure: None,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('CreateRepositoryConnection',?1,?2,?3,'completed','repository_connection',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), id, now],
            )
            .map_err(map_constraint)?;
        let cursor = append_event(
            &transaction,
            REPOSITORY_SERVICE,
            "repository_connection",
            &id,
            1,
            Some(&id),
            None,
            &resource,
        )?;
        transaction
            .execute(
                "INSERT INTO repository_connections(id,status,provider_key,payload,resource_version,event_cursor)
                 VALUES (?1,?2,?3,?4,1,?5)",
                params![
                    id,
                    resource.status,
                    provider_key,
                    resource.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        // The payload is immutable for this transaction; cursor is an observation identity,
        // not a field in the public resource.
        resource.resource_version = 1;
        Ok(resource)
    }

    pub(crate) fn repository_connection(
        &self,
        id: &str,
    ) -> Result<v1::RepositoryConnection, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM repository_connections WHERE id=?1",
            id,
            "repository_connection",
        )
    }

    pub(crate) fn transition_repository_connection(
        &mut self,
        id: &str,
        expected_version: u64,
        state: v1::RepositoryConnectionStatus,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::RepositoryConnection, CleanStoreError> {
        let mut resource = self.repository_connection(id)?;
        if resource.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let current = v1::RepositoryConnectionStatus::try_from(resource.status)
            .map_err(|_| CleanStoreError::CorruptState)?;
        if !valid_repository_transition(current, state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        resource.status = state as i32;
        resource.failure = bounded_failure(failure);
        resource.updated_at = Some(timestamp(unix_seconds()?));
        resource.resource_version = resource.resource_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            REPOSITORY_SERVICE,
            "repository_connection",
            &resource.id,
            resource.resource_version,
            Some(&resource.id),
            None,
            &resource,
        )?;
        let changed = transaction.execute(
            "UPDATE repository_connections SET status=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
            params![resource.status, resource.encode_to_vec(),
                i64::try_from(resource.resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?, resource.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?],
        ).map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(resource)
    }

    pub(crate) fn list_repository_connections(
        &mut self,
        status: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::RepositoryConnection>, CleanStoreError> {
        let status_text = status.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[&status_text]);
        self.page_messages(
            "ListRepositoryConnections",
            &filter,
            page,
            |connection| query_payloads(connection, "repository_connections", "", status, None),
            REPOSITORY_SERVICE,
        )
    }

    pub(crate) fn observe_repository_snapshot(
        &mut self,
        repository_connection_id: &str,
        revision: &str,
    ) -> Result<v1::RepositorySnapshot, CleanStoreError> {
        self.repository_connection(repository_connection_id)?;
        if let Some(payload) = self
            .connection
            .query_row(
                "SELECT payload FROM repository_snapshots
                 WHERE repository_connection_id=?1 AND revision=?2",
                params![repository_connection_id, revision],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?
        {
            return decode(&payload);
        }
        let connection = self.repository_connection(repository_connection_id)?;
        let id = resource_id("snapshot");
        let snapshot = v1::RepositorySnapshot {
            id: id.clone(),
            repository_connection_id: repository_connection_id.to_owned(),
            revision: revision.to_owned(),
            default_branch: connection.default_branch,
            observed_at: Some(timestamp(unix_seconds()?)),
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            REPOSITORY_SERVICE,
            "repository_snapshot",
            &id,
            1,
            Some(repository_connection_id),
            Some(repository_connection_id),
            &snapshot,
        )?;
        transaction
            .execute(
                "INSERT INTO repository_snapshots(
                    id,repository_connection_id,revision,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,1,?5)",
                params![
                    id,
                    repository_connection_id,
                    revision,
                    snapshot.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(snapshot)
    }

    pub(crate) fn repository_snapshot(
        &self,
        id: &str,
    ) -> Result<v1::RepositorySnapshot, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM repository_snapshots WHERE id=?1",
            id,
            "repository_snapshot",
        )
    }

    pub(crate) fn repository_snapshot_for_revision(
        &self,
        repository_connection_id: &str,
        revision: &str,
    ) -> Result<v1::RepositorySnapshot, CleanStoreError> {
        let payload = self
            .connection
            .query_row(
                "SELECT payload FROM repository_snapshots
                 WHERE repository_connection_id=?1 AND revision=?2",
                params![repository_connection_id, revision],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?
            .ok_or_else(|| CleanStoreError::NotFound {
                resource_type: "repository_snapshot",
                resource_id: revision.to_owned(),
            })?;
        decode(&payload)
    }

    pub(crate) fn list_repository_snapshots(
        &mut self,
        repository_connection_id: &str,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::RepositorySnapshot>, CleanStoreError> {
        self.repository_connection(repository_connection_id)?;
        let filter = encode_filter(&[repository_connection_id]);
        self.page_messages(
            "ListRepositorySnapshots",
            &filter,
            page,
            |connection| {
                query_payloads(
                    connection,
                    "repository_snapshots",
                    "repository_connection_id",
                    None,
                    Some(repository_connection_id),
                )
            },
            REPOSITORY_SERVICE,
        )
    }
}
