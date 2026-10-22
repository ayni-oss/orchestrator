//! Application-authorized proposal materialization; reports cannot choose scope.
use super::{StoreError, plans, prepare_head, record_update};
use crate::plans::{AgentPlanProvenance, PlanContent, PlanScope};
use crate::store::support;
use ayni_orchestrator_events::{
    EventIdentity,
    plan_proposals::validate_proposal,
    v1::{ConversationTurnBinding, ConversationTurnReport, ProviderCallEvidence},
};
use prost::Message as _;
use tokio_postgres::Transaction;

pub(in crate::store) struct PreparedProposal {
    content: PlanContent,
    bytes: Vec<u8>,
    provenance: AgentPlanProvenance,
}

/// Failure here is an accepted failed outcome, so capacity/malformed drafts do
/// not strand the turn or authorize an automatic paid retry.
// The caller holds the owning conversation lock acquired by detached_rejection
// before the turn lock, serializing plan capacity, revisions and cancellation.
pub(in crate::store) async fn prepare(
    tx: &Transaction<'_>,
    binding: &ConversationTurnBinding,
    report: &ConversationTurnReport,
    identity: &EventIdentity,
    report_digest: &str,
) -> Result<Result<Option<PreparedProposal>, &'static str>, StoreError> {
    let Some(proposal) = &report.plan_proposal else {
        return Ok(Ok(None));
    };
    let Some(evidence) = &report.provider_call else {
        return Ok(Err("invalid_plan_proposal"));
    };
    if binding.execution_binding_revision != "openai-responses-v1"
        || evidence.provider != "openai"
        || evidence.state != "completed"
        || evidence.response_id.is_empty()
        || report.kind != 2
    {
        return Ok(Err("invalid_plan_proposal"));
    }
    let Some(row) = tx
        .query_opt(
            "SELECT evidence FROM workflow_provider_calls WHERE turn_id=$1",
            &[&binding.turn_id],
        )
        .await?
    else {
        return Ok(Err("invalid_plan_proposal"));
    };
    let admitted = ProviderCallEvidence::decode(row.get::<_, Vec<u8>>(0).as_slice())
        .map_err(|_| StoreError::CorruptState)?;
    if !same_call(evidence, &admitted) {
        return Ok(Err("invalid_plan_proposal"));
    }
    let Ok(parsed) = validate_proposal(&proposal.proposal_json) else {
        return Ok(Err("invalid_plan_proposal"));
    };
    let bytes = serde_json::to_vec(&parsed.content).map_err(|_| StoreError::CorruptState)?;
    let content: PlanContent =
        serde_json::from_slice(&bytes).map_err(|_| StoreError::CorruptState)?;
    if plans::canonical_content(&content)? != bytes {
        return Err(StoreError::CorruptState);
    }
    let count: i64 = tx
        .query_one(
            "SELECT count(*) FROM workflow_plans WHERE conversation_id=$1",
            &[&binding.conversation_id],
        )
        .await?
        .get(0);
    if count >= plans::MAX_PLANS {
        return Ok(Err("plan_capacity"));
    }
    Ok(Ok(Some(PreparedProposal {
        content,
        bytes,
        provenance: AgentPlanProvenance {
            source_turn_id: binding.turn_id.clone(),
            configuration_revision_id: admitted.configuration_revision_id,
            context_digest: admitted.context_digest,
            provider_call_id: admitted.call_id,
            provider_response_id: evidence.response_id.clone(),
            assignment_id: binding.assignment_id.clone(),
            executor_generation: binding.executor_generation,
            report_source: identity.source.clone(),
            report_id: identity.id.clone(),
            report_digest: report_digest.into(),
            invocation_id: proposal.invocation_id.clone(),
            assumptions: parsed.assumptions,
            evidence: parsed.evidence,
        },
    })))
}

fn same_call(evidence: &ProviderCallEvidence, admitted: &ProviderCallEvidence) -> bool {
    evidence.call_id == admitted.call_id
        && evidence.provider == admitted.provider
        && evidence.model == admitted.model
        && evidence.configuration_revision_id == admitted.configuration_revision_id
        && evidence.context_digest == admitted.context_digest
        && evidence.request_digest == admitted.request_digest
        && evidence.reserved_input_tokens == admitted.reserved_input_tokens
        && evidence.reserved_output_tokens == admitted.reserved_output_tokens
        && evidence.reserved_total_tokens == admitted.reserved_total_tokens
}

pub(in crate::store) async fn retain(
    tx: &Transaction<'_>,
    binding: &ConversationTurnBinding,
    proposal: &PreparedProposal,
) -> Result<(), StoreError> {
    let scope = PlanScope {
        workspace: &binding.workspace_id,
        project: &binding.project_id,
        conversation: &binding.conversation_id,
    };
    let plan = support::new_id();
    let version = support::new_id();
    prepare_head(tx, scope, &plan, true, "", "", &version).await?;
    tx.execute("INSERT INTO workflow_plan_versions (id,workspace_id,project_id,conversation_id,plan_id,ordinal,content,content_digest,title,author) VALUES ($1,$2,$3,$4,$5,1,$6,$7,$8,'agent')", &[&version,&scope.workspace,&scope.project,&scope.conversation,&plan,&proposal.bytes,&plans::content_digest(&proposal.bytes),&proposal.content.title]).await?;
    let p = &proposal.provenance;
    tx.execute("INSERT INTO workflow_plan_proposals (version_id,plan_id,workspace_id,project_id,conversation_id,source_turn_id,configuration_revision_id,context_digest,provider_call_id,provider_response_id,assignment_id,executor_generation,report_source,report_id,report_digest,invocation_id,assumptions,evidence) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)", &[&version,&plan,&scope.workspace,&scope.project,&scope.conversation,&p.source_turn_id,&p.configuration_revision_id,&p.context_digest,&p.provider_call_id,&p.provider_response_id,&p.assignment_id,&i64::try_from(p.executor_generation).map_err(|_| StoreError::CorruptState)?,&p.report_source,&p.report_id,&p.report_digest,&p.invocation_id,&p.assumptions,&p.evidence]).await?;
    let revision: i64 = tx
        .query_one(
            "SELECT revision FROM workflow_conversations WHERE id=$1",
            &[&binding.conversation_id],
        )
        .await?
        .get(0);
    record_update(
        tx,
        scope,
        revision.checked_add(1).ok_or(StoreError::CorruptState)?,
        &plan,
        &version,
    )
    .await?;
    Ok(())
}
