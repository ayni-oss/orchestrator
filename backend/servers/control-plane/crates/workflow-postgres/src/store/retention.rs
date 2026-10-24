//! Delayed source reclamation never removes approval, result or recovery evidence.
use super::{StoreError, WorkflowStore};

// A finished draft has a retained remote identity. A confirmed continuation has
// its own exact source archive. All other attempts remain recoverable indefinitely.
const ELIGIBLE: &str = "r.state IN ('completed','failed','cancelled','interrupted') \
    AND r.provider_state NOT IN ('sending','uncertain') \
    AND (EXISTS (SELECT 1 FROM workflow_task_publications p WHERE p.attempt_id=r.attempt_id AND p.state='draft') \
         OR EXISTS (SELECT 1 FROM workflow_task_recovery_confirmations c \
                    JOIN workflow_task_runs child ON child.attempt_id=c.attempt_id \
                    WHERE c.parent_attempt_id=r.attempt_id AND octet_length(child.source_pack)>0)) \
    AND NOT EXISTS (SELECT 1 FROM workflow_task_publications p WHERE p.attempt_id=r.attempt_id \
                    AND p.state NOT IN ('draft','cancelled','blocked')) \
    AND NOT EXISTS (SELECT 1 FROM workflow_task_recovery q WHERE q.attempt_id=r.attempt_id AND q.kind='question' \
                    AND NOT EXISTS(SELECT 1 FROM workflow_task_recovery a WHERE a.question_id=q.id)) \
    AND NOT EXISTS (SELECT 1 FROM workflow_task_recovery p WHERE p.attempt_id=r.attempt_id AND p.kind='proposal' \
                    AND NOT EXISTS(SELECT 1 FROM workflow_task_recovery_confirmations c WHERE c.parent_attempt_id=r.attempt_id))";

impl WorkflowStore {
    /// Start a thirty-day grace period, then reclaim at most 32 superseded archives.
    /// Commands, accepted patches, quality artifacts and decision receipts survive.
    ///
    /// # Errors
    /// Fails atomically when the authoritative database is unavailable.
    pub async fn cleanup_retained_task_sources(&self) -> Result<u64, StoreError> {
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        // Match recovery's lock order so confirmation cannot race source reclamation.
        super::task_runs::lock(&tx).await?;
        super::publications::lock(&tx).await?;
        tx.execute(&format!("UPDATE workflow_task_runs r SET source_reclaim_after=NULL \
            WHERE attempt_id IN (SELECT r.attempt_id FROM workflow_task_runs r WHERE source_retired_at IS NULL \
            AND source_reclaim_after IS NOT NULL AND NOT ({ELIGIBLE}) ORDER BY r.attempt_id LIMIT 32)"), &[]).await?;
        tx.execute(&format!("UPDATE workflow_task_runs r SET source_reclaim_after=clock_timestamp()+interval '30 days' \
            WHERE attempt_id IN (SELECT r.attempt_id FROM workflow_task_runs r WHERE source_retired_at IS NULL \
            AND source_reclaim_after IS NULL AND {ELIGIBLE} ORDER BY r.attempt_id LIMIT 32)"), &[]).await?;
        let removed = tx.execute(&format!("UPDATE workflow_task_runs SET \
            source_retired_bytes=octet_length(source_pack),source_pack=''::bytea,source_retired_at=clock_timestamp() \
            WHERE attempt_id IN (SELECT r.attempt_id FROM workflow_task_runs r WHERE source_retired_at IS NULL \
            AND source_reclaim_after<=clock_timestamp() AND {ELIGIBLE} ORDER BY source_reclaim_after,r.attempt_id LIMIT 32)"), &[]).await?;
        tx.commit().await?;
        driver.abort();
        Ok(removed)
    }
}
