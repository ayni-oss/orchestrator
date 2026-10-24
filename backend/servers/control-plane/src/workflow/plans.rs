//! Projection only: plan authority stays in `PostgreSQL`, outside the provider worker.
use super::{RequestContext, Status, timestamp, v1};
use ayni_orchestrator_workflow_postgres::{StoreError, plans as model};

pub(super) const fn scope<'a>(
    workspace: &'a str,
    project: &'a str,
    conversation: &'a str,
) -> model::PlanScope<'a> {
    model::PlanScope {
        workspace,
        project,
        conversation,
    }
}

pub(super) fn content(
    value: Option<v1::PlanContent>,
    context: &RequestContext,
) -> Result<model::PlanContent, Status> {
    let value = value.ok_or_else(|| context.store(&StoreError::InvalidInput("plan_content")))?;
    Ok(model::PlanContent {
        title: value.title,
        objective: value.objective,
        scope: value.scope,
        out_of_scope: value.out_of_scope,
        constraints: value.constraints,
        unresolved_questions: value.unresolved_questions,
        tasks: value
            .tasks
            .into_iter()
            .map(|t| model::PlanTaskSpecification {
                key: t.key,
                objective: t.objective,
                scope: t.scope,
                out_of_scope: t.out_of_scope,
                acceptance_criteria: t.acceptance_criteria,
                repository: t.repository,
                source_ref: t.source_ref,
                execution_limits: t.execution_limits,
                unresolved_inputs: t.unresolved_inputs,
                depends_on: t.depends_on,
                source_paths: t.source_paths,
            })
            .collect(),
    })
}

pub(super) fn version(value: model::PlanVersion) -> v1::PlanVersion {
    let c = value.content;
    v1::PlanVersion {
        id: value.id,
        plan_id: value.plan_id,
        workspace_id: value.workspace_id,
        project_id: value.project_id,
        conversation_id: value.conversation_id,
        ordinal: value.ordinal,
        predecessor_id: value.predecessor_id,
        content_digest: value.content_digest,
        author: value.author,
        created_at: Some(timestamp(value.created_at)),
        provenance: value.provenance.map(|p| v1::AgentPlanProvenance {
            source_turn_id: p.source_turn_id,
            configuration_revision_id: p.configuration_revision_id,
            context_digest: p.context_digest,
            provider_call_id: p.provider_call_id,
            provider_response_id: p.provider_response_id,
            assignment_id: p.assignment_id,
            executor_generation: p.executor_generation,
            report_source: p.report_source,
            report_id: p.report_id,
            report_digest: p.report_digest,
            invocation_id: p.invocation_id,
            assumptions: p.assumptions,
            evidence: p.evidence,
        }),
        content: Some(v1::PlanContent {
            title: c.title,
            objective: c.objective,
            scope: c.scope,
            out_of_scope: c.out_of_scope,
            constraints: c.constraints,
            unresolved_questions: c.unresolved_questions,
            tasks: c
                .tasks
                .into_iter()
                .map(|t| v1::PlanTaskSpecification {
                    key: t.key,
                    objective: t.objective,
                    scope: t.scope,
                    out_of_scope: t.out_of_scope,
                    acceptance_criteria: t.acceptance_criteria,
                    repository: t.repository,
                    source_ref: t.source_ref,
                    execution_limits: t.execution_limits,
                    unresolved_inputs: t.unresolved_inputs,
                    depends_on: t.depends_on,
                    source_paths: t.source_paths,
                })
                .collect(),
        }),
    }
}

pub(super) fn summary(v: model::PlanSummary) -> v1::PlanSummary {
    v1::PlanSummary {
        id: v.id,
        workspace_id: v.workspace_id,
        project_id: v.project_id,
        conversation_id: v.conversation_id,
        current_version_id: v.current_version_id,
        current_ordinal: v.current_ordinal,
        title: v.title,
        follows_plan_id: v.follows_plan_id,
        approval_id: v.approval_id,
    }
}

