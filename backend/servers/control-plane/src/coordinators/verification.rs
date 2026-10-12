//! Restart repair for authenticated observations under their frozen authority.
use super::{AppState, ReconciliationError};
use std::path::PathBuf;
use tracing::Instrument;

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
            let execution = payload
                .binding
                .as_ref()
                .map(|b| b.execution_id.clone())
                .unwrap_or_default();
            let context = self
                .database
                .execute(move |store| Ok(store.verification_trace_context(&execution)))
                .await
                .ok()
                .flatten();
            let path = root.clone();
            let observation = payload.clone();
            let evidence_span = ayni_orchestrator_observability::verification_span(
                "evidence",
                observation
                    .binding
                    .as_ref()
                    .map_or("", |b| b.execution_id.as_str()),
                context.as_ref(),
            );
            let evidence = tokio::task::spawn_blocking(move || {
                evidence_span
                    .in_scope(|| crate::profiles::ayni::verification::read(&path, &observation))
            })
            .await
            .map_err(|_| ReconciliationError::Store)?;
            let span = ayni_orchestrator_observability::verification_span(
                "decision",
                payload
                    .binding
                    .as_ref()
                    .map_or("", |b| b.execution_id.as_str()),
                context.as_ref(),
            );
            self.database
                .execute(move |store| {
                    store
                        .decide_verification_with_interval(&identity, &payload, &evidence, interval)
                })
                .instrument(span)
                .await
                .map_err(|_| ReconciliationError::Store)?;
        }
        Ok(())
    }
}
