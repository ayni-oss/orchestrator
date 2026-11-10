//! Kubernetes diagnostics that outlive the observed workload's pod.

use crate::workflow::identifier;
pub use ayni_orchestrator_events::execution_diagnostics::Kind;
use ayni_orchestrator_events::execution_diagnostics::{ContainerDiagnostic, Diagnostic};
use k8s_openapi::api::core::v1::{ContainerState, Pod};

/// Record bounded lifecycle metadata in the supervisor log. Kubernetes messages,
/// termination text and container output can contain credentials or workload data.
pub fn record(
    kind: Kind,
    id: &str,
    generation: u64,
    step: &str,
    event: &'static str,
    pod: Option<&Pod>,
) -> Diagnostic {
    let execution_id = identifier(id);
    let step = identifier(step);
    let status = pod.and_then(|pod| pod.status.as_ref());
    let pod_uid = identifier(
        pod.and_then(|p| p.metadata.uid.as_deref())
            .unwrap_or_default(),
    );
    let phase = match status.and_then(|s| s.phase.as_deref()) {
        Some("Pending") => "pending",
        Some("Running") => "running",
        Some("Succeeded") => "succeeded",
        Some("Failed") => "failed",
        _ => "unknown",
    };
    let reason = known_reason(status.and_then(|s| s.reason.as_deref()));
    let mut diagnostic = Diagnostic::new(kind, execution_id, generation, step, event);
    diagnostic.pod_uid = pod_uid.into();
    diagnostic.outcome = phase.into();
    diagnostic.reason = reason.into();
    let kind = kind.name();
    tracing::info!(
        kind,
        execution_id,
        generation,
        stage = step,
        event,
        pod_uid,
        phase,
        reason,
        "Workload lifecycle"
    );

    let Some(status) = status else {
        return diagnostic;
    };
    'containers: for container in status
        .init_container_statuses
        .iter()
        .flatten()
        .chain(status.container_statuses.iter().flatten())
        .take(16)
    {
        for (observation, state) in [
            ("current", &container.state),
            ("previous", &container.last_state),
        ] {
            let Some(state) = state else { continue };
            let Some((state, reason, exit_code, signal)) = container_outcome(state) else {
                continue;
            };
            if diagnostic.containers.len() == 16 {
                break 'containers;
            }
            diagnostic.containers.push(ContainerDiagnostic {
                name: identifier(&container.name).into(),
                previous: observation == "previous",
                state: state.into(),
                reason: reason.into(),
                exit_code,
                signal,
                restart_count: container.restart_count,
            });
            tracing::info!(
                kind,
                execution_id,
                generation,
                stage = step,
                event,
                pod_uid,
                container = identifier(&container.name),
                observation,
                state,
                reason,
                exit_code,
                signal,
                restart_count = container.restart_count,
                "Workload container state"
            );
        }
    }
    diagnostic
}

fn container_outcome(
    state: &ContainerState,
) -> Option<(&'static str, &'static str, Option<i32>, Option<i32>)> {
    if let Some(terminated) = &state.terminated {
        return Some((
            "terminated",
            known_reason(terminated.reason.as_deref()),
            Some(terminated.exit_code),
            terminated.signal,
        ));
    }
    state.waiting.as_ref().map(|waiting| {
        (
            "waiting",
            known_reason(waiting.reason.as_deref()),
            None,
            None,
        )
    })
}

fn known_reason(value: Option<&str>) -> &'static str {
    match value {
        Some("Completed") => "completed",
        Some("Error") => "error",
        Some("OOMKilled") => "out_of_memory",
        Some("DeadlineExceeded") => "deadline_exceeded",
        Some("Evicted") => "evicted",
        Some("NodeLost") => "node_lost",
        Some("ContainerStatusUnknown") => "container_status_unknown",
        Some("ContainerCannotRun" | "StartError" | "RunContainerError") => "container_start_failed",
        Some("CreateContainerConfigError" | "CreateContainerError") => "container_create_failed",
        Some("ErrImagePull" | "ImagePullBackOff" | "InvalidImageName") => "image_pull_failed",
        Some("CrashLoopBackOff") => "crash_loop",
        Some("PodInitializing" | "ContainerCreating") => "initializing",
        Some(_) => "other",
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::ContainerStateTerminated;

    #[test]
    fn container_diagnostics_keep_exit_evidence_and_discard_untrusted_text() {
        let terminated = ContainerStateTerminated {
            exit_code: 137,
            signal: Some(9),
            reason: Some("OOMKilled".into()),
            message: Some("untrusted content".into()),
            ..Default::default()
        };
        let state = ContainerState {
            terminated: Some(terminated.clone()),
            ..Default::default()
        };
        assert_eq!(
            container_outcome(&state),
            Some(("terminated", "out_of_memory", Some(137), Some(9)))
        );
        let state = ContainerState {
            terminated: Some(ContainerStateTerminated {
                reason: Some("untrusted reason".into()),
                ..terminated
            }),
            ..Default::default()
        };
        assert_eq!(
            container_outcome(&state),
            Some(("terminated", "other", Some(137), Some(9)))
        );
    }
}
