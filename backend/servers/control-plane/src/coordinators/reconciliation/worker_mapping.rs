use ayni_orchestrator_api::v1;
use ayni_orchestrator_runtime::coding as worker;

pub(super) fn quality(value: worker::QualityResult) -> v1::QualityResult {
    v1::QualityResult {
        outcome: match value.outcome {
            worker::QualityOutcome::Passed => v1::QualityOutcome::Passed,
            worker::QualityOutcome::Failed => v1::QualityOutcome::Failed,
            worker::QualityOutcome::Unavailable => v1::QualityOutcome::Unavailable,
        } as i32,
        evidence: value.evidence.map(|e| v1::QualityEvidence {
            artifact_path: e.artifact_path,
            signals_digest: e.signals_digest,
            source_fingerprint: e.source_fingerprint,
            commit: e.commit,
        }),
        completed_targets: value.completed_targets,
        skipped_targets: value.skipped_targets,
        warning_offenders: value.warning_offenders,
        failing_offenders: value.failing_offenders,
        rows: value
            .rows
            .into_iter()
            .map(|r| v1::QualityRow {
                kind: r.kind,
                passed: r.passed,
                status: r.status,
                summary: r.summary,
                target: r.target,
            })
            .collect(),
        findings: value.findings,
    }
}
pub(super) const fn publication(value: &worker::PublicationOutcome) -> i32 {
    (match value {
        worker::PublicationOutcome::Pending => v1::PublicationOutcome::Pending,
        worker::PublicationOutcome::Publishing => v1::PublicationOutcome::Publishing,
        worker::PublicationOutcome::Published => v1::PublicationOutcome::Published,
        worker::PublicationOutcome::Failed => v1::PublicationOutcome::Failed,
        worker::PublicationOutcome::Conflict => v1::PublicationOutcome::Conflict,
        worker::PublicationOutcome::NotRequired => v1::PublicationOutcome::NotRequired,
    }) as i32
}
pub(super) fn progress(value: worker::WorkerProgress) -> v1::WorkerProgress {
    v1::WorkerProgress {
        phase: match value.phase {
            worker::WorkerPhase::Admission => v1::WorkerPhase::Admission,
            worker::WorkerPhase::Checkout => v1::WorkerPhase::Checkout,
            worker::WorkerPhase::Coding => v1::WorkerPhase::Coding,
            worker::WorkerPhase::Checking => v1::WorkerPhase::Checking,
            worker::WorkerPhase::Repairing => v1::WorkerPhase::Repairing,
            worker::WorkerPhase::Finalizing => v1::WorkerPhase::Finalizing,
            worker::WorkerPhase::Publishing => v1::WorkerPhase::Publishing,
            worker::WorkerPhase::Completed => v1::WorkerPhase::Completed,
        } as i32,
        coding_turns_consumed: u32::from(value.coding_turns_consumed),
        quality_round: u32::from(value.quality_round),
        latest_full_check: value.latest_full_check.map(quality),
        publication: publication(&value.publication),
    }
}
pub(super) fn result(
    value: worker::WorkerResult,
    usage: v1::AttemptModelUsage,
) -> v1::WorkerResult {
    v1::WorkerResult {
        execution: match value.execution {
            worker::ExecutionOutcome::Succeeded => v1::ExecutionOutcome::Succeeded,
            worker::ExecutionOutcome::Failed => v1::ExecutionOutcome::Failed,
            worker::ExecutionOutcome::Cancelled => v1::ExecutionOutcome::Cancelled,
            worker::ExecutionOutcome::DeadlineExceeded => v1::ExecutionOutcome::DeadlineExceeded,
            worker::ExecutionOutcome::AdmissionFailed => v1::ExecutionOutcome::AdmissionFailed,
            worker::ExecutionOutcome::UnsafeChanges => v1::ExecutionOutcome::UnsafeChanges,
            worker::ExecutionOutcome::NoChanges => v1::ExecutionOutcome::NoChanges,
            worker::ExecutionOutcome::InputRequired => v1::ExecutionOutcome::InputRequired,
        } as i32,
        commit: value.commit,
        changed_paths: value.changed_paths,
        quality: Some(quality(value.quality)),
        model_usage: Some(usage),
        requested_input: value.requested_input,
        publication: publication(&value.publication),
        pull_request: value.pull_request.map(|p| v1::PullRequestReceipt {
            repository: p.repository,
            number: p.number,
            url: p.url,
            branch: p.branch,
            base_branch: p.base_branch,
            head_commit: p.head_commit,
            draft: p.draft,
        }),
        coding_turns_consumed: u32::from(value.coding_turns_consumed),
        quality_round: u32::from(value.quality_round),
    }
}

pub(super) fn activity(value: ayni_orchestrator_runtime::activity::Activity) -> v1::AgentActivity {
    v1::AgentActivity {
        sequence: value.sequence,
        observed_at: Some(prost_types::Timestamp {
            seconds: value.timestamp_ms / 1000,
            nanos: i32::try_from((value.timestamp_ms % 1000) * 1_000_000).unwrap_or(0),
        }),
        message: value.failure.map_or_else(
            || value.stage.message().to_owned(),
            |failure| format!("{}: {}", value.stage.message(), failure.message()),
        ),
        failed: value.failed,
        details: value.details,
        truncated: value.truncated,
    }
}
