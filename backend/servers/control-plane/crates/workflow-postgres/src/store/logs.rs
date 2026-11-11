//! Bounded raw debug storage, independent of the diagnostic outbox.
use super::{StoreError, WorkflowStore};
use ayni_orchestrator_telemetry::v1::{
    LogEntry, LogSource, ReadLogsRequest, ReadLogsResponse, TelemetryRecord,
};
use sha2::{Digest as _, Sha256};
use tokio::sync::Semaphore;

static CONNECTIONS: Semaphore = Semaphore::const_new(4);
const LOCK: i64 = 4_702_199_126_064_922;

struct PreparedLog<'a> {
    source: &'a LogSource,
    sequence: i64,
    body: String,
    metadata: String,
    digest: String,
}

fn prepare_log(record: &TelemetryRecord) -> Result<PreparedLog<'_>, StoreError> {
    let source = record
        .source
        .as_ref()
        .ok_or(StoreError::InvalidInput("log source"))?;
    if source.id.is_empty()
        || source.id.len() > 256
        || record.sequence == 0
        || record.data.len() > 8192
    {
        return Err(StoreError::InvalidInput("log bounds"));
    }
    let sequence =
        i64::try_from(record.sequence).map_err(|_| StoreError::InvalidInput("log sequence"))?;
    let body = serde_json::to_string(record).map_err(|_| StoreError::CorruptState)?;
    let metadata = serde_json::to_string(source).map_err(|_| StoreError::CorruptState)?;
    if body.len() > 65536 || metadata.len() > 8192 {
        return Err(StoreError::InvalidInput("log bounds"));
    }
    let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
    Ok(PreparedLog {
        source,
        sequence,
        body,
        metadata,
        digest,
    })
}

impl WorkflowStore {
    /// Return only cancellation authority, never permission to start or retry work.
    /// # Errors
    /// Return unavailable state; existing deadlines and supervisor fencing remain authoritative.
    pub async fn log_cancellation(&self, source: &LogSource) -> Result<bool, StoreError> {
        let _permit = CONNECTIONS
            .try_acquire()
            .map_err(|_| StoreError::Capacity("log connections"))?;
        let (client, _driver) = crate::connect(&self.database_url).await?;
        let generation =
            i64::try_from(source.generation).map_err(|_| StoreError::InvalidInput("generation"))?;
        let row = if source.component == "task-worker" {
            client.query_opt("SELECT state IN ('cancelling','cancelled','interrupted') FROM workflow_task_runs WHERE attempt_id=$1 AND generation=$2", &[&source.execution_id,&generation]).await?
        } else if source.component == "runtime-worker" {
            client.query_opt("SELECT state IN ('cancelling','cancelled','superseded') OR generation>$2 FROM workflow_runtime_preparations WHERE id=$1", &[&source.execution_id,&generation]).await?
        } else {
            None
        };
        Ok(row.is_some_and(|r| r.get(0)))
    }

    /// Mirror retained diagnostics without changing their existing outbox/retention.
    /// # Errors
    /// Retry from durable source rows after log-storage outages.
    pub async fn mirror_execution_diagnostics(
        &self,
        deployment: &str,
        maximum: i64,
    ) -> Result<(), StoreError> {
        let rows = {
            let _permit = CONNECTIONS
                .try_acquire()
                .map_err(|_| StoreError::Capacity("log connections"))?;
            let (client, _driver) = crate::connect(&self.database_url).await?;
            client.query("SELECT d.cursor,d.diagnostic,(extract(epoch FROM d.observed_at)*1000)::bigint FROM workflow_execution_diagnostics d WHERE NOT EXISTS (SELECT 1 FROM workflow_log_sources s WHERE s.id='evidence:'||d.cursor::text) ORDER BY d.cursor LIMIT 32",&[]).await?
        };
        for row in rows {
            let id: i64 = row.get(0);
            let bytes: &str = row.get(1);
            let d: ayni_orchestrator_events::execution_diagnostics::Diagnostic =
                serde_json::from_str(bytes).map_err(|_| StoreError::CorruptState)?;
            let record = TelemetryRecord {
                source: Some(LogSource {
                    id: format!("evidence:{id}"),
                    deployment: deployment.into(),
                    component: format!("{}-diagnostics", d.kind.name()),
                    execution_id: d.execution_id,
                    pod_uid: d.pod_uid,
                    generation: d.generation,
                    stage: d.stage,
                    provenance: "retained_evidence".into(),
                    capture_status: "retained".into(),
                    ..Default::default()
                }),
                sequence: 1,
                event: "retained_diagnostic".into(),
                stream: "diagnostic".into(),
                data: bytes.as_bytes().to_vec(),
                observed_at_unix_ms: row.get(2),
                ..Default::default()
            };
            self.append_log_bounded(&record, maximum).await?;
        }
        Ok(())
    }

