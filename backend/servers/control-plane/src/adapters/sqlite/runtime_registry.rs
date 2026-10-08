//! Runtime-owned destinations are independent of repository enrollment and source revisions.
use super::{
    CleanStore, CleanStoreError, Message, PROFILE_SERVICE, action_row, append_event,
    map_constraint, params, request_digest, resource_id, timestamp, unix_seconds, v1,
};

impl CleanStore {
    pub(crate) fn normalize_runtime_destination(
        &mut self,
        profile_id: &str,
    ) -> Result<v1::RuntimeProfile, CleanStoreError> {
        let profile = self.runtime_profile(profile_id)?;
        let Some(desired) = self.desired_runtime_repository(&profile)? else {
            return Ok(profile);
        };
        let revision = profile
            .active_revision_id
            .as_deref()
            .map(|id| self.runtime_profile_revision(id))
            .transpose()?;
        let outdated = matches!(revision.as_ref().and_then(|r|r.configuration.as_ref()),
            Some(v1::runtime_profile_revision::Configuration::AyniManaged(configuration))
                if configuration.oci_repository != desired);
        if !outdated {
            return Ok(profile);
        }
        let selection = self.runtime_registry(&profile)?;
        let key = format!("normalize-{}-{}", profile.id, profile.resource_version);
        self.set_runtime_registry(
            &key,
            &v1::SetRuntimeRegistryRequest {
                idempotency_key: key.clone(),
                runtime_profile_id: profile.id,
                expected_resource_version: profile.resource_version,
                registry_connection_id: selection.registry_connection_id,
            },
        )
    }

    pub(crate) fn validate_runtime_registry(&self, id: &str) -> Result<(), CleanStoreError> {
        if id.is_empty() {
            return Ok(());
        }
        let connection = self.get_connection(id)?;
        if connection.kind != "ghcr"
            || !connection.enabled
            || connection.status != "ready"
            || !crate::registry_probe::valid_namespace(&connection.image_namespace)
        {
            return Err(CleanStoreError::InvalidTransition);
        }
        Ok(())
    }

