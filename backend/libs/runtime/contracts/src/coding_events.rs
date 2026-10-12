//! Supervisor-owned snapshots, separate from repository-writable recovery state.
use crate::{
    activity::Activity,
    coding::{WorkerProgress, WorkerResult},
    verification::ReportingConfiguration,
};
use ayni_orchestrator_events::v1::CodingExecutionBinding;
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingAdmission {
    pub configuration: ReportingConfiguration,
    pub binding: CodingExecutionBinding,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingSnapshot {
    pub binding: CodingExecutionBinding,
    pub progress: Option<WorkerProgress>,
    pub activity: Vec<Activity>,
    pub usage: serde_json::Value,
    pub publication_intent: Option<serde_json::Value>,
    pub publication_receipt: Option<serde_json::Value>,
    pub result: Option<WorkerResult>,
}
