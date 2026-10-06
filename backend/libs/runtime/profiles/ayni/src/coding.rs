//! The Ayni Managed specialization of shared coding inputs.

use crate::managed_runtime::RuntimeExpectation;

/// Coding in Ayni Managed always carries frozen Ayni admission expectations.
pub type CodingRequest = ayni_orchestrator_runtime::coding::CodingRequest<RuntimeExpectation>;

#[cfg(test)]
mod tests {
    use super::CodingRequest;
    use crate::runtime_execution::ExecutionRequest;
    use serde_json::{Value, json};

    fn mounted_request() -> Value {
        json!({
            "execution": {
                "repository_url": "https://github.com/team/repository.git",
                "revision": "a".repeat(40),
                "expected": {
                    "lock_fingerprint": format!("sha256:{}", "b".repeat(64)),
                    "platform": "linux/amd64",
                    "ayni_version": crate::ayni_release::CLI_VERSION
                }
            },
            "request": "Update the documentation",
            "acceptance_criteria": ["Examples describe the current behavior"],
            "constraints": {"protected_paths": ["policy"], "allow_test_edits": false},
            "model": "configured-model",
            "max_turns": 24
        })
    }

    #[test]
    fn managed_worker_requests_preserve_the_mounted_json_contract() -> Result<(), serde_json::Error>
    {
        let expected = mounted_request();
        let coding: CodingRequest = serde_json::from_value(expected.clone())?;
        assert_eq!(serde_json::to_value(coding)?, expected);
        let checkout: ExecutionRequest = serde_json::from_value(expected["execution"].clone())?;
        assert_eq!(serde_json::to_value(checkout)?, expected["execution"]);
        Ok(())
    }

    #[test]
    fn managed_worker_requests_require_every_ayni_expectation() {
        for field in ["lock_fingerprint", "platform", "ayni_version"] {
            let mut request = mounted_request();
            request["execution"]["expected"]
                .as_object_mut()
                .into_iter()
                .for_each(|expected| {
                    expected.remove(field);
                });
            assert!(serde_json::from_value::<CodingRequest>(request).is_err());
        }
        let mut custom = mounted_request();
        custom["execution"]["expected"] = json!({
            "platform": "linux/amd64", "launcher_protocol": "container-v1"
        });
        assert!(serde_json::from_value::<CodingRequest>(custom).is_err());
    }
}