    /// Shared durable gauges. Aggregate with max across control-plane replicas.
    /// # Errors
    /// Return unavailable projection, independently of workflow writes.
    pub async fn log_metrics(&self) -> Result<String, StoreError> {
        use std::fmt::Write as _;
        let _permit = CONNECTIONS
            .try_acquire()
            .map_err(|_| StoreError::Capacity("log connections"))?;
        let (client, _driver) = crate::connect(&self.database_url).await?;
        let mut output = String::new();
        let row=client.query_one("SELECT count(*),coalesce(sum(payload_bytes),0)::bigint,count(*) FILTER (WHERE record::jsonb->>'event'='capture_gap'),coalesce(sum((record::jsonb->>'lostBytes')::bigint),0)::bigint FROM workflow_log_records",&[]).await?;
        for (column, name) in ["records", "payload_bytes", "gap_records", "lost_bytes"]
            .into_iter()
            .enumerate()
        {
            let _ = writeln!(
                output,
                "workflow_capture_retained_{name} {}",
                row.get::<_, i64>(column)
            );
        }
        let row=client.query_one("SELECT count(*) FROM workflow_log_sources WHERE source::jsonb->>'component'='log-collector' AND observed_at>clock_timestamp()-interval '60 seconds'",&[]).await?;
        let _ = writeln!(
            output,
            "workflow_collector_recent_sources {}",
            row.get::<_, i64>(0)
        );
        let row=client.query_one("SELECT coalesce(sum((r.record::jsonb->>'pendingBytes')::bigint),0)::bigint,coalesce(max(greatest(0,extract(epoch FROM r.observed_at)*1000-(r.record::jsonb->>'observedAtUnixMs')::bigint)/1000),0)::double precision FROM workflow_log_sources s JOIN workflow_log_records r ON r.source_id=s.id AND r.sequence=s.sequence",&[]).await?;
        let _ = writeln!(
            output,
            "workflow_capture_reported_pending_bytes {}",
            row.get::<_, i64>(0)
        );
        let _ = writeln!(
            output,
            "workflow_capture_delivery_lag_seconds {}",
            row.get::<_, f64>(1)
        );
        let row=client.query_one("SELECT (SELECT count(*) FROM workflow_runtime_preparations WHERE state IN ('pending','running'))+(SELECT count(*) FROM workflow_task_runs WHERE state IN ('claimed','reporting')), (SELECT count(*) FROM workflow_runtime_preparations WHERE state IN ('stopping','cleanup_pending'))",&[]).await?;
        let _ = writeln!(
            output,
            "workflow_capture_active_executions {}\nworkflow_capture_cleanup_waits {}",
            row.get::<_, i64>(0),
            row.get::<_, i64>(1)
        );
        for row in client.query("SELECT record::jsonb->>'event',count(*),coalesce(sum((record::jsonb->>'durationMs')::bigint),0)::bigint FROM workflow_log_records WHERE record::jsonb->>'event' IN ('succeeded','failed','timed_out','cancelled','signalled','spawn_failed','drain_incomplete') GROUP BY 1",&[]).await? {
            let outcome:String=row.get(0);
            let _=writeln!(output,"workflow_capture_retained_outcomes{{outcome=\"{outcome}\"}} {}\nworkflow_capture_retained_duration_milliseconds{{outcome=\"{outcome}\"}} {}",row.get::<_,i64>(1),row.get::<_,i64>(2));
        }
        Ok(output)
    }

