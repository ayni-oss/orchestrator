//! Ayni admission expectations in a frozen worker specification.
use crate::managed_runtime::RuntimeExpectation;
pub type WorkerSpec = ayni_orchestrator_runtime::coding::WorkerSpec<RuntimeExpectation>;
