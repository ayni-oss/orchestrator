use super::{
    CleanStore, CleanStoreError, Message, OptionalExtension, PROFILE_SERVICE, append_event, params,
    timestamp, unix_seconds, v1,
};

impl CleanStore {
    pub(crate) fn runtime_cursor(
        &self,
        profile_id: &str,
    ) -> Result<crate::application::reconciliation::RuntimeCursor, CleanStoreError> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM reconciliation_cursors WHERE name=?1",
                [format!("managed:{profile_id}")],
                |row| row.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        value
            .map(|value| serde_json::from_str(&value).map_err(|_| CleanStoreError::CorruptState))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub(crate) fn save_runtime_cursor(
        &mut self,
        profile_id: &str,
        cursor: &crate::application::reconciliation::RuntimeCursor,
    ) -> Result<(), CleanStoreError> {
        let value = serde_json::to_string(cursor).map_err(|_| CleanStoreError::CorruptState)?;
        if value.len() > 65_536 {
            return Err(CleanStoreError::BoundExceeded);
        }
        let mut profile = self.runtime_profile(profile_id)?;
        let metadata = cursor.metadata();
        if metadata.len() > 16 || metadata.iter().any(|m| m.value.len() > 512) {
            return Err(CleanStoreError::BoundExceeded);
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction.execute("INSERT INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES (?1,?2,1,?3) ON CONFLICT(name) DO UPDATE SET value=excluded.value,resource_version=resource_version+1,updated_at=excluded.updated_at", params![format!("managed:{profile_id}"), value, unix_seconds()?]).map_err(CleanStoreError::Database)?;
        if profile.reconciliation_metadata != metadata {
            profile.reconciliation_metadata = metadata;
            profile.resource_version += 1;
            profile.updated_at = Some(timestamp(unix_seconds()?));
            let event = append_event(
                &transaction,
                PROFILE_SERVICE,
                "runtime_profile",
                profile_id,
                profile.resource_version,
                Some(&profile.repository_connection_id),
                Some(profile_id),
                &profile,
            )?;
            transaction.execute("UPDATE runtime_profiles SET payload=?1,resource_version=?2,event_cursor=?3 WHERE id=?4", params![profile.encode_to_vec(), i64::try_from(profile.resource_version).map_err(|_| CleanStoreError::CorruptState)?, i64::try_from(event).map_err(|_| CleanStoreError::CorruptState)?, profile_id]).map_err(CleanStoreError::Database)?;
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn runtime_failure(
        &mut self,
        profile_id: &str,
        reason: &str,
    ) -> Result<(), CleanStoreError> {
        let profile = self.runtime_profile(profile_id)?;
        if profile.state == v1::RuntimeProfileState::Degraded as i32 {
            return Ok(());
        }
        self.transition_runtime_profile(
            profile_id,
            profile.resource_version,
            v1::RuntimeProfileState::Degraded,
            Some(v1::ResourceFailure {
                reason: v1::ErrorReason::RuntimeArtifactNotVerified as i32,
                message: reason.into(),
                remediation: Some(
                    "Repair runtime source or access and reconcile the profile.".into(),
                ),
                observed_at: Some(timestamp(unix_seconds()?)),
            }),
        )?;
        Ok(())
    }
}
