//! Ayni Managed execution inputs and advisory runtime verification output.

use crate::managed_runtime::{RuntimeExpectation, VerifiedRuntime};
use serde::{Deserialize, Serialize};

/// Frozen managed request. Ayni expectations are mandatory for this profile.
pub type ExecutionRequest =
    ayni_orchestrator_runtime::runtime_execution::ExecutionRequest<RuntimeExpectation>;

/// Result of a runtime verification invocation. Quality remains advisory.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReport {
    pub revision: String,
    pub lock_digest: String,
    pub runtime: VerifiedRuntime,
    pub check_exit_code: i32,
}
