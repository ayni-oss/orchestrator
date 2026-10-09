//! Runtime signing selection retires mutable readiness while preserving frozen execution.
use super::{
    CleanStore, CleanStoreError, Message, PROFILE_SERVICE, action_row, append_event,
    map_constraint, params, request_digest, timestamp, unix_seconds, v1,
};

impl CleanStore {
    pub(crate) fn effective_runtime_certificate(
        &self,
        profile: &v1::RuntimeProfile,
    ) -> Result<Option<v1::Certificate>, CleanStoreError> {
        if profile.kind != v1::RuntimeProfileKind::AyniManaged as i32 {
            return Ok(None);
        }
        if let Some(id) = profile
            .certificate_id
            .as_deref()
            .filter(|id| !id.is_empty())
        {
            return match self.get_certificate(id) {
                Ok(certificate) => Ok(Some(certificate)),
                Err(CleanStoreError::NotFound { .. }) => Ok(None),
                Err(error) => Err(error),
            };
        }
        Ok(self
            .list_certificates()?
            .into_iter()
            .find(|certificate| certificate.is_default))
    }

    pub(crate) fn certificate_runtime_usage(&self, id: &str) -> Result<u64, CleanStoreError> {
        let mut query = self
            .connection
            .prepare("SELECT payload FROM runtime_profiles")
            .map_err(CleanStoreError::Database)?;
        let rows = query
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?;
        let mut count = 0;
        for row in rows {
            let profile =
                v1::RuntimeProfile::decode(row.map_err(CleanStoreError::Database)?.as_slice())
                    .map_err(|_| CleanStoreError::CorruptState)?;
            if self
                .effective_runtime_certificate(&profile)?
                .is_some_and(|certificate| certificate.id == id)
                || profile.certificate_id.as_deref() == Some(id)
            {
                count += 1;
            }
        }
        let mut query = self
            .connection
            .prepare("SELECT value FROM reconciliation_cursors WHERE name LIKE 'managed:%'")
            .map_err(CleanStoreError::Database)?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(CleanStoreError::Database)?;
        for row in rows {
            let cursor: crate::profiles::ayni::preparation_state::RuntimeCursor =
                serde_json::from_str(&row.map_err(CleanStoreError::Database)?)
                    .map_err(|_| CleanStoreError::CorruptState)?;
            if ayni_orchestrator_application::runtime_preparation::RetainedCredentials::credential_references(&cursor).certificates.contains(id) {
                count += 1;
            }
        }
        Ok(count)
    }

    pub(crate) fn import_legacy_runtime_certificate(
        &mut self,
        profile_id: &str,
        certificate_id: &str,
    ) -> Result<(), CleanStoreError> {
        let mut profile = self.runtime_profile(profile_id)?;
        if profile.kind != v1::RuntimeProfileKind::AyniManaged as i32
            || profile.certificate_id.is_some()
        {
            return Ok(());
        }
        profile.certificate_id = Some(certificate_id.into());
        let effective = self.effective_runtime_certificate(&profile)?;
        self.update_runtime_certificate(profile, effective, None)?;
        Ok(())
    }