    /// # Errors
    /// Reject a task producer outside the retained pod/generation binding.
    pub async fn validate_log_task_binding(&self, source: &LogSource) -> Result<(), StoreError> {
        let _permit = CONNECTIONS
            .try_acquire()
            .map_err(|_| StoreError::Capacity("log connections"))?;
        let (client, _driver) = crate::connect(&self.database_url).await?;
        let generation =
            i64::try_from(source.generation).map_err(|_| StoreError::InvalidInput("generation"))?;
        let known=client.query_opt("SELECT 1 FROM workflow_task_runs WHERE attempt_id=$1 AND generation=$2 AND pod_uid=$3",&[&source.execution_id,&generation,&source.pod_uid]).await?;
        known.map(|_| ()).ok_or(StoreError::NotFound)
    }

    /// # Errors
    /// Bind publication observations to an assigned pod or its registered replay source.
    pub async fn validate_log_publication_binding(
        &self,
        source: &LogSource,
    ) -> Result<(), StoreError> {
        let _permit = CONNECTIONS
            .try_acquire()
            .map_err(|_| StoreError::Capacity("log connections"))?;
        let (client, _driver) = crate::connect(&self.database_url).await?;
        let generation =
            i64::try_from(source.generation).map_err(|_| StoreError::InvalidInput("generation"))?;
        let known = client.query_opt("SELECT 1 FROM workflow_task_publications WHERE publication_id=$1 AND generation=$2 AND pod_uid=$3 UNION ALL SELECT 1 FROM workflow_log_sources WHERE id=$4 AND source::jsonb->>'executionId'=$1 AND (source::jsonb->>'generation')::bigint=$2 AND source::jsonb->>'podUid'=$3 LIMIT 1", &[&source.execution_id, &generation, &source.pod_uid, &source.id]).await?;
        known.map(|_| ()).ok_or(StoreError::NotFound)
    }

    /// Validate retained identity without granting execution authority.
    /// # Errors
    /// Reject unknown or mismatched execution/pod bindings.
    pub async fn validate_log_runtime_binding(
        &self,
        binding: &ayni_orchestrator_events::runtime_preparation::Binding,
    ) -> Result<(), StoreError> {
        let _permit = CONNECTIONS
            .try_acquire()
            .map_err(|_| StoreError::Capacity("log connections"))?;
        let (client, _driver) = crate::connect(&self.database_url).await?;
        let generation = i64::try_from(binding.generation)
            .map_err(|_| StoreError::InvalidInput("generation"))?;
        let known=client.query_opt("SELECT 1 FROM workflow_runtime_preparation_attempts WHERE operation_id=$1 AND stage=$2 AND generation=$3 AND pod_uid=$4 AND command_digest=$5", &[&binding.operation_id,&binding.stage,&generation,&binding.pod_uid,&binding.command_digest]).await?;
        known.map(|_| ()).ok_or(StoreError::NotFound)
    }

    /// # Errors
    /// Reject sequence gaps and conflicting duplicates; ACK only after commit.
    pub async fn append_log(&self, record: &TelemetryRecord) -> Result<(), StoreError> {
        self.append_log_bounded(record, 1024 * 1024 * 1024).await
    }

