use super::{Decision, ParsedReport, ReportDecisionReceipt, StoreError, decision_fact};
use ayni_orchestrator_events::workflow::decision_subject;
use tokio_postgres::Transaction;

pub(super) async fn detached_rejection(
    transaction: &Transaction<'_>,
    parsed: &ParsedReport,
    bytes: &[u8],
) -> Result<Option<ReportDecisionReceipt>, StoreError> {
    let binding = parsed
        .report
        .binding
        .as_ref()
        .ok_or(StoreError::CorruptState)?;
    // Serialize one report identity across distinct turns and differently scoped
    // malformed requests. A hash collision merely serializes unrelated work.
    let identity = format!("{}\u{1f}{}", parsed.identity.source, parsed.identity.id);
    transaction
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
            &[&identity],
        )
        .await?;
    // Align with admission's conversation-before-turn-before-capacity lock order.
    transaction.query_opt("SELECT c.id FROM workflow_conversations c JOIN workflow_turns t ON t.conversation_id=c.id WHERE t.id=$1 FOR UPDATE OF c",&[&binding.turn_id]).await?;
    if let Some(row)=transaction.query_opt("SELECT decision_event_id,safe_reason FROM workflow_transport_rejections WHERE report_source=$1 AND report_id=$2 AND report_digest=$3",&[&parsed.identity.source,&parsed.identity.id,&parsed.digest]).await? {
        return Ok(Some(receipt(parsed,row.get(0),row.get(1))));
    }
    let conflict:bool=transaction.query_one(
        "SELECT EXISTS(SELECT 1 FROM workflow_report_receipts WHERE report_source=$1 AND report_id=$2 AND report_digest<>$3) OR EXISTS(SELECT 1 FROM workflow_pending_reports WHERE report_source=$1 AND report_id=$2 AND report_digest<>$3) OR EXISTS(SELECT 1 FROM workflow_transport_rejections WHERE report_source=$1 AND report_id=$2 AND report_digest<>$3)",
        &[&parsed.identity.source,&parsed.identity.id,&parsed.digest]).await?.get(0);
    let scope:bool=transaction.query_one("SELECT EXISTS(SELECT 1 FROM workflow_turns WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND id=$4)",&[&binding.workspace_id,&binding.project_id,&binding.conversation_id,&binding.turn_id]).await?.get(0);
    let reason = if conflict {
        "event_conflict"
    } else if !scope {
        "wrong_scope"
    } else {
        return Ok(None);
    };
    super::super::capacity::reserve_decision(transaction, true).await?;
    let decision = Decision {
        accepted: false,
        safe_reason: reason,
        turn_state: "queued",
        accepted_executor: None,
    };
    let (decision_id, decision_bytes) =
        decision_fact(transaction, parsed, binding, &decision).await?;
    let subject = decision_subject(&parsed.report.executor_instance_id)
        .map_err(|_| StoreError::CorruptState)?;
    transaction.execute("INSERT INTO workflow_transport_rejections(report_source,report_id,report_digest,exact_report_bytes,decision_event_id,decision_bytes,executor_instance_id,safe_reason,subject) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)",&[&parsed.identity.source,&parsed.identity.id,&parsed.digest,&bytes,&decision_id,&decision_bytes,&parsed.report.executor_instance_id,&reason,&subject]).await?;
    Ok(Some(receipt(parsed, decision_id, reason.into())))
}

fn receipt(
    parsed: &ParsedReport,
    decision_event_id: String,
    safe_reason: String,
) -> ReportDecisionReceipt {
    ReportDecisionReceipt {
        report_source: parsed.identity.source.clone(),
        report_id: parsed.identity.id.clone(),
        decision_event_id,
        accepted: false,
        safe_reason,
    }
}
