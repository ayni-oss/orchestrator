//! Runtime-owned destinations are independent of repository enrollment and source revisions.
use super::{
    CleanStore, CleanStoreError, Message, PROFILE_SERVICE, action_row, append_event,
    map_constraint, params, request_digest, resource_id, timestamp, unix_seconds, v1,
};

impl CleanStore {
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
            image_namespace: repository.image_namespace,
        })
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
        self.validate_registry_selection(
            &request.registry_connection_id,
            &request.image_namespace,
        )?;
        let selection = v1::RuntimeRegistrySelection {
            registry_connection_id: request.registry_connection_id.clone(),
            image_namespace: request.image_namespace.clone(),
        };
        let changed = self.runtime_registry(&profile)? != selection;
        let now = unix_seconds()?;
        let mut revision = if changed {
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
            revision.created_at = Some(timestamp(now));
            revision.resource_version = 1;
            Some(revision)
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
