//! Bounded cleanup of replay data; unresolved workflow authority is retained.
use ayni_orchestrator_workflow_postgres::{WorkflowStore, metrics};
use std::time::{Duration, Instant};
use tokio::{
    sync::watch,
    time::{MissedTickBehavior, interval},
};

pub async fn run(store: WorkflowStore, mut shutdown: watch::Receiver<bool>) {
    let mut tick = interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let began = Instant::now();
                if store.cleanup_observation().await.is_err() {
                    metrics::increment("workflow_observation_cleanup_errors_total");
                }
                if store.cleanup_history_snapshots().await.is_err() {
                    metrics::increment("workflow_history_cleanup_errors_total");
                }
                if store.cleanup_transport_quarantine().await.is_err() {
                    metrics::increment("workflow_quarantine_cleanup_errors_total");
                }
                if store.cleanup_retained_task_sources().await.is_err() {
                    metrics::increment("workflow_task_source_cleanup_errors_total");
                }
                if store.expire_idle_sessions().await.is_err() {
                    metrics::increment("workflow_session_expiry_errors_total");
                }
                if store.admit_outcome_assessments(16).await.is_err() {
                    metrics::increment("workflow_assessment_admission_errors_total");
                }
                metrics::observe("workflow_cleanup_seconds", began.elapsed());
            },
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return; }
            }
        }
    }
}
