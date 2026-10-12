//! Supervisor-owned retained runtime preparation results.
use super::{image_discovery::ImageDiscovery, verification::ReportingConfiguration};
use ayni_orchestrator_events::v1::{RuntimeOperationBinding, RuntimeOperationKind};
use serde::{Deserialize, Serialize};

pub const RECORD_FILE: &str = "runtime-operation.json";
pub const EVENT_FILE: &str = "runtime-operation-event.json";
pub const RECEIPT_FILE: &str = "runtime-operation-receipt.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationAdmission {
    pub configuration: ReportingConfiguration,
    pub binding: RuntimeOperationBinding,
    pub kind: RuntimeOperationKind,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRecord {
    pub contract: String,
    pub binding: RuntimeOperationBinding,
    pub outcome: OperationOutcome,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationOutcome {
    Discovery {
        result: ImageDiscovery,
    },
    Trust {
        revision: String,
        policy: String,
        lock: String,
    },
    Built {
        candidate_image_id: String,
    },
    Published,
    Failed {
        reason: String,
    },
}
impl OperationOutcome {
    pub fn matches(&self, kind: RuntimeOperationKind) -> bool {
        match self {
            Self::Failed { reason } => {
                !reason.is_empty()
                    && reason.len() <= 128
                    && reason.bytes().all(|v| v.is_ascii_lowercase() || v == b'_')
            }
            Self::Discovery { result } => {
                kind == RuntimeOperationKind::Discovery
                    && !matches!(result, ImageDiscovery::Pending)
            }
            Self::Trust { .. } => kind == RuntimeOperationKind::Trust,
            Self::Built { candidate_image_id } => {
                kind == RuntimeOperationKind::Build
                    && ayni_orchestrator_domain::digest::digest(candidate_image_id)
            }
            Self::Published => kind == RuntimeOperationKind::Publication,
        }
    }
}
