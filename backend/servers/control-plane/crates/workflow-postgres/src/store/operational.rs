//! Read-only operational projections; metrics never act as workflow authority.
use super::{StoreError, WorkflowStore};
use std::fmt::Write as _;

impl WorkflowStore {
    /// Read current durable backlog and state pressure without copying content.
    ///
    /// # Errors
    /// Returns an unavailable-authority error when `PostgreSQL` cannot be read.
    pub async fn operational_metrics(&self) -> Result<String, StoreError> {
        let (client, driver) = crate::connect(&self.database_url).await?;
        let mut output = String::new();
        for row in client
            .query(
                "SELECT scheduling_state, dispatch_state, execution_state, count(*) \
             FROM workflow_turns GROUP BY scheduling_state, dispatch_state, execution_state",
                &[],
            )
            .await?
        {
            let _ = writeln!(
                output,
                "workflow_turns{{scheduling=\"{}\",dispatch=\"{}\",execution=\"{}\"}} {}",
                row.get::<_, String>(0),
                row.get::<_, String>(1),
                row.get::<_, String>(2),
                row.get::<_, i64>(3)
            );
        }
        for row in client
            .query(
                "SELECT state, count(*), COALESCE(sum(octet_length(event_bytes)),0)::bigint, \
             COALESCE(max(extract(epoch FROM clock_timestamp()-created_at)),0)::double precision \
             FROM workflow_outbox GROUP BY state",
                &[],
            )
            .await?
        {
            let state: String = row.get(0);
            let _ = writeln!(
                output,
                "workflow_outbox_records{{state=\"{state}\"}} {}",
                row.get::<_, i64>(1)
            );
            let _ = writeln!(
                output,
                "workflow_outbox_bytes{{state=\"{state}\"}} {}",
                row.get::<_, i64>(2)
            );
            let _ = writeln!(
                output,
                "workflow_outbox_oldest_seconds{{state=\"{state}\"}} {}",
                row.get::<_, f64>(3)
            );
        }
        let row = client
            .query_one(
                "SELECT count(*), COALESCE(sum(octet_length(exact_command_bytes)),0)::bigint, \
             COALESCE(max(extract(epoch FROM clock_timestamp()-created_at)),0)::double precision \
             FROM workflow_dispatch_intents d WHERE eligible AND EXISTS (SELECT 1 FROM workflow_turns t WHERE t.id=d.turn_id AND t.dispatch_state<>'transferred')",
                &[],
            )
            .await?;
        let _ = writeln!(output, "workflow_dispatch_intents {}", row.get::<_, i64>(0));
        let _ = writeln!(output, "workflow_dispatch_bytes {}", row.get::<_, i64>(1));
        let _ = writeln!(
            output,
            "workflow_dispatch_oldest_seconds {}",
            row.get::<_, f64>(2)
        );
        let row = client
            .query_one(
                "SELECT count(*) FROM workflow_report_receipts r JOIN workflow_outbox o \
             ON o.id=r.decision_event_id WHERE o.state <> 'transferred'",
                &[],
            )
            .await?;
        let _ = writeln!(
            output,
            "workflow_report_decisions_retained {}",
            row.get::<_, i64>(0)
        );
        let pending = client.query_one("SELECT count(*),COALESCE(sum(octet_length(exact_bytes)),0)::bigint FROM workflow_pending_reports", &[]).await?;
        let _ = writeln!(
            output,
            "workflow_pending_reports {}",
            pending.get::<_, i64>(0)
        );
        let _ = writeln!(
            output,
            "workflow_pending_report_bytes {}",
            pending.get::<_, i64>(1)
        );
        append_queue_metrics(&client, &mut output).await?;
        let row = client
            .query_one("SELECT max(version) FROM workflow_schema_migrations", &[])
            .await?;
        let _ = writeln!(output, "workflow_schema_version {}", row.get::<_, i64>(0));
        driver.abort();
        Ok(output)
    }
}

async fn append_queue_metrics(
    client: &tokio_postgres::Client,
    output: &mut String,
) -> Result<(), StoreError> {
    // At most 1,000 active conversation series per scrape. Report truncation
    // explicitly; terminal conversations never grow the label set.
    for row in client
        .query(
            "SELECT conversation_id, count(*), \
             max(extract(epoch FROM clock_timestamp()-accepted_at))::double precision \
             FROM workflow_turns WHERE execution_state NOT IN ('completed','failed') \
             GROUP BY conversation_id ORDER BY conversation_id LIMIT 1000",
            &[],
        )
        .await?
    {
        let id: String = row.get(0);
        let _ = writeln!(
            *output,
            "workflow_queue_depth{{conversation_id=\"{id}\"}} {}",
            row.get::<_, i64>(1)
        );
        let _ = writeln!(
            *output,
            "workflow_queue_oldest_seconds{{conversation_id=\"{id}\"}} {}",
            row.get::<_, f64>(2)
        );
    }
    let row = client
        .query_one(
            "SELECT greatest(count(DISTINCT conversation_id)-1000,0) \
             FROM workflow_turns WHERE execution_state NOT IN ('completed','failed')",
            &[],
        )
        .await?;
    let _ = writeln!(
        *output,
        "workflow_queue_series_omitted {}",
        row.get::<_, i64>(0)
    );
    Ok(())
}