    pub(crate) fn desired_runtime_repository(
        &self,
        profile: &v1::RuntimeProfile,
    ) -> Result<Option<String>, CleanStoreError> {
        let selection = self.runtime_registry(profile)?;
        if selection.registry_connection_id.is_empty() {
            return Ok(None);
        }
        let connection = match self.get_connection(&selection.registry_connection_id) {
            Ok(connection) => connection,
            Err(CleanStoreError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let repository = self.repository_connection(&profile.repository_connection_id)?;
        let Some(v1::repository_provider::Provider::Github(github)) =
            repository.provider.and_then(|p| p.provider)
        else {
            return Ok(None);
        };
        crate::process::runtime_configuration::connection_image_repository(
            &connection.image_namespace,
            &github.owner,
            &github.name,
        )
        .map(Some)
        .map_err(|_| CleanStoreError::InvalidTransition)
    }

    pub(crate) fn describe_runtime_profile(
        &self,
        mut profile: v1::RuntimeProfile,
    ) -> Result<v1::RuntimeProfile, CleanStoreError> {
        profile.desired_image_repository = self
            .desired_runtime_repository(&profile)?
            .unwrap_or_default();
        profile.effective_certificate = self.effective_runtime_certificate(&profile)?;
        profile.findings.clear();
        let cursor = self.runtime_cursor(&profile.id)?;
        let metadata = cursor.metadata();
        profile.preparation_phase = metadata
            .iter()
            .find(|m| m.key == "phase")
            .map_or_else(|| "analyzing".into(), |m| m.value.clone());
        if let Some(failure) = &profile.failure {
            profile
                .findings
                .push(crate::profiles::ayni::findings::finding(&failure.message));
            profile.rebuild_available =
                crate::profiles::ayni::findings::rebuild_allowed(&failure.message);
            if crate::profiles::ayni::findings::setup_required(&failure.message) {
                profile.preparation_phase = "setup_required".into();
            }
        } else {
            profile.rebuild_available = profile.state == v1::RuntimeProfileState::Ready as i32;
        }
        if profile.effective_certificate.is_none() {
            profile
                .findings
                .push(crate::profiles::ayni::findings::finding(
                    "certificate_setup_required",
                ));
            profile.rebuild_available = false;
            profile.preparation_phase = "setup_required".into();
        }
        if profile.desired_image_repository.is_empty() {
            profile
                .findings
                .push(crate::profiles::ayni::findings::finding(
                    "registry_connection_missing",
                ));
            profile.rebuild_available = false;
            profile.preparation_phase = "setup_required".into();
        }
        Ok(profile)
    }

    pub(crate) fn runtime_registry(
        &self,
        profile: &v1::RuntimeProfile,
    ) -> Result<v1::RuntimeRegistrySelection, CleanStoreError> {
        if let Some(selection) = &profile.registry {
            return Ok(selection.clone());
        }
        // Schema-six payloads remain readable; absent fields inherit the existing default.
        let repository = self.repository_connection(&profile.repository_connection_id)?;
        Ok(v1::RuntimeRegistrySelection {
            registry_connection_id: repository.registry_connection_id,
        })
    }

    pub(super) fn registry_replacement_revision(
        &self,
        profile: &v1::RuntimeProfile,
        desired: Option<&String>,
        now: i64,
    ) -> Result<v1::RuntimeProfileRevision, CleanStoreError> {
        let mut revision = self.runtime_profile_revision(
            profile
                .active_revision_id
                .as_deref()
                .ok_or(CleanStoreError::CorruptState)?,
        )?;
        let number: i64 = self.connection.query_row(
                "SELECT MAX(revision_number)+1 FROM runtime_profile_revisions WHERE runtime_profile_id=?1",
                [&profile.id], |row| row.get(0),
            ).map_err(CleanStoreError::Database)?;
        revision.id = resource_id("profile-revision");
        revision.revision_number =
            u64::try_from(number).map_err(|_| CleanStoreError::CorruptState)?;
        if let Some(v1::runtime_profile_revision::Configuration::AyniManaged(configuration)) =
            revision.configuration.as_mut()
        {
            if let Some(desired) = desired {
                configuration.oci_repository.clone_from(desired);
            }
            configuration.runtime_source_digest = None;
        }
        revision.created_at = Some(timestamp(now));
        revision.resource_version = 1;
        Ok(revision)
    }

    pub(crate) fn set_runtime_registry(
        &mut self,
        key: &str,
        request: &v1::SetRuntimeRegistryRequest,
    ) -> Result<v1::RuntimeProfile, CleanStoreError> {
        let digest = request_digest("SetRuntimeRegistry", request);
        if let Some(existing) = action_row(&self.connection, "SetRuntimeRegistry", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return self.runtime_profile(&existing.target_id.ok_or(CleanStoreError::CorruptState)?);
        }
        let mut profile = self.runtime_profile(&request.runtime_profile_id)?;
        if profile.resource_version != request.expected_resource_version {
            return Err(CleanStoreError::StaleVersion);
        }
        if profile.kind != v1::RuntimeProfileKind::AyniManaged as i32 {
            return Err(CleanStoreError::InvalidTransition);
        }
        self.validate_runtime_registry(&request.registry_connection_id)?;
        let selection = v1::RuntimeRegistrySelection {
            registry_connection_id: request.registry_connection_id.clone(),
        };
        let old_selection = self.runtime_registry(&profile)?;
        let selected_revision = profile
            .active_revision_id
            .as_deref()
            .map(|id| self.runtime_profile_revision(id))
            .transpose()?;
        let mut next_profile = profile.clone();
        next_profile.registry = Some(selection.clone());
        let desired = self.desired_runtime_repository(&next_profile)?;
        let outdated_destination = desired.as_ref().is_some_and(|desired| {
            matches!(selected_revision.as_ref().and_then(|r|r.configuration.as_ref()),
                Some(v1::runtime_profile_revision::Configuration::AyniManaged(configuration))
                    if configuration.oci_repository != *desired)
        });
        let changed = old_selection != selection || outdated_destination;
        let now = unix_seconds()?;
        let mut revision = if changed {
            Some(self.registry_replacement_revision(&profile, desired.as_ref(), now)?)
        } else {
            None
        };
        profile.registry = Some(selection);
        if let Some(revision) = &revision {
            // Retire admission authority immediately. A frozen pending Job can finish,
            // but its old revision can never reactivate the changed destination.
            profile.active_revision_id = Some(revision.id.clone());
            profile.active_runtime_artifact_id = None;
            profile.state = v1::RuntimeProfileState::Updating as i32;
            profile.failure = None;
        }
        profile.resource_version = profile.resource_version.saturating_add(1);
        profile.updated_at = Some(timestamp(now));
        let tx = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        if let Some(revision) = revision.take() {
            let cursor = append_event(
                &tx,
                PROFILE_SERVICE,
                "runtime_profile_revision",
                &revision.id,
                1,
                Some(&profile.repository_connection_id),
                Some(&profile.id),
                &revision,
            )?;
            tx.execute("INSERT INTO runtime_profile_revisions(id,runtime_profile_id,repository_snapshot_id,revision_number,payload,resource_version,event_cursor) VALUES(?1,?2,?3,?4,?5,1,?6)", params![revision.id,profile.id,revision.repository_snapshot_id,i64::try_from(revision.revision_number).map_err(|_| CleanStoreError::CorruptState)?,revision.encode_to_vec(),i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?]).map_err(map_constraint)?;
        }
        let cursor = append_event(
            &tx,
            PROFILE_SERVICE,
            "runtime_profile",
            &profile.id,
            profile.resource_version,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &profile,
        )?;
        let updated = tx.execute("UPDATE runtime_profiles SET state=?1,active_revision_id=?2,active_runtime_artifact_id=?3,payload=?4,resource_version=?5,event_cursor=?6 WHERE id=?7 AND resource_version=?8",params![profile.state,profile.active_revision_id,profile.active_runtime_artifact_id,profile.encode_to_vec(),i64::try_from(profile.resource_version).map_err(|_| CleanStoreError::CorruptState)?,i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,profile.id,i64::try_from(request.expected_resource_version).map_err(|_| CleanStoreError::CorruptState)?]).map_err(CleanStoreError::Database)?;
        if updated != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        // Wake reconciliation without replacing the persisted frozen Job intent.
        tx.execute("UPDATE reconciliation_cursors SET value=json_set(value,'$.next_at',0),resource_version=resource_version+1,updated_at=?1 WHERE name=?2", params![now,format!("managed:{}",profile.id)]).map_err(CleanStoreError::Database)?;
        tx.execute("INSERT INTO durable_actions(operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at) VALUES('SetRuntimeRegistry',?1,?2,?3,'completed','runtime_profile',?4,?5,?5)",params![key,digest,request.encode_to_vec(),profile.id,now]).map_err(map_constraint)?;
        tx.commit().map_err(CleanStoreError::Database)?;
        Ok(profile)
    }
}
