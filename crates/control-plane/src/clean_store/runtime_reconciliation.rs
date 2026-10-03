impl CleanStore {
    pub(crate) fn runtime_cursor(&self, profile_id: &str) -> Result<crate::runtime_reconciler::RuntimeCursor, CleanStoreError> {
        let value: Option<String> = self.connection.query_row("SELECT value FROM reconciliation_cursors WHERE name=?1", [format!("managed:{profile_id}")], |row| row.get(0))
            .optional().map_err(CleanStoreError::Database)?;
        value.map(|value| serde_json::from_str(&value).map_err(|_| CleanStoreError::CorruptState)).transpose().map(Option::unwrap_or_default)
    }

    pub(crate) fn save_runtime_cursor(&mut self, profile_id: &str, cursor: &crate::runtime_reconciler::RuntimeCursor) -> Result<(), CleanStoreError> {
        let value = serde_json::to_string(cursor).map_err(|_| CleanStoreError::CorruptState)?;
        if value.len() > 65_536 { return Err(CleanStoreError::BoundExceeded); }
        let mut profile = self.runtime_profile(profile_id)?;
        let metadata = cursor.metadata();
        if metadata.len() > 16 || metadata.iter().any(|m| m.value.len() > 512) { return Err(CleanStoreError::BoundExceeded); }
        let transaction = self.connection.transaction().map_err(CleanStoreError::Database)?;
        transaction.execute("INSERT INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES (?1,?2,1,?3) ON CONFLICT(name) DO UPDATE SET value=excluded.value,resource_version=resource_version+1,updated_at=excluded.updated_at", params![format!("managed:{profile_id}"), value, unix_seconds()?]).map_err(CleanStoreError::Database)?;
        if profile.reconciliation_metadata != metadata {
            profile.reconciliation_metadata = metadata;
            profile.resource_version += 1;
            profile.updated_at = Some(timestamp(unix_seconds()?));
            let event = append_event(&transaction, PROFILE_SERVICE, "runtime_profile", profile_id, profile.resource_version, Some(&profile.repository_connection_id), Some(profile_id), &profile)?;
            transaction.execute("UPDATE runtime_profiles SET payload=?1,resource_version=?2,event_cursor=?3 WHERE id=?4", params![profile.encode_to_vec(), i64::try_from(profile.resource_version).map_err(|_| CleanStoreError::CorruptState)?, i64::try_from(event).map_err(|_| CleanStoreError::CorruptState)?, profile_id]).map_err(CleanStoreError::Database)?;
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn runtime_failure(&mut self, profile_id: &str, reason: &str) -> Result<(), CleanStoreError> {
        let profile = self.runtime_profile(profile_id)?;
        if profile.state == v1::RuntimeProfileState::Degraded as i32 { return Ok(()); }
        self.transition_runtime_profile(profile_id, profile.resource_version, v1::RuntimeProfileState::Degraded, Some(v1::ResourceFailure {
            reason:v1::ErrorReason::RuntimeArtifactNotVerified as i32, message:reason.into(), remediation:Some("Repair runtime source or access and reconcile the profile.".into()), observed_at:Some(timestamp(unix_seconds()?))
        }))?;
        Ok(())
    }

    pub(crate) fn prepare_runtime_revision(&mut self, profile_id: &str, revision: &str, fingerprint: &str, repository: &str, source_digest: &str) -> Result<String, CleanStoreError> {
        let profile = self.runtime_profile(profile_id)?;
        let previous = self.runtime_profile_revision(profile.active_revision_id.as_deref().ok_or(CleanStoreError::CorruptState)?)?;
        let Some(v1::runtime_profile_revision::Configuration::AyniManaged(mut configuration)) = previous.configuration else { return Err(CleanStoreError::InvalidTransition); };
        if previous.repository_revision == revision && configuration.lock_fingerprint == fingerprint && configuration.oci_repository == repository && configuration.runtime_source_digest.as_deref() == Some(source_digest) {
            return Ok(previous.id);
        }
        let snapshot = self.observe_repository_snapshot(&profile.repository_connection_id, revision)?;
        fingerprint.clone_into(&mut configuration.lock_fingerprint);
        repository.clone_into(&mut configuration.oci_repository);
        configuration.ayni_version = "0.14.5".into();
        configuration.runtime_source_digest = Some(source_digest.to_owned());
        let revision = self.record_selected_runtime_revision(profile_id, &snapshot.id, v1::runtime_profile_revision::Configuration::AyniManaged(configuration), false)?;
        Ok(revision.id)
    }

    pub(crate) fn activate_runtime(&mut self, profile_id: &str, revision_id: &str, image: &str, digest: &str, report: &ayni_orchestrator_runtime::runtime_execution::VerificationReport) -> Result<(), CleanStoreError> {
        let profile = self.runtime_profile(profile_id)?;
        if profile.active_revision_id.as_deref() != Some(revision_id) { return Err(CleanStoreError::StaleVersion); }
        let revision = self.runtime_profile_revision(revision_id)?;
        if revision.repository_revision != report.revision { return Err(CleanStoreError::StaleVersion); }
        let Some(v1::runtime_profile_revision::Configuration::AyniManaged(configuration)) = revision.configuration else { return Err(CleanStoreError::InvalidTransition); };
        if report.runtime.lock_fingerprint != format!("sha256:{}", configuration.lock_fingerprint) { return Err(CleanStoreError::StaleVersion); }
        if let Some(id) = profile.active_runtime_artifact_id {
            let artifact = self.runtime_artifact(&id)?;
            if artifact.resolved_digest != digest { return Err(CleanStoreError::IdempotencyConflict); }
            if profile.state != v1::RuntimeProfileState::Ready as i32 {
                self.transition_runtime_profile(profile_id, profile.resource_version, v1::RuntimeProfileState::Ready, None)?;
            }
            return Ok(());
        }
        let metadata = [
            ("certificate-key-id", report.runtime.certificate_key_id.clone()),
            ("runtime-metadata-digest", report.runtime.metadata_digest.clone()),
            ("protected-content-root", report.runtime.protected_content_root.clone()),
            ("source-lock-digest", report.lock_digest.clone()),
            ("lock-fingerprint", report.runtime.lock_fingerprint.clone()),
        ].into_iter().map(|(key,value)| v1::VerifiedMetadata { key:key.into(),value }).collect();
        self.record_runtime_artifact(profile_id, v1::RuntimeArtifact {
            runtime_profile_revision_id:revision_id.into(), oci_repository:configuration.oci_repository,
            discovery_tag:image.rsplit_once(':').filter(|_| !image.contains('@')).map(|(_,tag)|tag.to_owned()),
            resolved_digest:digest.into(),platform:Some(v1::OciPlatform { operating_system:"linux".into(), architecture:report.runtime.platform.strip_prefix("linux/").ok_or(CleanStoreError::CorruptState)?.into(),variant:None }),
            origin:Some(v1::RuntimeArtifactOrigin { origin:Some(v1::runtime_artifact_origin::Origin::External(v1::ExternalArtifactOrigin { supplied_reference:image.into(),observed_at:Some(timestamp(unix_seconds()?)) })) }),
            status:v1::RuntimeArtifactStatus::Verified as i32,verified_metadata:metadata,..Default::default()
        })?;
        Ok(())
    }
}
