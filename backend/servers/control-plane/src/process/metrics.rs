use std::{
    collections::BTreeMap,
    fmt::Write as _,
    sync::{Mutex, OnceLock},
};
use tonic::Code;

type GrpcAdmissionCounts = BTreeMap<(&'static str, &'static str, &'static str), u64>;
type GrpcCompletionCounts = BTreeMap<(&'static str, &'static str), u64>;
static GRPC_ADMISSIONS: OnceLock<Mutex<GrpcAdmissionCounts>> = OnceLock::new();
static GRPC_COMPLETIONS: OnceLock<Mutex<GrpcCompletionCounts>> = OnceLock::new();

pub fn record_grpc_admission(rpc: &'static str, decision: &'static str, code: Code) {
    let values = GRPC_ADMISSIONS.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(mut values) = values.lock() {
        *values.entry((rpc, decision, grpc_code(code))).or_default() += 1;
    }
}

pub fn record_grpc_completion(rpc: &'static str, code: Code) {
    let values = GRPC_COMPLETIONS.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(mut values) = values.lock() {
        *values.entry((rpc, grpc_code(code))).or_default() += 1;
    }
}

static RUNTIME_FAILURES: OnceLock<Mutex<BTreeMap<(String, String), u64>>> = OnceLock::new();
pub fn record_runtime_failure(stage: &str, reason: &str) {
    let stage = match stage {
        "build" | "publish" | "discover" | "verify" | "cleanup" | "engine" | "execution"
        | "observation" | "reporting" | "publication" => stage,
        _ => "configuration",
    };
    let reason = match reason {
        "engine_startup_failed"
        | "engine_tls_or_startup_failed"
        | "engine_cleanup_failed"
        | "engine_lost"
        | "engine_recovery_exhausted"
        | "engine_deployment_mode_mismatch"
        | "runtime_version_unsupported" => reason,
        _ => "stage_failed",
    };
    if let Ok(mut values) = RUNTIME_FAILURES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
    {
        *values.entry((stage.into(), reason.into())).or_default() += 1;
    }
}
#[must_use]
pub fn render_clean_metrics() -> String {
    let mut output = format!(
        "# HELP orchestrator_clean_v1_schema_info Clean-v1 durable schema information.\n\
         # TYPE orchestrator_clean_v1_schema_info gauge\n\
         orchestrator_clean_v1_schema_info{{version=\"{}\"}} 1\n",
        crate::adapters::sqlite::SCHEMA_VERSION,
    );
    output.push_str(
        "# HELP orchestrator_grpc_admissions_total gRPC requests admitted or rejected by the access boundary.\n\
         # TYPE orchestrator_grpc_admissions_total counter\n",
    );
    if let Some(values) = GRPC_ADMISSIONS.get().and_then(|values| values.lock().ok()) {
        for ((rpc, decision, code), value) in values.iter() {
            let _ = writeln!(
                output,
                "orchestrator_grpc_admissions_total{{rpc=\"{rpc}\",decision=\"{decision}\",code=\"{code}\"}} {value}"
            );
        }
    }
    output.push_str(
        "# HELP orchestrator_grpc_completions_total Completed gRPC requests by canonical status.\n\
         # TYPE orchestrator_grpc_completions_total counter\n",
    );
    if let Some(values) = GRPC_COMPLETIONS.get().and_then(|values| values.lock().ok()) {
        for ((rpc, code), value) in values.iter() {
            let _ = writeln!(
                output,
                "orchestrator_grpc_completions_total{{rpc=\"{rpc}\",code=\"{code}\"}} {value}"
            );
        }
    }
    output.push_str("# TYPE orchestrator_runtime_preparation_failures_total counter\n");
    if let Some(values) = RUNTIME_FAILURES.get().and_then(|v| v.lock().ok()) {
        for ((stage, reason), count) in values.iter() {
            let _ = writeln!(
                output,
                "orchestrator_runtime_preparation_failures_total{{stage=\"{stage}\",reason=\"{reason}\"}} {count}"
            );
        }
    }
    output.push_str(&render_outbox_metrics());
    output.push_str(&render_consumer_metrics());
    output
}

const fn grpc_code(code: Code) -> &'static str {
    match code {
        Code::Ok => "ok",
        Code::Cancelled => "cancelled",
        Code::Unknown => "unknown",
        Code::InvalidArgument => "invalid_argument",
        Code::DeadlineExceeded => "deadline_exceeded",
        Code::NotFound => "not_found",
        Code::AlreadyExists => "already_exists",
        Code::PermissionDenied => "permission_denied",
        Code::ResourceExhausted => "resource_exhausted",
        Code::FailedPrecondition => "failed_precondition",
        Code::Aborted => "aborted",
        Code::OutOfRange => "out_of_range",
        Code::Unimplemented => "unimplemented",
        Code::Internal => "internal",
        Code::Unavailable => "unavailable",
        Code::DataLoss => "data_loss",
        Code::Unauthenticated => "unauthenticated",
    }
}

