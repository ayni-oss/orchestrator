use crate::runtime_execution::ExecutionRequest;
use ayni_orchestrator_domain::ChangeConstraints;
use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodingRequest {
    pub execution: ExecutionRequest,
    pub request: String,
    pub acceptance_criteria: Vec<String>,
    pub constraints: ChangeConstraints,
    pub model: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodingReport {
    pub commit: Option<String>,
    pub bundle_digest: Option<String>,
    pub changed_paths: Vec<String>,
    pub requested_input: Option<String>,
    pub advisory: String,
}
