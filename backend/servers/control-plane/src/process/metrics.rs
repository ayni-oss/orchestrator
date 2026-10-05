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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_bounded_transport_metrics() {
        record_grpc_admission("GetRuntimeProfile", "allow", Code::Ok);
        record_grpc_completion("GetRuntimeProfile", Code::NotFound);
        let rendered = render_clean_metrics();
        assert!(rendered.contains("orchestrator_clean_v1_schema_info{version=\"4\"} 1"));
        assert!(rendered.contains("rpc=\"GetRuntimeProfile\""));
        assert!(!rendered.contains("repository_id"));
    }
}