    /// # Errors
    /// Reject invalid records or storage failure. Enforce the raw payload quota atomically.
    pub async fn append_log_bounded(
        &self,
        record: &TelemetryRecord,
        maximum: i64,
    ) -> Result<(), StoreError> {
        let _permit = CONNECTIONS
            .try_acquire()
            .map_err(|_| StoreError::Capacity("log connections"))?;
        let PreparedLog {
            source,
            sequence: seq,
            body,
            metadata,
            digest,
        } = prepare_log(record)?;
        let (mut client, _driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        tx.batch_execute("SET LOCAL statement_timeout='3s'").await?;
        // Allocate cursors only under the same commit lock across all producers.
        tx.query_one("SELECT pg_advisory_xact_lock($1)", &[&LOCK])
            .await?;
        tx.execute("INSERT INTO workflow_log_sources(id,source,sequence,digest) VALUES($1,$2,0,'') ON CONFLICT DO NOTHING",&[&source.id,&metadata]).await?;
        let head = tx
            .query_one(
                "SELECT sequence,digest,receipts,source FROM workflow_log_sources WHERE id=$1",
                &[&source.id],
            )
            .await?;
        validate_source_identity(source, head.get(3))?;
        let last: i64 = head.get(0);
        let mut receipts: std::collections::BTreeMap<u64, String> =
            serde_json::from_str(head.get::<_, &str>(2)).map_err(|_| StoreError::CorruptState)?;
        if seq <= last {
            let stored = tx
                .query_opt(
                    "SELECT digest FROM workflow_log_records WHERE source_id=$1 AND sequence=$2",
                    &[&source.id, &seq],
                )
                .await?;
            let previous = stored
                .map(|r| r.get::<_, String>(0))
                .or_else(|| receipts.get(&record.sequence).cloned())
                .or_else(|| (seq == last).then(|| head.get(1)));
            if previous.as_deref() != Some(&digest) {
                return Err(StoreError::IdempotencyConflict);
            }
        } else {
            if seq != last + 1 {
                return Err(StoreError::InvalidInput("log sequence gap"));
            }
            tx.execute("INSERT INTO workflow_log_records(source_id,sequence,digest,record,payload_bytes) VALUES($1,$2,$3,$4,$5)",&[&source.id,&seq,&digest,&body,&i64::try_from(record.data.len()).map_err(|_|StoreError::InvalidInput("log bytes"))?]).await?;
            enforce_quota(&tx, record.data.len(), maximum).await?;
            // Producers keep at most 32 unacknowledged records in flight. Retain
            // 64 digest receipts independently of raw expiry, including lost ACKs.
            receipts.insert(record.sequence, digest.clone());
            while receipts.len() > 64 {
                receipts.pop_first();
            }
            let receipts =
                serde_json::to_string(&receipts).map_err(|_| StoreError::CorruptState)?;
            tx.execute("UPDATE workflow_log_sources SET source=CASE WHEN coalesce(($2::text::jsonb->>'observedAtUnixMs')::bigint,0)>=coalesce((source::jsonb->>'observedAtUnixMs')::bigint,0) THEN $2 ELSE source END,sequence=$3,digest=$4,observed_at=clock_timestamp(),receipts=$5 WHERE id=$1",&[&source.id,&metadata,&seq,&digest,&receipts]).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// # Errors
    /// Return bounded source inventory, including deleted pods.
    pub async fn log_sources(
        &self,
        execution: &str,
        after: &str,
    ) -> Result<Vec<LogSource>, StoreError> {
        let _permit = CONNECTIONS
            .try_acquire()
            .map_err(|_| StoreError::Capacity("log connections"))?;
        let (client, _driver) = crate::connect(&self.database_url).await?;
        let rows=client.query("SELECT source FROM workflow_log_sources WHERE id>$1 AND ($2='' OR source::jsonb->>'executionId'=$2) ORDER BY id LIMIT 100",&[&after,&execution]).await?;
        rows.iter()
            .map(|r| {
                serde_json::from_str(r.get::<_, &str>(0)).map_err(|_| StoreError::CorruptState)
            })
            .collect()
    }

    /// # Errors
    /// Return a bounded scan and cursor, even when filters match no records.
    pub async fn read_logs(&self, query: &ReadLogsRequest) -> Result<ReadLogsResponse, StoreError> {
        if query.after < 0 || query.text.len() > 1024 {
            return Err(StoreError::InvalidInput("log query"));
        }
        let _permit = CONNECTIONS
            .try_acquire()
            .map_err(|_| StoreError::Capacity("log connections"))?;
        let (client, _driver) = crate::connect(&self.database_url).await?;
        let rows=client.query("SELECT cursor,record FROM workflow_log_records WHERE cursor>$1 ORDER BY cursor LIMIT 100",&[&query.after]).await?;
        let expired: i64 = client
            .query_one("SELECT expired_through FROM workflow_log_retention", &[])
            .await?
            .get(0);
        let high_water_cursor: i64 = client
            .query_one(
                "SELECT coalesce(max(cursor),0) FROM workflow_log_records",
                &[],
            )
            .await?
            .get(0);
        let mut response = ReadLogsResponse {
            high_water_cursor,
            next_cursor: query.after.max(expired),
            expired_through_cursor: expired,
            entries: Vec::new(),
        };
        for row in rows {
            let cursor: i64 = row.get(0);
            response.next_cursor = response.next_cursor.max(cursor);
            let record: TelemetryRecord = serde_json::from_str(row.get::<_, &str>(1))
                .map_err(|_| StoreError::CorruptState)?;
            if matches(query, &record) {
                response.entries.push(LogEntry {
                    cursor,
                    record: Some(record),
                });
            }
        }
        Ok(response)
    }

    /// # Errors
    /// Expire bounded batches independently of workflow retention.
    pub async fn cleanup_logs(&self, seconds: i64, maximum: i64) -> Result<(), StoreError> {
        let _permit = CONNECTIONS
            .try_acquire()
            .map_err(|_| StoreError::Capacity("log connections"))?;
        let (mut client, _driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        tx.query_one("SELECT pg_advisory_xact_lock($1)", &[&LOCK])
            .await?;
        let rows=tx.query("SELECT cursor FROM (SELECT cursor,observed_at,sum(payload_bytes) OVER (ORDER BY cursor DESC) AS newer_bytes FROM workflow_log_records) records WHERE observed_at<clock_timestamp()-($1::bigint * interval '1 second') OR newer_bytes>$2::bigint ORDER BY cursor LIMIT 1000",&[&seconds,&maximum]).await?;
        if let Some(last) = rows.last() {
            let through: i64 = last.get(0);
            expire_logs(&tx, through).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

async fn expire_logs(tx: &tokio_postgres::Transaction<'_>, through: i64) -> Result<(), StoreError> {
    let removed:i64=tx.query_one("WITH expired AS (DELETE FROM workflow_log_records WHERE cursor<=$1 RETURNING payload_bytes) SELECT coalesce(sum(payload_bytes),0)::bigint FROM expired",&[&through]).await?.get(0);
    tx.execute("UPDATE workflow_log_retention SET expired_through=greatest(expired_through,$1),retained_bytes=greatest(0,retained_bytes-$2)",&[&through,&removed]).await?;
    Ok(())
}

fn matches(q: &ReadLogsRequest, r: &TelemetryRecord) -> bool {
    let Some(s) = &r.source else {
        return false;
    };
    [
        (&q.source_id, &s.id),
        (&q.component, &s.component),
        (&q.execution_id, &s.execution_id),
        (&q.namespace, &s.namespace),
        (&q.pod_uid, &s.pod_uid),
        (&q.container, &s.container),
        (&q.stream, &r.stream),
    ]
    .into_iter()
    .all(|(filter, value)| filter.is_empty() || filter == value)
        && q.restart.is_none_or(|v| v == s.restart)
        && (q.from_unix_ms == 0 || r.observed_at_unix_ms >= q.from_unix_ms)
        && (q.until_unix_ms == 0 || r.observed_at_unix_ms <= q.until_unix_ms)
        && (q.text.is_empty() || String::from_utf8_lossy(&r.data).contains(&q.text))
}

fn validate_source_identity(source: &LogSource, stored: &str) -> Result<(), StoreError> {
    let old: LogSource = serde_json::from_str(stored).map_err(|_| StoreError::CorruptState)?;
    let identity = |s: &LogSource| {
        (
            s.execution_id.clone(),
            s.generation,
            s.pod_uid.clone(),
            s.container.clone(),
            s.restart,
            s.provenance.clone(),
        )
    };
    if identity(source) != identity(&old) {
        return Err(StoreError::IdempotencyConflict);
    }
    Ok(())
}

async fn enforce_quota(
    tx: &tokio_postgres::Transaction<'_>,
    bytes: usize,
    maximum: i64,
) -> Result<(), StoreError> {
    let payload = i64::try_from(bytes).map_err(|_| StoreError::InvalidInput("payload"))?;
    let retained:i64=tx.query_one("UPDATE workflow_log_retention SET retained_bytes=retained_bytes+$1 RETURNING retained_bytes",&[&payload]).await?.get(0);
    if retained > maximum.max(1) {
        let overflow = retained - maximum.max(1);
        let through:i64=tx.query_one("SELECT cursor FROM (SELECT cursor,sum(payload_bytes) OVER (ORDER BY cursor) AS removed FROM workflow_log_records) r WHERE removed>=$1::bigint ORDER BY cursor LIMIT 1",&[&overflow]).await?.get(0);
        expire_logs(tx, through).await?;
    }
    Ok(())
}
