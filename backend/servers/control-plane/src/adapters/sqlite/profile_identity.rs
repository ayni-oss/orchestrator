use super::*;

impl CleanStore {
    /// Reject a second managed profile for the same repository and active lock.
    pub(super) fn ensure_managed_lock_available(
        &self,
        repository_id: &str,
        lock_fingerprint: &str,
    ) -> Result<(), CleanStoreError> {
        let existing_revisions = {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT profiles.id,revisions.payload
                     FROM runtime_profiles profiles
                     JOIN runtime_profile_revisions revisions
                       ON revisions.id=profiles.active_revision_id
                     WHERE profiles.repository_connection_id=?1 AND profiles.kind=?2
                     ORDER BY profiles.id",
                )
                .map_err(CleanStoreError::Database)?;
            let rows = statement
                .query_map(
                    params![repository_id, v1::RuntimeProfileKind::AyniManaged as i32],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )
                .map_err(CleanStoreError::Database)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(CleanStoreError::Database)?
        };
        for (profile_id, payload) in existing_revisions {
            let revision = v1::RuntimeProfileRevision::decode(payload.as_slice())
                .map_err(|_| CleanStoreError::CorruptState)?;
            let existing_fingerprint = match revision.configuration {
                Some(v1::runtime_profile_revision::Configuration::AyniManaged(value)) => {
                    value.lock_fingerprint
                }
                _ => continue,
            };
            if existing_fingerprint == lock_fingerprint {
                return Err(CleanStoreError::ManagedLockProfileAlreadyExists { profile_id });
            }
        }
        Ok(())
    }
}
