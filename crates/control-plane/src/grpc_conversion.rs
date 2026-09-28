fn repository_binding(repository: RepositoryView) -> v1::RepositoryBinding {
    v1::RepositoryBinding {
        enrollment: Some(repository_enrollment(repository.binding)),
        latest_preparation: repository.latest_preparation.map(preparation_message),
    }
}

fn repository_enrollment(repository: RepositoryBinding) -> v1::RepositoryEnrollment {
    v1::RepositoryEnrollment {
        id: repository.id,
        owner: repository.owner,
        repository: repository.repository,
        installation_id: u64::try_from(repository.installation_id).unwrap_or_default(),
        base_branch: repository.base_branch,
        lock_sha256: repository.lock_sha256,
    }
}

fn preparation_message(value: PreparationView) -> v1::Preparation {
    v1::Preparation {
        id: value.id,
        repository_id: value.repository_id,
        revision: value.revision,
        lock_sha256: value.lock_sha256,
        status: match value.status {
            PreparationStatus::Queued => v1::PreparationStatus::Queued,
            PreparationStatus::Preparing => v1::PreparationStatus::Preparing,
            PreparationStatus::Ready => v1::PreparationStatus::Ready,
            PreparationStatus::SetupRequired => v1::PreparationStatus::SetupRequired,
            PreparationStatus::Failed => v1::PreparationStatus::Failed,
        } as i32,
        diagnostic: value.diagnostic,
        duration: value.duration_seconds.map(|seconds| Duration {
            seconds: i64::try_from(seconds).unwrap_or(i64::MAX),
            nanos: 0,
        }),
        reuse_count: value.reuse_count,
    }
}

fn provider_model(value: crate::AvailableModel) -> v1::ProviderModel {
    v1::ProviderModel {
        id: value.id,
        name: value.name,
        owned_by: value.owned_by,
        created_at: value.created_at.map(|seconds| Timestamp {
            seconds: i64::try_from(seconds).unwrap_or(i64::MAX),
            nanos: 0,
        }),
        context_length: value.context_length,
        max_output_tokens: value.max_output_tokens,
    }
}

fn development_task(task: DevelopmentTask) -> v1::DevelopmentTask {
    let failure = task.failure.as_ref().map(|message| v1::TaskFailure {
        reason: task_failure_reason(&task) as i32,
        message: crate::truncate_utf8(message, 512),
    });
    v1::DevelopmentTask {
        id: task.id.to_string(),
        input: Some(v1::DevelopmentTaskInput {
            repository_id: task.input.repository_id,
            model: task.agent_profile.model.clone(),
            start_revision: task.input.start_revision,
            request: task.input.request,
            acceptance_criteria: task.input.acceptance_criteria,
            change_constraints: Some(change_constraints(task.input.change_constraints)),
            deadline: task.input.deadline_seconds.map(|seconds| Duration {
                seconds: i64::try_from(seconds).unwrap_or(i64::MAX),
                nanos: 0,
            }),
        }),
        repository: Some(v1::RepositorySnapshot {
            id: task.repository.id,
            owner: task.repository.owner,
            repository: task.repository.repository,
            base_branch: task.repository.base_branch,
            lock_sha256: task.repository.lock_sha256,
        }),
        model: Some(v1::ModelSnapshot {
            provider: task.agent_profile.provider,
            model: task.agent_profile.model,
        }),
        preparation_id: task.preparation_id,
        retry_of_task_id: task.retry_of.map(|id| id.to_string()),
        created_at: Some(Timestamp {
            seconds: task.created_at,
            nanos: 0,
        }),
        updated_at: Some(Timestamp {
            seconds: task.updated_at,
            nanos: 0,
        }),
        deadline_at: task
            .deadline_at
            .map(|seconds| Timestamp { seconds, nanos: 0 }),
        phase: development_phase(task.phase) as i32,
        requested_input: task.requested_input,
        candidate_commit: task.candidate_commit,
        validation_manifest_digest: task.validation_manifest,
        policy_or_test_edits: task.policy_or_test_edits,
        scope_violations: task.scope_violations,
        draft_pr_url: task.draft_pr_url,
        failure,
        lifecycle_complete: task.lifecycle_complete,
        lifecycle: task.lifecycle.into_iter().map(lifecycle_event).collect(),
        model_usage: Some(task_model_usage(task.model_usage)),
        provided_input: task.provided_input,
        resource_version: task.resource_version,
    }
}

