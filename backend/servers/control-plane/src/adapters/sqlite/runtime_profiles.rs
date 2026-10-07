#[cfg(test)]
use super::transitions_schema::valid_artifact_transition;
use super::{
    CleanStore, CleanStoreError, Message, OptionalExtension, PROFILE_SERVICE, Page, action_row,
    append_event, bounded_failure, collect_bounded_payloads, decode, encode_filter, get_payload,
    map_constraint, params, query_payloads, request_digest, resource_id, same_artifact_report,
    timestamp, unix_seconds, v1, valid_artifact_report, valid_profile_transition,
};

impl CleanStore {
    pub(crate) fn create_runtime_profile(
        &mut self,
        key: &str,
        request: &v1::CreateRuntimeProfileRequest,
    ) -> Result<(v1::RuntimeProfile, v1::RuntimeProfileRevision), CleanStoreError> {
        let digest = request_digest("CreateRuntimeProfile", request);
        if let Some(existing) = action_row(&self.connection, "CreateRuntimeProfile", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            let profile_id = existing.target_id.ok_or(CleanStoreError::CorruptState)?;
            let profile = self.runtime_profile(&profile_id)?;
            let revision: v1::RuntimeProfileRevision = get_payload(
                &self.connection,
                "SELECT payload FROM runtime_profile_revisions
                 WHERE runtime_profile_id=?1 AND revision_number=1",
                &profile_id,
                "runtime_profile_revision",
            )?;
            return Ok((profile, revision));
        }
        let repository = self.repository_connection(&request.repository_connection_id)?;
        let snapshot = self.repository_snapshot(&request.repository_snapshot_id)?;
        if snapshot.repository_connection_id != repository.id {
            return Err(CleanStoreError::ParentMismatch);
        }
        let configuration = request
            .configuration
            .as_ref()
            .and_then(|value| value.configuration.clone())
            .ok_or(CleanStoreError::CorruptState)?;
        let (kind, revision_configuration) = match configuration {
            v1::runtime_profile_configuration::Configuration::AyniManaged(value) => (
                v1::RuntimeProfileKind::AyniManaged,
                v1::runtime_profile_revision::Configuration::AyniManaged(value),
            ),
            v1::runtime_profile_configuration::Configuration::CustomRuntime(value) => (
                v1::RuntimeProfileKind::CustomRuntime,
                v1::runtime_profile_revision::Configuration::CustomRuntime(value),
            ),
        };
        if let v1::runtime_profile_revision::Configuration::AyniManaged(requested) =
            &revision_configuration
        {
            self.ensure_managed_lock_available(&repository.id, &requested.lock_fingerprint)?;
        }
        let registry = if repository.registry_connection_id.is_empty()
            || match self.get_connection(&repository.registry_connection_id) {
                Ok(_) => true,
                Err(CleanStoreError::NotFound { .. }) => false,
                Err(error) => return Err(error),
            } {
            v1::RuntimeRegistrySelection {
                registry_connection_id: repository.registry_connection_id.clone(),
            }
        } else {
            v1::RuntimeRegistrySelection::default()
        };
        let now = unix_seconds()?;
        let profile_id = resource_id("profile");
        let revision_id = resource_id("profile-revision");
        let operational = kind == v1::RuntimeProfileKind::AyniManaged;
        let profile = v1::RuntimeProfile {
            reconciliation_metadata: vec![],
            registry: Some(registry),
            desired_image_repository: String::new(),
            id: profile_id.clone(),
            repository_connection_id: repository.id.clone(),
            kind: kind as i32,
            support: if operational {
                v1::RuntimeProfileSupport::Operational as i32
            } else {
                v1::RuntimeProfileSupport::Unsupported as i32
            },
            state: if operational {
                v1::RuntimeProfileState::SetupRequired as i32
            } else {
                v1::RuntimeProfileState::Failed as i32
            },
            active_revision_id: Some(revision_id.clone()),
            active_runtime_artifact_id: None,
            failure: None,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let revision = v1::RuntimeProfileRevision {
            id: revision_id.clone(),
            runtime_profile_id: profile_id.clone(),
            repository_snapshot_id: snapshot.id,
            repository_revision: snapshot.revision,
            revision_number: 1,
            configuration: Some(revision_configuration),
            created_at: Some(timestamp(now)),
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
                 ) VALUES ('CreateRuntimeProfile',?1,?2,?3,'completed','runtime_profile',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), profile_id, now],
            )
            .map_err(map_constraint)?;
        let profile_cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_profile",
            &profile_id,
            1,
            Some(&repository.id),
            Some(&profile_id),
            &profile,
        )?;
        transaction
            .execute(
                "INSERT INTO runtime_profiles(
                    id,repository_connection_id,kind,state,active_revision_id,
                    active_runtime_artifact_id,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,NULL,?6,1,?7)",
                params![
                    profile_id,
                    repository.id,
                    profile.kind,
                    profile.state,
                    revision_id,
                    profile.encode_to_vec(),
                    i64::try_from(profile_cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        let revision_cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_profile_revision",
            &revision_id,
            1,
            Some(&profile.repository_connection_id),
            Some(&profile_id),
            &revision,
        )?;
        transaction
            .execute(
                "INSERT INTO runtime_profile_revisions(
                    id,runtime_profile_id,repository_snapshot_id,revision_number,payload,
                    resource_version,event_cursor
                 ) VALUES (?1,?2,?3,1,?4,1,?5)",
                params![
                    revision_id,
                    profile_id,
                    revision.repository_snapshot_id,
                    revision.encode_to_vec(),
                    i64::try_from(revision_cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok((profile, revision))
    }

    pub(crate) fn runtime_profile(&self, id: &str) -> Result<v1::RuntimeProfile, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM runtime_profiles WHERE id=?1",
            id,
            "runtime_profile",
        )
    }

    pub(crate) fn transition_runtime_profile(
        &mut self,
        id: &str,
        expected_version: u64,
        state: v1::RuntimeProfileState,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::RuntimeProfile, CleanStoreError> {
        let mut resource = self.runtime_profile(id)?;
        if resource.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let current = v1::RuntimeProfileState::try_from(resource.state)
            .map_err(|_| CleanStoreError::CorruptState)?;
        if !valid_profile_transition(current, state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        resource.state = state as i32;
        resource.failure = bounded_failure(failure);
        resource.updated_at = Some(timestamp(unix_seconds()?));
        resource.resource_version = resource.resource_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_profile",
            &resource.id,
            resource.resource_version,
            Some(&resource.repository_connection_id),
            Some(&resource.id),
            &resource,
        )?;
        let changed = transaction.execute(
            "UPDATE runtime_profiles SET state=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
            params![resource.state, resource.encode_to_vec(),
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

    pub(crate) fn list_runtime_profiles(
        &mut self,
        repository_connection_id: Option<&str>,
        kind: Option<i32>,
        state: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::RuntimeProfile>, CleanStoreError> {
        let kind_text = kind.map_or_else(String::new, |value| value.to_string());
        let state_text = state.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[
            repository_connection_id.unwrap_or(""),
            &kind_text,
            &state_text,
        ]);
        self.page_messages(
            "ListRuntimeProfiles",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM runtime_profiles
                         WHERE (?1 IS NULL OR repository_connection_id=?1)
                           AND (?2 IS NULL OR kind=?2)
                           AND (?3 IS NULL OR state=?3)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![repository_connection_id, kind, state])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            PROFILE_SERVICE,
        )
    }

    pub(crate) fn runtime_profile_revision(
        &self,
        id: &str,
    ) -> Result<v1::RuntimeProfileRevision, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM runtime_profile_revisions WHERE id=?1",
            id,
            "runtime_profile_revision",
        )
    }

    pub(crate) fn list_runtime_profile_revisions(
        &mut self,
        profile_id: &str,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::RuntimeProfileRevision>, CleanStoreError> {
        self.runtime_profile(profile_id)?;
        let filter = encode_filter(&[profile_id]);
        self.page_messages(
            "ListRuntimeProfileRevisions",
            &filter,
            page,
            |connection| {
                query_payloads(
                    connection,
                    "runtime_profile_revisions",
                    "runtime_profile_id",
                    None,
                    Some(profile_id),
                )
            },
            PROFILE_SERVICE,
        )
    }

    #[cfg(test)]
    pub(crate) fn record_runtime_profile_revision(
        &mut self,
        profile_id: &str,
        repository_snapshot_id: &str,
        configuration: v1::runtime_profile_revision::Configuration,
    ) -> Result<v1::RuntimeProfileRevision, CleanStoreError> {
        self.record_selected_runtime_revision(
            profile_id,
            repository_snapshot_id,
            configuration,
            true,
        )
    }

    pub(super) fn record_selected_runtime_revision(
        &mut self,
        profile_id: &str,
        repository_snapshot_id: &str,
        configuration: v1::runtime_profile_revision::Configuration,
        replay_historical: bool,
    ) -> Result<v1::RuntimeProfileRevision, CleanStoreError> {
        let mut profile = self.runtime_profile(profile_id)?;
        let snapshot = self.repository_snapshot(repository_snapshot_id)?;
        if snapshot.repository_connection_id != profile.repository_connection_id {
            return Err(CleanStoreError::ParentMismatch);
        }
        let expected_kind = match &configuration {
            v1::runtime_profile_revision::Configuration::AyniManaged(_) => {
                v1::RuntimeProfileKind::AyniManaged as i32
            }
            v1::runtime_profile_revision::Configuration::CustomRuntime(_) => {
                v1::RuntimeProfileKind::CustomRuntime as i32
            }
        };
        if profile.kind != expected_kind {
            return Err(CleanStoreError::ParentMismatch);
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT payload FROM runtime_profile_revisions
                 WHERE runtime_profile_id=?1 ORDER BY revision_number DESC",
            )
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement
            .query([profile_id])
            .map_err(CleanStoreError::Database)?;
        let payloads = collect_bounded_payloads(&mut rows)?;
        drop(rows);
        drop(statement);
        let revisions = payloads
            .iter()
            .map(|payload| decode::<v1::RuntimeProfileRevision>(payload))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(existing) = revisions.iter().find(|existing| {
            existing.repository_snapshot_id == repository_snapshot_id
                && existing.configuration.as_ref() == Some(&configuration)
                && (replay_historical
                    || profile.active_revision_id.as_deref() == Some(existing.id.as_str()))
        }) {
            return Ok(existing.clone());
        }
        let revision_number = revisions
            .first()
            .map_or(1, |value| value.revision_number.saturating_add(1));
        let id = resource_id("profile-revision");
        let revision = v1::RuntimeProfileRevision {
            id: id.clone(),
            runtime_profile_id: profile.id.clone(),
            repository_snapshot_id: snapshot.id,
            repository_revision: snapshot.revision,
            revision_number,
            configuration: Some(configuration),
            created_at: Some(timestamp(unix_seconds()?)),
            resource_version: 1,
        };
        let previous_version = profile.resource_version;
        profile.active_revision_id = Some(id.clone());
        profile.active_runtime_artifact_id = None;
        profile.state = v1::RuntimeProfileState::Updating as i32;
        profile.failure = None;
        profile.updated_at = Some(timestamp(unix_seconds()?));
        profile.resource_version = previous_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let revision_cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_profile_revision",
            &id,
            1,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &revision,
        )?;
        transaction
            .execute(
                "INSERT INTO runtime_profile_revisions(
                id,runtime_profile_id,repository_snapshot_id,revision_number,payload,
                resource_version,event_cursor
             ) VALUES (?1,?2,?3,?4,?5,1,?6)",
                params![
                    id,
                    profile.id,
                    revision.repository_snapshot_id,
                    i64::try_from(revision_number).map_err(|_| CleanStoreError::CorruptState)?,
                    revision.encode_to_vec(),
                    i64::try_from(revision_cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        let profile_cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_profile",
            &profile.id,
            profile.resource_version,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &profile,
        )?;
        let changed = transaction
            .execute(
                "UPDATE runtime_profiles SET state=?1,active_revision_id=?2,
                active_runtime_artifact_id=NULL,payload=?3,resource_version=?4,event_cursor=?5
             WHERE id=?6 AND resource_version=?7",
                params![
                    profile.state,
                    profile.active_revision_id,
                    profile.encode_to_vec(),
                    i64::try_from(profile.resource_version)
                        .map_err(|_| CleanStoreError::CorruptState)?,
                    i64::try_from(profile_cursor).map_err(|_| CleanStoreError::CorruptState)?,
                    profile.id,
                    i64::try_from(previous_version).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(revision)
    }

    pub(crate) fn record_runtime_artifact(
        &mut self,
        profile_id: &str,
        mut artifact: v1::RuntimeArtifact,
    ) -> Result<v1::RuntimeArtifact, CleanStoreError> {
        let mut profile = self.runtime_profile(profile_id)?;
        let revision = self.runtime_profile_revision(&artifact.runtime_profile_revision_id)?;
        if revision.runtime_profile_id != profile.id {
            return Err(CleanStoreError::ParentMismatch);
        }
        artifact.failure = bounded_failure(artifact.failure);
        if !valid_artifact_report(&artifact) {
            return Err(CleanStoreError::BoundExceeded);
        }
        let platform = artifact
            .platform
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        let platform_key = format!(
            "{}/{}/{}",
            platform.operating_system,
            platform.architecture,
            platform.variant.as_deref().unwrap_or("")
        );
        let existing: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT payload FROM runtime_artifacts
                 WHERE runtime_profile_revision_id=?1 AND resolved_digest=?2 AND platform_key=?3",
                params![
                    artifact.runtime_profile_revision_id,
                    artifact.resolved_digest,
                    platform_key
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        if let Some(payload) = existing {
            let existing = decode::<v1::RuntimeArtifact>(&payload)?;
            return if same_artifact_report(&existing, &artifact) {
                Ok(existing)
            } else {
                Err(CleanStoreError::IdempotencyConflict)
            };
        }
        if artifact.id.is_empty() {
            artifact.id = resource_id("artifact");
        }
        artifact.resource_version = 1;
        if artifact.created_at.is_none() {
            artifact.created_at = Some(timestamp(unix_seconds()?));
        }
        let verified = artifact.status == v1::RuntimeArtifactStatus::Verified as i32;
        if verified && artifact.verified_at.is_none() {
            artifact.verified_at = Some(timestamp(unix_seconds()?));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_artifact",
            &artifact.id,
            1,
            Some(&profile.repository_connection_id),
            Some(profile_id),
            &artifact,
        )?;
        transaction
            .execute(
                "INSERT INTO runtime_artifacts(
                    id,runtime_profile_id,runtime_profile_revision_id,status,resolved_digest,
                    platform_key,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8)",
                params![
                    artifact.id,
                    profile_id,
                    artifact.runtime_profile_revision_id,
                    artifact.status,
                    artifact.resolved_digest,
                    platform_key,
                    artifact.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        if verified
            && profile.active_revision_id.as_deref() == Some(revision.id.as_str())
            && profile.active_runtime_artifact_id.is_none()
        {
            profile.active_runtime_artifact_id = Some(artifact.id.clone());
            profile.active_revision_id = Some(revision.id);
            profile.state = v1::RuntimeProfileState::Ready as i32;
            profile.failure = None;
            profile.updated_at = Some(timestamp(unix_seconds()?));
            profile.resource_version = profile.resource_version.saturating_add(1);
            let profile_cursor = append_event(
                &transaction,
                PROFILE_SERVICE,
                "runtime_profile",
                &profile.id,
                profile.resource_version,
                Some(&profile.repository_connection_id),
                Some(&profile.id),
                &profile,
            )?;
            let changed = transaction
                .execute(
                    "UPDATE runtime_profiles SET state=?1,active_revision_id=?2,
                        active_runtime_artifact_id=?3,payload=?4,resource_version=?5,event_cursor=?6
                     WHERE id=?7 AND resource_version=?8",
                    params![
                        profile.state,
                        profile.active_revision_id,
                        profile.active_runtime_artifact_id,
                        profile.encode_to_vec(),
                        i64::try_from(profile.resource_version)
                            .map_err(|_| CleanStoreError::CorruptState)?,
                        i64::try_from(profile_cursor).map_err(|_| CleanStoreError::CorruptState)?,
                        profile.id,
                        i64::try_from(profile.resource_version.saturating_sub(1))
                            .map_err(|_| CleanStoreError::CorruptState)?
                    ],
                )
                .map_err(CleanStoreError::Database)?;
            if changed != 1 {
                return Err(CleanStoreError::StaleVersion);
            }
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(artifact)
    }

    pub(crate) fn runtime_artifact(
        &self,
        id: &str,
    ) -> Result<v1::RuntimeArtifact, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM runtime_artifacts WHERE id=?1",
            id,
            "runtime_artifact",
        )
    }

    #[cfg(test)]
    pub(crate) fn transition_runtime_artifact(
        &mut self,
        id: &str,
        expected_version: u64,
        state: v1::RuntimeArtifactStatus,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::RuntimeArtifact, CleanStoreError> {
        let mut resource = self.runtime_artifact(id)?;
        if resource.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let current = v1::RuntimeArtifactStatus::try_from(resource.status)
            .map_err(|_| CleanStoreError::CorruptState)?;
        if !valid_artifact_transition(current, state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        let profile_id: String = self
            .connection
            .query_row(
                "SELECT runtime_profile_id FROM runtime_artifacts WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        let mut profile = self.runtime_profile(&profile_id)?;
        resource.status = state as i32;
        resource.failure = bounded_failure(failure);
        if state == v1::RuntimeArtifactStatus::Verified && resource.verified_at.is_none() {
            resource.verified_at = Some(timestamp(unix_seconds()?));
        }
        resource.resource_version = resource.resource_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "runtime_artifact",
            &resource.id,
            resource.resource_version,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &resource,
        )?;
        let changed = transaction.execute(
            "UPDATE runtime_artifacts SET status=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
            params![resource.status, resource.encode_to_vec(),
                i64::try_from(resource.resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?, resource.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?],
        ).map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        let activates_current = state == v1::RuntimeArtifactStatus::Verified
            && profile.active_revision_id.as_deref()
                == Some(resource.runtime_profile_revision_id.as_str())
            && profile.active_runtime_artifact_id.is_none();
        let invalidates_current = state == v1::RuntimeArtifactStatus::Unavailable
            && profile.active_runtime_artifact_id.as_deref() == Some(resource.id.as_str());
        if activates_current || invalidates_current {
            let previous_profile_version = profile.resource_version;
            if activates_current {
                profile.active_runtime_artifact_id = Some(resource.id.clone());
                profile.state = v1::RuntimeProfileState::Ready as i32;
                profile.failure = None;
            } else {
                profile.active_runtime_artifact_id = None;
                profile.state = v1::RuntimeProfileState::Degraded as i32;
                profile.failure.clone_from(&resource.failure);
            }
            profile.updated_at = Some(timestamp(unix_seconds()?));
            profile.resource_version = previous_profile_version.saturating_add(1);
            let profile_cursor = append_event(
                &transaction,
                PROFILE_SERVICE,
                "runtime_profile",
                &profile.id,
                profile.resource_version,
                Some(&profile.repository_connection_id),
                Some(&profile.id),
                &profile,
            )?;
            let profile_changed = transaction
                .execute(
                    "UPDATE runtime_profiles SET state=?1,active_runtime_artifact_id=?2,
                        payload=?3,resource_version=?4,event_cursor=?5
                     WHERE id=?6 AND resource_version=?7",
                    params![
                        profile.state,
                        profile.active_runtime_artifact_id,
                        profile.encode_to_vec(),
                        i64::try_from(profile.resource_version)
                            .map_err(|_| CleanStoreError::CorruptState)?,
                        i64::try_from(profile_cursor).map_err(|_| CleanStoreError::CorruptState)?,
                        profile.id,
                        i64::try_from(previous_profile_version)
                            .map_err(|_| CleanStoreError::CorruptState)?
                    ],
                )
                .map_err(CleanStoreError::Database)?;
            if profile_changed != 1 {
                return Err(CleanStoreError::StaleVersion);
            }
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(resource)
    }

    pub(crate) fn list_runtime_artifacts(
        &mut self,
        profile_id: &str,
        revision_id: Option<&str>,
        status: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::RuntimeArtifact>, CleanStoreError> {
        self.runtime_profile(profile_id)?;
        let status_text = status.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[profile_id, revision_id.unwrap_or(""), &status_text]);
        self.page_messages(
            "ListRuntimeArtifacts",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM runtime_artifacts
                         WHERE runtime_profile_id=?1
                           AND (?2 IS NULL OR runtime_profile_revision_id=?2)
                           AND (?3 IS NULL OR status=?3)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![profile_id, revision_id, status])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            PROFILE_SERVICE,
        )
    }
}
