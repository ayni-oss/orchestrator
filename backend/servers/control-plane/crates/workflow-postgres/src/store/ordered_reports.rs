use super::{StoreError, WorkflowStore};
use ayni_orchestrator_events::workflow::decode_report;
use sha2::{Digest as _, Sha256};

const PENDING_LOCK: i64 = 4_702_199_126_064_926;

impl WorkflowStore {
    /// Retain a terminal report that awaits its ownership claim. The durable pending
    /// row authorizes a transport ACK, never execution or a client-visible outcome.
    ///
    /// # Errors
    /// Returns conflicts, capacity, invalid scope, or database errors.
    pub async fn retain_ordered_report(&self, bytes: &[u8]) -> Result<bool, StoreError> {
        let (identity, _, report) =
            decode_report(bytes).map_err(|_| StoreError::InvalidInput("executor_report"))?;
        if report.report_sequence != 2 {
            return Ok(false);
        }
        let binding = report
            .binding
            .as_ref()
            .ok_or(StoreError::InvalidInput("executor_report_binding"))?;
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        if super::acceptance::detached_report_decision(&transaction, bytes)
            .await?
            .is_some()
        {
            transaction.commit().await?;
            driver.abort();
            return Ok(true);
        }

        let authority=transaction.query_opt("SELECT t.execution_state,t.assignment_id,t.executor_generation,t.execution_binding_revision,t.context_revision,d.command_event_id,d.exact_command_bytes FROM workflow_turns t JOIN workflow_dispatch_intents d ON d.turn_id=t.id WHERE t.workspace_id=$1 AND t.project_id=$2 AND t.conversation_id=$3 AND t.id=$4 FOR UPDATE OF t", &[&binding.workspace_id,&binding.project_id,&binding.conversation_id,&binding.turn_id]).await?.ok_or(StoreError::InvalidInput("executor_report_scope"))?;
        if authority.get::<_, String>(0) != "not_started" {
            transaction.commit().await?;
            driver.abort();
            return Ok(false);
        }
        let command: Option<Vec<u8>> = authority.get(6);
        let eligible = binding.assignment_id == authority.get::<_, String>(1)
            && i64::try_from(binding.executor_generation).ok() == Some(authority.get::<_, i64>(2))
            && binding.execution_binding_revision == authority.get::<_, String>(3)
            && i64::try_from(binding.context_revision).ok() == Some(authority.get::<_, i64>(4))
            && binding.command_event_id == authority.get::<_, String>(5)
            && command.is_some_and(|bytes| {
                binding.command_digest == format!("sha256:{:x}", Sha256::digest(bytes))
            });
        if !eligible {
            transaction.commit().await?;
            driver.abort();
            return Ok(false);
        }
        transaction
            .query_one("SELECT pg_advisory_xact_lock($1)", &[&PENDING_LOCK])
            .await?;
        let prior=transaction.query_opt("SELECT report_digest FROM workflow_pending_reports WHERE report_source=$1 AND report_id=$2",&[&identity.source,&identity.id]).await?;
        if let Some(prior) = prior {
            if prior.get::<_, String>(0) != digest {
                return Err(StoreError::InvalidInput("pending_report_conflict"));
            }
        } else {
            let totals=transaction.query_one("SELECT count(*),COALESCE(sum(octet_length(exact_bytes)+8192),0)::bigint FROM workflow_pending_reports",&[]).await?;
            let added = i64::try_from(bytes.len() + 8192)
                .map_err(|_| StoreError::Capacity("pending_reports"))?;
            if totals.get::<_, i64>(0) >= 50_000
                || totals.get::<_, i64>(1).saturating_add(added) > 192 * 1024 * 1024
            {
                return Err(StoreError::Capacity("pending_reports"));
            }
            transaction.execute("INSERT INTO workflow_pending_reports(report_source,report_id,report_digest,exact_bytes,turn_id) VALUES($1,$2,$3,$4,$5)",&[&identity.source,&identity.id,&digest,&bytes,&binding.turn_id]).await?;
        }
        transaction.commit().await?;
        driver.abort();
        Ok(true)
    }

    /// Reconcile committed out-of-order reports independently of broker redelivery.
    /// # Errors
    /// Returns database, verification, or capacity failures without dropping pending bytes.
    pub async fn reconcile_pending_reports(&self) -> Result<u64, StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows=client.query("SELECT p.report_source,p.report_id,p.report_digest,p.exact_bytes FROM workflow_pending_reports p JOIN workflow_turns t ON t.id=p.turn_id WHERE t.execution_state <> 'not_started' ORDER BY p.recorded_at LIMIT 64",&[]).await?;
        let mut count = 0;
        for row in rows {
            let source: String = row.get(0);
            let id: String = row.get(1);
            let digest: String = row.get(2);
            let bytes: Vec<u8> = row.get(3);
            self.accept_terminal_report(&bytes).await?;
            count+=client.execute("DELETE FROM workflow_pending_reports WHERE report_source=$1 AND report_id=$2 AND report_digest=$3",&[&source,&id,&digest]).await?;
        }
        driver.abort();
        Ok(count)
    }
}