fn development_summary(task: DevelopmentTaskListItem) -> v1::DevelopmentTaskSummary {
    let failure = task.failure.as_ref().map(|message| v1::TaskFailure {
        reason: failure_reason(task.failure_reason_code.as_deref()) as i32,
        message: crate::truncate_utf8(message, 512),
    });
    v1::DevelopmentTaskSummary {
        id: task.id.to_string(),
        repository_id: task.repository_id,
        repository_owner: task.repository_owner,
        repository_name: task.repository_name,
        start_revision: task.start_revision,
        request: task.request,
        model: Some(v1::ModelSnapshot {
            provider: task.model_provider,
            model: task.model,
        }),
        retry_of_task_id: task.retry_of.map(|id| id.to_string()),
        created_at: Some(Timestamp {
            seconds: task.created_at,
            nanos: 0,
        }),
        updated_at: Some(Timestamp {
            seconds: task.updated_at,
            nanos: 0,
        }),
        phase: development_phase(task.phase) as i32,
        candidate_commit: task.candidate_commit,
        draft_pr_url: task.draft_pr_url,
        failure,
        model_usage: Some(model_usage_summary(task.model_usage)),
        resource_version: task.resource_version,
    }
}

fn change_constraints(value: ChangeConstraints) -> v1::ChangeConstraints {
    v1::ChangeConstraints {
        protected_paths: value.protected_paths,
        allow_test_edits: value.allow_test_edits,
    }
}

fn lifecycle_event(value: crate::development::DevelopmentLifecycleEvent) -> v1::LifecycleEvent {
    v1::LifecycleEvent {
        sequence: u64::try_from(value.sequence).unwrap_or_default(),
        event_time: Some(Timestamp {
            seconds: value.event_time,
            nanos: 0,
        }),
        stage: match value.stage {
            DevelopmentLifecycleStage::Queue => v1::LifecycleStage::Queue,
            DevelopmentLifecycleStage::Coding => v1::LifecycleStage::Coding,
            DevelopmentLifecycleStage::Validation => v1::LifecycleStage::Validation,
            DevelopmentLifecycleStage::PrPreparation => v1::LifecycleStage::PrPreparation,
            DevelopmentLifecycleStage::Unknown => v1::LifecycleStage::Unknown,
        } as i32,
        outcome: match value.outcome {
            DevelopmentLifecycleOutcome::Queued => v1::LifecycleOutcome::Queued,
            DevelopmentLifecycleOutcome::Started => v1::LifecycleOutcome::Started,
            DevelopmentLifecycleOutcome::Succeeded => v1::LifecycleOutcome::Succeeded,
            DevelopmentLifecycleOutcome::Blocked => v1::LifecycleOutcome::Blocked,
            DevelopmentLifecycleOutcome::Failed => v1::LifecycleOutcome::Failed,
            DevelopmentLifecycleOutcome::Cancelled => v1::LifecycleOutcome::Cancelled,
            DevelopmentLifecycleOutcome::Unsupported => v1::LifecycleOutcome::Unsupported,
        } as i32,
        reason_code: value.reason_code,
    }
}