static OUTBOX_STATS: OnceLock<Mutex<crate::adapters::sqlite::verification_outbox::OutboxStats>> =
    OnceLock::new();
static OUTBOX_PUBLICATIONS: OnceLock<Mutex<BTreeMap<&'static str, u64>>> = OnceLock::new();
pub fn set_outbox_stats(stats: crate::adapters::sqlite::verification_outbox::OutboxStats) {
    if let Ok(mut current) = OUTBOX_STATS
        .get_or_init(|| {
            Mutex::new(crate::adapters::sqlite::verification_outbox::OutboxStats::default())
        })
        .lock()
    {
        *current = stats;
    }
}
pub fn record_outbox_publication(outcome: &'static str) {
    if let Ok(mut counts) = OUTBOX_PUBLICATIONS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
    {
        *counts.entry(outcome).or_default() += 1;
    }
}
fn render_outbox_metrics() -> String {
    let mut output = String::new();
    if let Some(stats) = OUTBOX_STATS.get().and_then(|v| v.lock().ok()) {
        let now = crate::adapters::sqlite::unix_seconds().unwrap_or(0);
        let age = if stats.pending == 0 {
            0
        } else {
            now.saturating_sub(stats.oldest_at).max(0)
        };
        for (name, value) in [
            ("pending", stats.pending),
            ("pending_bytes", stats.bytes),
            ("oldest_pending_age_seconds", age),
            ("retained_published", stats.retained),
        ] {
            let _ = writeln!(
                output,
                "# TYPE orchestrator_verification_outbox_{name} gauge\norchestrator_verification_outbox_{name} {value}"
            );
        }
    }
    output.push_str("# TYPE orchestrator_verification_outbox_publications_total counter\n");
    if let Some(counts) = OUTBOX_PUBLICATIONS.get().and_then(|v| v.lock().ok()) {
        for (outcome, count) in counts.iter() {
            let _ = writeln!(
                output,
                "orchestrator_verification_outbox_publications_total{{outcome=\"{outcome}\"}} {count}"
            );
        }
    }
    output
}

static CONSUMER_LAG: OnceLock<Mutex<(u64, usize)>> = OnceLock::new();
static CONSUMER_DELIVERIES: OnceLock<Mutex<BTreeMap<&'static str, u64>>> = OnceLock::new();
pub fn set_consumer_lag(pending: u64, unacknowledged: usize) {
    if let Ok(mut lag) = CONSUMER_LAG.get_or_init(|| Mutex::new((0, 0))).lock() {
        *lag = (pending, unacknowledged);
    }
}
pub fn record_consumer_delivery(outcome: &'static str) {
    if let Ok(mut counts) = CONSUMER_DELIVERIES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
    {
        *counts.entry(outcome).or_default() += 1;
    }
}
fn render_consumer_metrics() -> String {
    let mut output = String::new();
    if let Some(lag) = CONSUMER_LAG.get().and_then(|v| v.lock().ok()) {
        for (name, value) in [
            ("pending", lag.0),
            ("unacknowledged", u64::try_from(lag.1).unwrap_or(u64::MAX)),
        ] {
            let _ = writeln!(
                output,
                "# TYPE orchestrator_verification_consumer_{name} gauge\norchestrator_verification_consumer_{name} {value}"
            );
        }
    }
    output.push_str("# TYPE orchestrator_verification_consumer_deliveries_total counter\n");
    if let Some(counts) = CONSUMER_DELIVERIES.get().and_then(|v| v.lock().ok()) {
        for (outcome, count) in &*counts {
            let _ = writeln!(
                output,
                "orchestrator_verification_consumer_deliveries_total{{outcome=\"{outcome}\"}} {count}"
            );
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_bounded_transport_metrics() {
        record_grpc_admission("GetRuntimeProfile", "allow", Code::Ok);
        record_grpc_completion("GetRuntimeProfile", Code::NotFound);
        let rendered = render_clean_metrics();
        assert!(rendered.contains(&format!(
            "orchestrator_clean_v1_schema_info{{version=\"{}\"}} 1",
            crate::adapters::sqlite::SCHEMA_VERSION
        )));
        assert!(rendered.contains("rpc=\"GetRuntimeProfile\""));
        assert!(!rendered.contains("repository_id"));
    }
}