pub(super) fn written(v: model::WrittenPlan) -> v1::CreatePlanResponse {
    let r = v.receipt;
    v1::CreatePlanResponse {
        version: Some(version(v.version)),
        receipt: Some(v1::PlanWriteReceipt {
            command_id: r.command_id,
            plan_id: r.plan_id,
            version_id: r.version_id,
            conversation_revision: r.conversation_revision,
            resume_cursor: r.resume_cursor,
        }),
    }
}

pub(super) fn page(v: model::VersionPage) -> v1::ListPlanVersionsResponse {
    v1::ListPlanVersionsResponse {
        versions: v
            .versions
            .into_iter()
            .map(|r| v1::PlanVersionSummary {
                id: r.id,
                ordinal: r.ordinal,
                content_digest: r.content_digest,
                author: r.author,
                created_at: Some(timestamp(r.created_at)),
            })
            .collect(),
        next_before_ordinal: v.next_before_ordinal,
    }
}

pub(super) fn revised(v: model::WrittenPlan) -> v1::RevisePlanResponse {
    let result = written(v);
    v1::RevisePlanResponse {
        version: result.version,
        receipt: result.receipt,
    }
}

pub(super) fn approval(v: model::PlanApprovalReceipt) -> v1::PlanApprovalReceipt {
    v1::PlanApprovalReceipt {
        id: v.id,
        command_id: v.command_id,
        workspace_id: v.workspace_id,
        project_id: v.project_id,
        conversation_id: v.conversation_id,
        plan_id: v.plan_id,
        version_id: v.version_id,
        content_digest: v.content_digest,
        operator: v.operator,
        request_digest: v.request_digest,
        created_at: Some(timestamp(v.created_at)),
        conversation_revision: v.conversation_revision,
        resume_cursor: v.resume_cursor,
        task_ids: v.task_ids,
    }
}

pub(super) fn task(v: model::AdmittedTask) -> v1::AdmittedTask {
    let t = v.specification;
    v1::AdmittedTask {
        id: v.id,
        workspace_id: v.workspace_id,
        project_id: v.project_id,
        conversation_id: v.conversation_id,
        plan_id: v.plan_id,
        version_id: v.version_id,
        content_digest: v.content_digest,
        approval_id: v.approval_id,
        proposed_task_key: v.proposed_task_key,
        specification: Some(v1::PlanTaskSpecification {
            key: t.key,
            objective: t.objective,
            scope: t.scope,
            out_of_scope: t.out_of_scope,
            acceptance_criteria: t.acceptance_criteria,
            repository: t.repository,
            source_ref: t.source_ref,
            execution_limits: t.execution_limits,
            unresolved_inputs: t.unresolved_inputs,
            depends_on: t.depends_on,
            source_paths: t.source_paths,
        }),
        state: v.state,
        execution_blockers: v.execution_blockers,
        created_at: Some(timestamp(v.created_at)),
    }
}

pub(super) fn approved(v: model::ApprovedPlan) -> v1::ApprovePlanResponse {
    v1::ApprovePlanResponse {
        approval: Some(approval(v.approval)),
        tasks: v.tasks.into_iter().map(task).collect(),
    }
}

pub(super) fn preparation(
    v: ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
) -> v1::TaskExecutionPreparation {
    let i = v.inputs;
    v1::TaskExecutionPreparation {
        command_id: v.command_id,
        task_id: v.task_id,
        attempt_id: v.attempt_id,
        inputs: Some(v1::TaskExecutionInputs {
            repository: i.repository,
            source_ref: i.source_ref,
            source_commit: i.source_commit,
            lock_digest: i.lock_digest,
            runtime_image: i.runtime_image,
            max_seconds: i.max_seconds,
            max_tokens: i.max_tokens,
            upstream_selections: i
                .upstream_selections
                .into_iter()
                .map(|s| v1::TaskUpstreamSelection {
                    task_id: s.task_id,
                    attempt_id: s.attempt_id,
                    commit_sha: s.commit_sha,
                })
                .collect(),
        }),
        blockers: v.blockers,
        configuration_revision_id: v.configuration_revision_id,
        conversation_revision: v.conversation_revision,
        resume_cursor: v.resume_cursor,
        created_at: Some(timestamp(v.created_at)),
    }
}
