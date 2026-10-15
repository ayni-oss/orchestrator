//! Safe observations of retained workflow identities; no prompts, evidence bodies or bindings.
use super::v1;
use ayni_orchestrator_observability::workflow::Workflow;

pub(super) fn conversation(c: &v1::ProjectConversation) {
    let base = Workflow {
        project_id: c.project_id.clone(),
        conversation_id: c.id.clone(),
        ..Workflow::default()
    };
    if let Some(message) = c.messages.last() {
        Workflow {
            message_id: message.id.clone(),
            run_id: message.run_id.clone(),
            ..base.clone()
        }
        .observe("conversation.turn", &message.role);
    }
    if let Some(run) = c.runs.last() {
        let workflow = Workflow {
            run_id: run.id.clone(),
            model: run.model.clone(),
            profile_revision: run
                .planner_profile
                .as_ref()
                .map_or_else(String::new, |p| p.revision.clone()),
            ..base.clone()
        };
        let span = workflow.span("research.state");
        span.record("outcome", run.state.as_str());
        span.record("model_calls", run.model_calls);
        if let Some(usage) = &run.token_usage {
            if let Some(tokens) = usage.input_tokens {
                span.record("input_tokens", tokens);
            }
            if let Some(tokens) = usage.output_tokens {
                span.record("output_tokens", tokens);
            }
        }
    }
    if let Some(plan) = &c.plan {
        if let Some(version) = plan.versions.last() {
            Workflow {
                plan_id: plan.id.clone(),
                plan_version: version.version,
                run_id: version.run_id.clone(),
                ..base.clone()
            }
            .observe("plan.version", &version.state);
        }
        for proposal in c.proposals.iter().rev().take(100) {
            if let Some(version) = plan
                .versions
                .iter()
                .find(|v| v.proposal_ids.contains(&proposal.id))
            {
                Workflow {
                    plan_id: plan.id.clone(),
                    plan_version: version.version,
                    run_id: proposal.run_id.clone(),
                    proposal_id: proposal.id.clone(),
                    task_id: proposal.task_id.clone(),
                    ..base.clone()
                }
                .observe("approval.handoff", &proposal.state);
            }
        }
    }
}

pub(super) fn outcome(attempt: &v1::DevelopmentAttempt, origin: &v1::TaskOrigin) {
    let workflow = Workflow {
        project_id: origin.project_id.clone(),
        conversation_id: origin.conversation_id.clone(),
        plan_id: origin.plan_id.clone(),
        plan_version: origin.plan_version,
        run_id: origin.research_run_id.clone(),
        proposal_id: origin.proposal_id.clone(),
        task_id: attempt.development_task_id.clone(),
        attempt_id: attempt.id.clone(),
        profile_revision: attempt
            .snapshot
            .as_ref()
            .and_then(|s| s.agent_profile.as_ref())
            .map_or_else(String::new, |p| p.revision.clone()),
        ..Workflow::default()
    };
    let span = workflow.span("worker.outcome");
    span.record("outcome", "observed");
    if let Some(result) = &attempt.result {
        span.record("execution", result.execution);
        span.record("publication", result.publication);
        if let Some(quality) = &result.quality {
            span.record("quality", quality.outcome);
        }
    }
}