    /// Called after certificate lifecycle changes and before runtime observations.
    pub(crate) fn invalidate_certificate_runtime_profiles(
        &mut self,
    ) -> Result<(), CleanStoreError> {
        let ids = {
            let mut query = self
                .connection
                .prepare("SELECT id FROM runtime_profiles WHERE kind=?1")
                .map_err(CleanStoreError::Database)?;
            query
                .query_map([v1::RuntimeProfileKind::AyniManaged as i32], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(CleanStoreError::Database)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(CleanStoreError::Database)?
        };
        for id in ids {
            let profile = self.runtime_profile(&id)?;
            let effective = self.effective_runtime_certificate(&profile)?;
            if profile
                .effective_certificate
                .as_ref()
                .map(certificate_identity)
                != effective.as_ref().map(certificate_identity)
            {
                self.update_runtime_certificate(profile, effective, None)?;
            }
        }
        Ok(())
    }

    pub(crate) fn set_runtime_certificate(
        &mut self,
        key: &str,
        request: &v1::SetRuntimeCertificateRequest,
    ) -> Result<v1::RuntimeProfile, CleanStoreError> {
        let digest = request_digest("SetRuntimeCertificate", request);
        if let Some(existing) = action_row(&self.connection, "SetRuntimeCertificate", key)? {
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
        if !request.certificate_id.is_empty() {
            let certificate = self.get_certificate(&request.certificate_id)?;
            if !certificate.enabled || certificate.status != "ready" {
                return Err(CleanStoreError::InvalidTransition);
            }
        }
        let old = profile.certificate_id.clone().unwrap_or_default();
        profile.certificate_id = Some(request.certificate_id.clone());
        let effective = self.effective_runtime_certificate(&profile)?;
        let changed = old != request.certificate_id
            || profile
                .effective_certificate
                .as_ref()
                .map(certificate_identity)
                != effective.as_ref().map(certificate_identity);
        self.update_runtime_certificate(
            profile,
            effective,
            Some((key, digest, request.encode_to_vec(), changed)),
        )
    }

    fn update_runtime_certificate(
        &mut self,
        mut profile: v1::RuntimeProfile,
        effective: Option<v1::Certificate>,
        action: Option<(&str, Vec<u8>, Vec<u8>, bool)>,
    ) -> Result<v1::RuntimeProfile, CleanStoreError> {
        let now = unix_seconds()?;
        let changed = action.as_ref().is_none_or(|(_, _, _, changed)| *changed);
        let revision = if changed && profile.active_revision_id.is_some() {
            Some(self.registry_replacement_revision(&profile, None, now)?)
        } else {
            None
        };
        profile.effective_certificate = effective;
        if let Some(revision) = &revision {
            profile.active_revision_id = Some(revision.id.clone());
        }
        if changed {
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
        if let Some(revision) = revision {
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
            tx.execute("INSERT INTO runtime_profile_revisions(id,runtime_profile_id,repository_snapshot_id,revision_number,payload,resource_version,event_cursor) VALUES(?1,?2,?3,?4,?5,1,?6)",
                params![revision.id,profile.id,revision.repository_snapshot_id,i64::try_from(revision.revision_number).map_err(|_|CleanStoreError::CorruptState)?,revision.encode_to_vec(),i64::try_from(cursor).map_err(|_|CleanStoreError::CorruptState)?]).map_err(map_constraint)?;
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
        tx.execute("UPDATE runtime_profiles SET state=?1,active_revision_id=?2,active_runtime_artifact_id=?3,payload=?4,resource_version=?5,event_cursor=?6 WHERE id=?7",
            params![profile.state,profile.active_revision_id,profile.active_runtime_artifact_id,profile.encode_to_vec(),i64::try_from(profile.resource_version).map_err(|_|CleanStoreError::CorruptState)?,i64::try_from(cursor).map_err(|_|CleanStoreError::CorruptState)?,profile.id]).map_err(CleanStoreError::Database)?;
        tx.execute("UPDATE reconciliation_cursors SET value=json_set(value,'$.next_at',0),resource_version=resource_version+1,updated_at=?1 WHERE name=?2",params![now,format!("managed:{}",profile.id)]).map_err(CleanStoreError::Database)?;
        if let Some((key, digest, payload, _)) = action {
            tx.execute("INSERT INTO durable_actions(operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at) VALUES('SetRuntimeCertificate',?1,?2,?3,'completed','runtime_profile',?4,?5,?5)",params![key,digest,payload,profile.id,now]).map_err(map_constraint)?;
        }
        tx.commit().map_err(CleanStoreError::Database)?;
        Ok(profile)
    }
}

const fn certificate_identity(
    certificate: &v1::Certificate,
) -> (&str, &str, &str, &str, bool, &str) {
    (
        certificate.id.as_str(),
        certificate.revision.as_str(),
        certificate.key_id.as_str(),
        certificate.public_key.as_str(),
        certificate.enabled,
        certificate.status.as_str(),
    )
}
