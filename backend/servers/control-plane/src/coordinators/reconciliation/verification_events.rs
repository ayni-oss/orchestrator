//! Event-owned generations never consult the retained Kubernetes result reader.
use super::{
    ExecutionBackend, PendingVerification, ProfileObservation, RuntimeCursor, RuntimeReconciler,
};
use crate::profiles::ayni::verification::{Evidence, failure_reason, read};
use ayni_orchestrator_application::execution::ExecutionState;
use ayni_orchestrator_events::v1::verification_result_available::Outcome;

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn observe_event_verification(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
        pending: &PendingVerification,
    ) -> Result<(), &'static str> {
        let execution = pending.name.clone();
        let receipt = self
            .with_database(move |store| {
                store
                    .verification_receipt(&execution)
                    .map_err(|_| "runtime_verification_receipt_rejected")
            })
            .await?;
        if let Some((identity, payload)) = receipt {
            let evidence = tokio::task::spawn_blocking(move || {
                let evidence = read(std::path::Path::new("/runtime-outputs"), &payload);
                (payload, evidence)
            })
            .await
            .map_err(|_| "runtime_report_unavailable")?;
            return self
                .apply_verification_receipt(observation, cursor, identity, evidence.0, evidence.1)
                .await;
        }
        let profile = observation.id.to_owned();
        let current = self
            .with_database(move |store| {
                store
                    .runtime_cursor(&profile)
                    .map_err(|_| "runtime_cursor_invalid")
            })
            .await?;
        if serde_json::to_vec(&current).map_err(|_| "runtime_cursor_invalid")?
            != serde_json::to_vec(&cursor).map_err(|_| "runtime_cursor_invalid")?
        {
            *cursor = current;
            return Ok(());
        }
        if observation.time.saturating_sub(pending.started_at) > ayni_orchestrator_application::runtime_reconciliation::VERIFICATION_OBSERVATION_TIMEOUT_SECONDS {
            return self.fail_execution(observation.id, cursor, &pending.name, "runtime_verification_timeout").await;
        }
        match self.kubernetes.coding_state(&pending.name).await? {
            ExecutionState::Missing => self.start_verification(observation, cursor, pending).await,
            ExecutionState::Running => Ok(()),
            ExecutionState::Succeeded | ExecutionState::Failed => Err("runtime_report_missing"),
        }
    }

    pub(super) async fn apply_verification_receipt(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
        identity: ayni_orchestrator_events::EventIdentity,
        payload: ayni_orchestrator_events::v1::VerificationResultAvailable,
        evidence: Evidence,
    ) -> Result<(), &'static str> {
        match evidence {
            Evidence::Unavailable => Err("runtime_report_unavailable"),
            Evidence::Invalid(reason) => {
                self.with_database(move |store| {
                    store
                        .reject_verification_receipt(&identity, reason)
                        .map_err(|_| "runtime_store_failed")
                })
                .await?;
                Err("runtime_result_invalid")
            }
            Evidence::Report(report) => {
                let digest = payload
                    .binding
                    .as_ref()
                    .ok_or("runtime_result_invalid")?
                    .resolved_image_digest
                    .clone();
                self.accept_report(observation, cursor, digest, Some(identity), &report)
                    .await
            }
            Evidence::Failed => {
                let Some(Outcome::ExecutionFailed(failure)) = &payload.outcome else {
                    return Err("runtime_result_invalid");
                };
                let reason = failure_reason(failure.code)?;
                let profile = observation.id.to_owned();
                let interval = self.configuration.interval_seconds;
                let updated = self
                    .with_database(move |store| {
                        store
                            .decide_verification_with_interval(
                                &identity,
                                &payload,
                                &Evidence::Failed,
                                interval,
                            )
                            .map_err(|_| "runtime_failure_changed")?;
                        store
                            .runtime_cursor(&profile)
                            .map_err(|_| "runtime_cursor_invalid")
                    })
                    .await?;
                *cursor = updated;
                Err(reason)
            }
        }
    }
}