fn task_model_usage(value: TaskModelUsage) -> v1::TaskModelUsage {
    v1::TaskModelUsage {
        collection_status: match value.collection_status {
            ModelUsageCollectionStatus::Unavailable => v1::ModelUsageCollectionStatus::Unavailable,
            ModelUsageCollectionStatus::Collecting => v1::ModelUsageCollectionStatus::Collecting,
            ModelUsageCollectionStatus::Complete => v1::ModelUsageCollectionStatus::Complete,
        } as i32,
        interactions: value
            .interactions
            .into_iter()
            .map(model_interaction)
            .collect(),
        by_stage: value.by_stage.into_iter().map(stage_model_usage).collect(),
        total: Some(model_usage_summary(value.total)),
    }
}

fn model_interaction(value: ModelUsageInteraction) -> v1::ModelUsageInteraction {
    v1::ModelUsageInteraction {
        call_id: value.call_id,
        stage: model_usage_stage(value.stage) as i32,
        provider: value.provider,
        model: value.model,
        resolved_model: value.resolved_model,
        started_at: Some(timestamp_millis(value.started_at_ms)),
        completed_at: Some(timestamp_millis(value.completed_at_ms)),
        latency: Some(duration_millis(value.latency_ms)),
        outcome: match value.outcome {
            ModelInteractionOutcome::InProgress => v1::ModelInteractionOutcome::Unspecified,
            ModelInteractionOutcome::Succeeded => v1::ModelInteractionOutcome::Succeeded,
            ModelInteractionOutcome::Failed => v1::ModelInteractionOutcome::Failed,
            ModelInteractionOutcome::Cancelled => v1::ModelInteractionOutcome::Cancelled,
        } as i32,
        tokens: Some(provider_token_usage(value.tokens)),
        estimated_cost: value.estimated_cost.map(estimated_model_cost),
    }
}

const fn provider_token_usage(value: ProviderTokenUsage) -> v1::ProviderTokenUsage {
    v1::ProviderTokenUsage {
        input_tokens: value.input_tokens,
        output_tokens: value.output_tokens,
        cached_input_tokens: value.cached_input_tokens,
        reasoning_tokens: value.reasoning_tokens,
        total_tokens: value.total_tokens,
    }
}

fn estimated_model_cost(value: EstimatedModelCost) -> v1::EstimatedModelCost {
    v1::EstimatedModelCost {
        label: value.label.to_owned(),
        amount_micros: value.amount_micros,
        currency: value.currency,
        pricing_version: value.pricing_version,
        priced_input_tokens: value.priced_input_tokens,
        priced_cached_input_tokens: value.priced_cached_input_tokens,
        priced_output_tokens: value.priced_output_tokens,
        input_rate_micros_per_million: value.input_rate_micros_per_million,
        cached_input_rate_micros_per_million: value.cached_input_rate_micros_per_million,
        output_rate_micros_per_million: value.output_rate_micros_per_million,
    }
}

fn stage_model_usage(value: StageModelUsage) -> v1::StageModelUsage {
    v1::StageModelUsage {
        stage: model_usage_stage(value.stage) as i32,
        summary: Some(model_usage_summary(value.summary)),
    }
}

const fn model_usage_stage(value: ModelUsageStage) -> v1::ModelUsageStage {
    match value {
        ModelUsageStage::Coding => v1::ModelUsageStage::Coding,
        ModelUsageStage::PrPreparation => v1::ModelUsageStage::PrPreparation,
    }
}

fn model_usage_summary(value: ModelUsageSummary) -> v1::ModelUsageSummary {
    v1::ModelUsageSummary {
        interaction_count: u32::try_from(value.interaction_count).unwrap_or(u32::MAX),
        unavailable_interaction_count: u32::try_from(value.unavailable_interaction_count)
            .unwrap_or(u32::MAX),
        input_tokens: value.input_tokens,
        output_tokens: value.output_tokens,
        cached_input_tokens: value.cached_input_tokens,
        reasoning_tokens: value.reasoning_tokens,
        total_tokens: value.total_tokens,
        estimated_cost_micros: value.estimated_cost_micros,
        estimated_cost_currency: value.estimated_cost_currency,
        pricing_version: value.pricing_version,
    }
}

