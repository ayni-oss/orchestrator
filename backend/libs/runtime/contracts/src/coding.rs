use crate::runtime_execution::ExecutionRequest;
use ayni_orchestrator_domain::ChangeConstraints;
use serde::{Deserialize, Serialize};

/// Bounded Ayni result captured by the coding worker for one candidate.
///
/// This is task validation produced in the coding runtime, not independent
/// assessment evidence.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationOutcome {
    Passed,
    Failed,
    Unavailable,
    Missing,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValidationResult {
    pub outcome: ValidationOutcome,
    pub signals_digest: Option<String>,
    pub completed_targets: u32,
    pub skipped_targets: u32,
    pub warning_offenders: u32,
    pub failing_offenders: u32,
    pub rows: Vec<ValidationRow>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValidationRow {
    pub kind: String,
    pub passed: bool,
}

/// Shared coding inputs with the selected profile's admission expectations.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodingRequest<Profile> {
    pub execution: ExecutionRequest<Profile>,
    pub request: String,
    pub acceptance_criteria: Vec<String>,
    pub constraints: ChangeConstraints,
    pub model: String,
    /// Operator-selected cap on model and tool turns for this mounted attempt.
    pub max_turns: u16,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodingReport {
    pub commit: Option<String>,
    pub bundle_digest: Option<String>,
    pub changed_paths: Vec<String>,
    pub requested_input: Option<String>,
    pub validation: ValidationResult,
}

#[cfg(test)]
mod tests {
    use super::CodingRequest;
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    /// A container admission contract can be expressed without Ayni metadata.
    /// This test does not enable a Custom Runtime worker or admission path.
    #[derive(Clone, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct ContainerExpectation {
        platform: String,
        launcher_protocol: String,
    }

    #[test]
    fn coding_inputs_support_an_explicit_container_profile() -> Result<(), serde_json::Error> {
        let mounted_request = json!({
            "execution": {
                "repository_url": "https://github.com/team/repository.git",
                "revision": "a".repeat(40),
                "expected": {
                    "platform": "linux/amd64",
                    "launcher_protocol": "container-v1"
                }
            },
            "request": "Update the documentation",
            "acceptance_criteria": ["Examples describe the current behavior"],
            "constraints": {"protected_paths": ["policy"], "allow_test_edits": false},
            "model": "configured-model",
            "max_turns": 24
        });
        let request: CodingRequest<ContainerExpectation> =
            serde_json::from_value(mounted_request.clone())?;
        assert_eq!(request.execution.expected.launcher_protocol, "container-v1");
        assert_eq!(serde_json::to_value(request)?, mounted_request);

        // The profile contract remains explicit: a request cannot omit it or
        // smuggle a different profile's fields into this specialization.
        let mut missing = mounted_request.clone();
        missing["execution"]["expected"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<CodingRequest<ContainerExpectation>>(missing).is_err());
        let mut wrong_profile = mounted_request;
        wrong_profile["execution"]["expected"]["lock_fingerprint"] = json!("unexpected");
        assert!(
            serde_json::from_value::<CodingRequest<ContainerExpectation>>(wrong_profile).is_err()
        );
        Ok(())
    }
}
