//! Restart repair for authenticated observations under their frozen authority.
use super::{AppState, ReconciliationError};
use std::path::PathBuf;

impl AppState {
    /// Processes a bounded batch of durable observations without holding the DB
    /// transaction across evidence reads. Unavailable evidence is deferred fairly.
    /// # Errors
    /// Reports database/worker failure while preserving pending observations.
    pub async fn reconcile_verification_observations(&self) -> Result<(), ReconciliationError> {
        self.verification_observations_at(PathBuf::from("/runtime-outputs"))
            .await
    }

    pub(crate) async fn verification_observations_at(
        &self,
        root: PathBuf,
    ) -> Result<(), ReconciliationError> {
        let interval =
            crate::process::runtime_configuration::RuntimeConfiguration::from_environment()
                .map_err(|_| ReconciliationError::Store)?
                .map_or(60, |c| c.interval_seconds);
        let pending = self
            .database
            .execute(|store| store.pending_verifications())
            .await
            .map_err(|_| ReconciliationError::Store)?;
        for (identity, payload) in pending {
            let path = root.clone();
            let observation = payload.clone();
            let evidence = tokio::task::spawn_blocking(move || {
                crate::profiles::ayni::verification::read(&path, &observation)
            })
            .await
            .map_err(|_| ReconciliationError::Store)?;
            self.database
                .execute(move |store| {
                    store
                        .decide_verification_with_interval(&identity, &payload, &evidence, interval)
                })
                .await
                .map_err(|_| ReconciliationError::Store)?;
        }
        Ok(())
    }
}