fn development_evidence(value: EvidenceView) -> v1::DevelopmentTaskEvidence {
    v1::DevelopmentTaskEvidence {
        manifest_digest: value.manifest_digest,
        outcome: match value.outcome.as_str() {
            "success" => v1::EvidenceOutcome::Success,
            "quality_failure" => v1::EvidenceOutcome::QualityFailure,
            "policy_rejected" => v1::EvidenceOutcome::PolicyRejected,
            "dependency_change_unsupported" => v1::EvidenceOutcome::DependencyChangeUnsupported,
            "scope_rejected" => v1::EvidenceOutcome::ScopeRejected,
            _ => v1::EvidenceOutcome::Unspecified,
        } as i32,
        check_exit_code: value.check_exit_code,
        start_revision: value.start_revision,
        candidate_commit: value.candidate_commit,
        lock_sha256: value.lock_sha256,
        change_constraints: value.change_constraints.map(change_constraints),
        preparation_id: value.preparation_id,
        preparation_reused: value.preparation_reused,
        policy_edited: value.policy_edited,
        tests_edited: value.tests_edited,
        scope_violations: value.scope_violations,
        changed_paths: value.changed_paths,
        artifacts: value
            .artifacts
            .into_iter()
            .map(|artifact| v1::EvidenceArtifact {
                name: artifact.name,
                sha256: artifact.sha256,
            })
            .collect(),
    }
}

fn watch_response(task: DevelopmentTask) -> v1::WatchDevelopmentTaskResponse {
    let resource_version = task.resource_version;
    v1::WatchDevelopmentTaskResponse {
        task: Some(development_task(task)),
        resource_version,
    }
}

const fn development_phase(value: DevelopmentPhase) -> v1::DevelopmentTaskPhase {
    match value {
        DevelopmentPhase::Queued => v1::DevelopmentTaskPhase::Queued,
        DevelopmentPhase::Coding => v1::DevelopmentTaskPhase::Coding,
        DevelopmentPhase::WaitingForInput => v1::DevelopmentTaskPhase::WaitingForInput,
        DevelopmentPhase::ValidationQueued => v1::DevelopmentTaskPhase::ValidationQueued,
        DevelopmentPhase::Validating => v1::DevelopmentTaskPhase::Validating,
        DevelopmentPhase::PrQueued => v1::DevelopmentTaskPhase::PrQueued,
        DevelopmentPhase::PreparingPr => v1::DevelopmentTaskPhase::PreparingPr,
        DevelopmentPhase::Completed => v1::DevelopmentTaskPhase::Completed,
        DevelopmentPhase::Failed => v1::DevelopmentTaskPhase::Failed,
        DevelopmentPhase::Cancelled => v1::DevelopmentTaskPhase::Cancelled,
        DevelopmentPhase::Unsupported => v1::DevelopmentTaskPhase::Unsupported,
    }
}

fn development_phase_from_proto(
    value: i32,
    context: &RequestContext,
) -> Result<DevelopmentPhase, Status> {
    match v1::DevelopmentTaskPhase::try_from(value) {
        Ok(v1::DevelopmentTaskPhase::Queued) => Ok(DevelopmentPhase::Queued),
        Ok(v1::DevelopmentTaskPhase::Coding) => Ok(DevelopmentPhase::Coding),
        Ok(v1::DevelopmentTaskPhase::WaitingForInput) => Ok(DevelopmentPhase::WaitingForInput),
        Ok(v1::DevelopmentTaskPhase::ValidationQueued) => Ok(DevelopmentPhase::ValidationQueued),
        Ok(v1::DevelopmentTaskPhase::Validating) => Ok(DevelopmentPhase::Validating),
        Ok(v1::DevelopmentTaskPhase::PrQueued) => Ok(DevelopmentPhase::PrQueued),
        Ok(v1::DevelopmentTaskPhase::PreparingPr) => Ok(DevelopmentPhase::PreparingPr),
        Ok(v1::DevelopmentTaskPhase::Completed) => Ok(DevelopmentPhase::Completed),
        Ok(v1::DevelopmentTaskPhase::Failed) => Ok(DevelopmentPhase::Failed),
        Ok(v1::DevelopmentTaskPhase::Cancelled) => Ok(DevelopmentPhase::Cancelled),
        Ok(v1::DevelopmentTaskPhase::Unsupported) => Ok(DevelopmentPhase::Unsupported),
        Ok(v1::DevelopmentTaskPhase::Unspecified) | Err(_) => {
            Err(context.invalid("phase must be a recognized non-unspecified value"))
        }
    }
}

