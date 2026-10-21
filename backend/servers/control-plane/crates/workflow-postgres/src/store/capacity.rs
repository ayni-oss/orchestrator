//! Serialized application reservations cover obligations before their events exist.
use super::StoreError;
use tokio_postgres::Transaction;

const CAPACITY_LOCK: i64 = 4_702_199_126_064_923;
const COMMAND_BYTES: i64 = 264 * 1024;
const DECISION_BYTES: i64 = 24 * 1024;

pub(super) async fn reserve_admission(tx: &Transaction<'_>) -> Result<(), StoreError> {
    tx.query_one("SELECT pg_advisory_xact_lock($1)", &[&CAPACITY_LOCK])
        .await?;
    let commands = tx
        .query_one(
            "SELECT count(*), COALESCE(sum(CASE WHEN d.exact_command_bytes IS NULL THEN $1::bigint \
         ELSE octet_length(d.exact_command_bytes)+8192 END),0)::bigint \
         FROM workflow_turns t JOIN workflow_dispatch_intents d ON d.turn_id=t.id \
         WHERE t.dispatch_state <> 'transferred'",
            &[&COMMAND_BYTES],
        )
        .await?;
    let occupancy = occupancy(tx, "command").await?;
    if commands.get::<_, i64>(0) + occupancy.0 >= 25_000
        || commands.get::<_, i64>(1) + COMMAND_BYTES + occupancy.1 > 256 * 1024 * 1024
    {
        return Err(StoreError::Capacity("command_lane_reservation"));
    }
    check_decisions(tx, 2).await
}

pub(super) async fn reserve_decision(
    tx: &Transaction<'_>,
    rejected: bool,
) -> Result<(), StoreError> {
    tx.query_one("SELECT pg_advisory_xact_lock($1)", &[&CAPACITY_LOCK])
        .await?;
    check_decisions(tx, i64::from(rejected)).await
}

async fn check_decisions(tx: &Transaction<'_>, additional: i64) -> Result<(), StoreError> {
    // Each Turn owns two decision obligations. Accepted, consumer-ACKed decisions
    // release one; rejected reports add independent obligations until their ACK.
    let row = tx
        .query_one(
            "SELECT (SELECT count(*)*2 FROM workflow_turns) \
         - (SELECT count(*) FROM workflow_report_receipts r JOIN workflow_outbox o \
            ON o.id=r.decision_event_id WHERE r.decision='accepted' AND o.state='transferred') \
         + (SELECT count(*) FROM workflow_report_receipts r JOIN workflow_outbox o \
            ON o.id=r.decision_event_id WHERE r.decision='rejected' AND o.state<>'transferred') \
         + (SELECT count(*) FROM workflow_transport_rejections WHERE state<>'transferred')",
            &[],
        )
        .await?;
    let obligations = row
        .get::<_, i64>(0)
        .checked_add(additional)
        .ok_or(StoreError::CorruptState)?;
    let occupancy = occupancy(tx, "decision").await?;
    if obligations + occupancy.0 > 50_000
        || obligations.saturating_mul(DECISION_BYTES) + occupancy.1 > 64 * 1024 * 1024
    {
        return Err(StoreError::Capacity("decision_lane_reservation"));
    }
    Ok(())
}

async fn occupancy(tx: &Transaction<'_>, lane: &str) -> Result<(i64, i64), StoreError> {
    let row = tx
        .query_opt(
            "SELECT retained_messages,retained_bytes FROM workflow_broker_capacity WHERE lane=$1",
            &[&lane],
        )
        .await?;
    Ok(row.map_or((0, 0), |row| (row.get(0), row.get(1))))
}