const fn terminal_phase(value: DevelopmentPhase) -> bool {
    matches!(
        value,
        DevelopmentPhase::Completed
            | DevelopmentPhase::Failed
            | DevelopmentPhase::Cancelled
            | DevelopmentPhase::Unsupported
    )
}

fn terminal_task(task: &DevelopmentTask) -> bool {
    terminal_phase(task.phase)
        && task.model_usage.collection_status != ModelUsageCollectionStatus::Collecting
}

fn task_failure_reason(task: &DevelopmentTask) -> v1::ErrorReason {
    failure_reason(
        task.lifecycle
            .iter()
            .rev()
            .find_map(|event| event.reason_code.as_deref()),
    )
}

fn failure_reason(reason: Option<&str>) -> v1::ErrorReason {
    match reason {
        Some("operator_cancelled") => v1::ErrorReason::Cancelled,
        Some("deadline_exceeded" | "task_deadline_exceeded" | "task_deadline_expired") => {
            v1::ErrorReason::TaskDeadlineExpired
        }
        Some(reason) if reason.contains("repository") => v1::ErrorReason::RepositoryAccess,
        Some(reason) if reason.contains("provider") => v1::ErrorReason::ProviderUnavailable,
        Some("scope_rejected" | "policy_rejected" | "dependency_change_unsupported") => {
            v1::ErrorReason::FailedPrecondition
        }
        _ => v1::ErrorReason::Internal,
    }
}

fn timestamp_millis(value: i64) -> Timestamp {
    let seconds = value.div_euclid(1000);
    let nanos = i32::try_from(value.rem_euclid(1000) * 1_000_000).unwrap_or_default();
    Timestamp { seconds, nanos }
}

fn duration_millis(value: u64) -> Duration {
    Duration {
        seconds: i64::try_from(value / 1000).unwrap_or(i64::MAX),
        nanos: i32::try_from((value % 1000) * 1_000_000).unwrap_or_default(),
    }
}

fn duration_seconds(value: Duration, context: &RequestContext) -> Result<u64, Status> {
    if value.nanos != 0 || !(1..=86_400).contains(&value.seconds) {
        return Err(
            context.invalid("deadline must be a whole duration from 1 second through 24 hours")
        );
    }
    u64::try_from(value.seconds)
        .map_err(|_| context.invalid("deadline must be a positive duration"))
}

#[derive(Clone, PartialEq, Message)]
struct RichStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<Any>,
}

const fn grpc_code(reason: ApplicationErrorReason) -> Code {
    match reason {
            ApplicationErrorReason::InvalidIdentifier
            | ApplicationErrorReason::InvalidInput
            | ApplicationErrorReason::InvalidSourceRevision
            | ApplicationErrorReason::InvalidLockDigest => Code::InvalidArgument,
            ApplicationErrorReason::IdempotencyKeyReused => Code::AlreadyExists,
            ApplicationErrorReason::ResourceNotFound
            | ApplicationErrorReason::EvidenceUnavailable => Code::NotFound,
            ApplicationErrorReason::Conflict => Code::Aborted,
            ApplicationErrorReason::FailedPrecondition
            | ApplicationErrorReason::TaskNotActionable
            | ApplicationErrorReason::ModelUnavailable
            | ApplicationErrorReason::RepositoryNotReady
            | ApplicationErrorReason::TaskPhaseNotEligible
            | ApplicationErrorReason::TaskDeadlineExpired => Code::FailedPrecondition,
            ApplicationErrorReason::ProviderUnavailable
            | ApplicationErrorReason::RepositoryAccess
            | ApplicationErrorReason::DependencyUnavailable => Code::Unavailable,
            ApplicationErrorReason::Cancelled => Code::Cancelled,
            ApplicationErrorReason::DeadlineExceeded => Code::DeadlineExceeded,
            ApplicationErrorReason::Internal => Code::Internal,
        }
}

const fn protobuf_error_reason(reason: ApplicationErrorReason) -> v1::ErrorReason {
    match reason {
        ApplicationErrorReason::InvalidIdentifier => v1::ErrorReason::InvalidIdentifier,
        ApplicationErrorReason::InvalidInput => v1::ErrorReason::InvalidInput,
        ApplicationErrorReason::IdempotencyKeyReused => v1::ErrorReason::IdempotencyKeyReused,
        ApplicationErrorReason::ResourceNotFound => v1::ErrorReason::ResourceNotFound,
        ApplicationErrorReason::Conflict => v1::ErrorReason::Conflict,
        ApplicationErrorReason::FailedPrecondition => v1::ErrorReason::FailedPrecondition,
        ApplicationErrorReason::TaskNotActionable => v1::ErrorReason::TaskNotActionable,
        ApplicationErrorReason::ProviderUnavailable => v1::ErrorReason::ProviderUnavailable,
        ApplicationErrorReason::ModelUnavailable => v1::ErrorReason::ModelUnavailable,
        ApplicationErrorReason::RepositoryAccess => v1::ErrorReason::RepositoryAccess,
        ApplicationErrorReason::EvidenceUnavailable => v1::ErrorReason::EvidenceUnavailable,
        ApplicationErrorReason::DependencyUnavailable | ApplicationErrorReason::Internal => {
            v1::ErrorReason::Internal
        }
        ApplicationErrorReason::Cancelled => v1::ErrorReason::Cancelled,
        ApplicationErrorReason::DeadlineExceeded => v1::ErrorReason::DeadlineExceeded,
        ApplicationErrorReason::InvalidSourceRevision => v1::ErrorReason::InvalidSourceRevision,
        ApplicationErrorReason::InvalidLockDigest => v1::ErrorReason::InvalidLockDigest,
        ApplicationErrorReason::RepositoryNotReady => v1::ErrorReason::RepositoryNotReady,
        ApplicationErrorReason::TaskPhaseNotEligible => v1::ErrorReason::TaskPhaseNotEligible,
        ApplicationErrorReason::TaskDeadlineExpired => v1::ErrorReason::TaskDeadlineExpired,
    }
}

fn application_status(error: ApplicationError, request_id: Option<&str>) -> Status {
    let code = grpc_code(error.reason);
    let reason = protobuf_error_reason(error.reason);
    let message = crate::truncate_utf8(&error.message, 512);
    let detail = v1::ErrorDetail {
        reason: reason as i32,
        message: message.clone(),
        resource_type: error.resource_type.map(str::to_owned),
        resource_id: error.resource_id,
        metadata: Vec::new(),
        request_id: request_id.map(str::to_owned),
    };
    let rich = RichStatus {
        code: code as i32,
        message: message.clone(),
        details: vec![Any {
            type_url: ERROR_DETAIL_TYPE_URL.to_owned(),
            value: detail.encode_to_vec(),
        }],
    };
    let mut status = Status::with_details(code, message, rich.encode_to_vec().into());
    if let Some(request_id) = request_id
        && let Ok(value) = MetadataValue::from_str(request_id)
    {
        status.metadata_mut().insert("x-request-id", value);
    }
    status
}
