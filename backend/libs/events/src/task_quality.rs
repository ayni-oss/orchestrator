//! Retained quality evidence, separate from coding and publication outcomes.
use crate::task_execution::{TaskCommand, TaskOutcome, digest};
use serde::{Deserialize, Serialize};

pub const MAX_ARTIFACT: usize = 64 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityOutcome {
    pub state: String,
    pub safe_reason: String,
    pub snapshot_digest: String,
    pub validation_input_digest: String,
    pub lock_digest: String,
    pub runtime_image: String,
    pub runtime_platform: String,
    pub runtime_metadata_digest: String,
    pub executor_digest: String,
    pub contract_digest: String,
    pub lock_fingerprint: String,
    pub source_fingerprint: String,
    pub exit_code: Option<i32>,
    pub artifact_digest: String,
    pub artifact: String,
    pub expected_targets: u64,
    pub completed_targets: u64,
}

/// Bind quality to the complete immutable source and accepted coding result.
#[must_use]
pub fn snapshot_digest(command: &TaskCommand, outcome: &TaskOutcome) -> String {
    digest(
        &serde_json::to_vec(&(
            &command.source_pack_digest,
            &command.source_commit,
            &command.lock_digest,
            &outcome.patch,
        ))
        .unwrap_or_default(),
    )
}

impl QualityOutcome {
    #[must_use]
    pub fn validate(&self) -> bool {
        if !self.valid_envelope() {
            return false;
        }
        if matches!(self.state.as_str(), "blocked" | "interrupted") {
            return self.valid_blocked();
        }
        let Ok(artifact) = serde_json::from_str::<serde_json::Value>(&self.artifact) else {
            return false;
        };
        if !valid_aggregate(&artifact)
            || !self.valid_managed_identity(&artifact)
            || !self.valid_completion_context(&artifact)
        {
            return false;
        }
        if self.state == "passed" {
            self.passing(&artifact) && self.safe_reason.is_empty()
        } else {
            !self.passing(&artifact) && self.safe_reason == "quality_failed"
        }
    }
    fn valid_envelope(&self) -> bool {
        matches!(
            self.state.as_str(),
            "passed" | "failed" | "blocked" | "interrupted"
        ) && matches!(
            self.safe_reason.as_str(),
            "" | "quality_failed"
                | "evidence_invalid"
                | "runtime_unavailable"
                | "source_changed"
                | "execution_failed"
                | "fixture_only"
                | "quality_interrupted"
        ) && crate::digest(&self.snapshot_digest)
            && crate::digest(&self.lock_digest)
            && self.runtime_image.len() <= 512
            && self.artifact.len() <= MAX_ARTIFACT
    }
    const fn valid_blocked(&self) -> bool {
        !self.safe_reason.is_empty()
            && self.artifact.is_empty()
            && self.artifact_digest.is_empty()
            && self.expected_targets == 0
            && self.completed_targets == 0
    }
    fn valid_managed_identity(&self, a: &serde_json::Value) -> bool {
        [
            &self.validation_input_digest,
            &self.runtime_metadata_digest,
            &self.executor_digest,
            &self.lock_fingerprint,
            &self.source_fingerprint,
            &self.contract_digest,
        ]
        .into_iter()
        .all(|v| crate::digest(v))
            && self.artifact_digest == digest(self.artifact.as_bytes())
            && matches!(
                self.runtime_platform.as_str(),
                "linux/amd64" | "linux/arm64"
            )
            && a["schema_version"] == "0.5.0"
            && a["invocation"]["command"] == "check"
            && a["execution_mode"] == "managed"
            && a["environment_lock_fingerprint"] == self.lock_fingerprint
            && a["source_fingerprint"] == self.source_fingerprint
            && a["contract_digest"] == self.contract_digest
            && self.valid_runtime_certificate(a)
            && a["tool_versions"].as_array().is_some_and(|v| !v.is_empty())
    }
    fn valid_runtime_certificate(&self, a: &serde_json::Value) -> bool {
        a["prebuilt_runtime"]["metadata_digest"] == self.runtime_metadata_digest
            && a["prebuilt_runtime"]["certificate"]["certificate"]["platform"]
                == self.runtime_platform
            && a["prebuilt_runtime"]["certificate"]["certificate"]["lock_fingerprint"]
                == self.lock_fingerprint
    }
    fn valid_completion_context(&self, a: &serde_json::Value) -> bool {
        let complete = &a["completion"];
        complete["scope"] == "repository"
            && ["path", "package", "file"]
                .into_iter()
                .all(|key| a["invocation"]["scope"][key].is_null())
            && complete["expected_targets"].as_u64() == Some(self.expected_targets)
            && complete["completed_targets"].as_u64() == Some(self.completed_targets)
            && self.completed_targets <= self.expected_targets
            && self.exit_code.is_some()
    }
    fn passing(&self, a: &serde_json::Value) -> bool {
        let complete = &a["completion"];
        self.exit_code == Some(0)
            && complete["state"] == "complete"
            && self.expected_targets > 0
            && self.completed_targets == self.expected_targets
            && complete["detected_targets"].as_u64() == Some(self.expected_targets)
            && complete["skipped_targets"] == 0
            && complete["issues"].as_array().is_some_and(Vec::is_empty)
            && a["aggregate"]["status"] == "pass"
            && a["rows"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty() && rows.iter().all(|row| row["pass"] == true))
    }
}
fn valid_aggregate(a: &serde_json::Value) -> bool {
    let Some(rows) = a["rows"].as_array() else {
        return false;
    };
    let passing = rows.iter().filter(|row| row["pass"] == true).count() as u64;
    let total = rows.len() as u64;
    a["aggregate"]["total_rows"].as_u64() == Some(total)
        && a["aggregate"]["passing_rows"].as_u64() == Some(passing)
        && a["aggregate"]["failing_rows"].as_u64() == Some(total - passing)
        && rows.iter().all(|row| row["pass"].is_boolean())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn evidence() -> QualityOutcome {
        let hash = digest(b"fixture");
        let a = json!({
            "schema_version":"0.5.0", "invocation":{"command":"check","scope":{}},
            "execution_mode":"managed", "environment_lock_fingerprint":hash,
            "source_fingerprint":hash, "contract_digest":hash,
            "prebuilt_runtime":{"metadata_digest":hash,"certificate":{"certificate":{"platform":"linux/arm64","lock_fingerprint":hash}}},
            "tool_versions":[{"tool":"rust","version":"1.94.0"}],
            "completion":{"scope":"repository","state":"complete","expected_targets":1,"detected_targets":1,"completed_targets":1,"skipped_targets":0,"issues":[]},
            "aggregate":{"status":"pass","total_rows":1,"passing_rows":1,"failing_rows":0},
            "rows":[{"pass":true}]
        });
        let artifact = a.to_string();
        QualityOutcome {
            state: "passed".into(),
            snapshot_digest: hash.clone(),
            validation_input_digest: hash.clone(),
            lock_digest: hash.clone(),
            runtime_image: format!("example/runtime@{hash}"),
            runtime_platform: "linux/arm64".into(),
            runtime_metadata_digest: hash.clone(),
            executor_digest: hash.clone(),
            contract_digest: hash.clone(),
            lock_fingerprint: hash.clone(),
            source_fingerprint: hash,
            exit_code: Some(0),
            artifact_digest: digest(artifact.as_bytes()),
            artifact,
            expected_targets: 1,
            completed_targets: 1,
            ..Default::default()
        }
    }
    #[test]
    fn host_focused_incomplete_and_forged_aggregate_cannot_pass()
    -> Result<(), Box<dyn std::error::Error>> {
        let valid = evidence();
        assert!(valid.validate());
        for (pointer, value) in [
            ("/execution_mode", json!("host")),
            ("/schema_version", json!("0.4.0")),
            ("/invocation/command", json!("verify")),
            ("/invocation/scope", json!({"file":"src/lib.rs"})),
            ("/completion/completed_targets", json!(0)),
            ("/completion/detected_targets", json!(0)),
            ("/completion/state", json!("incomplete")),
            ("/aggregate/total_rows", json!(2)),
            ("/aggregate/status", json!("fail")),
            ("/rows/0/pass", json!(false)),
            (
                "/prebuilt_runtime/metadata_digest",
                json!(digest(b"different")),
            ),
            (
                "/environment_lock_fingerprint",
                json!(digest(b"other-lock")),
            ),
            ("/source_fingerprint", json!(digest(b"other-source"))),
            ("/contract_digest", json!(digest(b"other-policy"))),
        ] {
            let mut changed = valid.clone();
            let mut a: Value = serde_json::from_str(&changed.artifact)?;
            let slot = a.pointer_mut(pointer).ok_or("missing pointer")?;
            *slot = value;
            changed.artifact = a.to_string();
            changed.artifact_digest = digest(changed.artifact.as_bytes());
            assert!(!changed.validate(), "accepted {pointer}");
        }
        let mut nonzero = valid;
        nonzero.exit_code = Some(1);
        assert!(!nonzero.validate());
        Ok(())
    }
    #[test]
    fn snapshot_binds_exact_source_and_accepted_patch() {
        let command = TaskCommand {
            source_commit: "commit".into(),
            source_pack_digest: digest(b"pack"),
            lock_digest: digest(b"lock"),
            ..Default::default()
        };
        let mut result = TaskOutcome {
            state: "completed".into(),
            patch: "patch".into(),
            ..Default::default()
        };
        let snapshot = snapshot_digest(&command, &result);
        result.patch.push('x');
        assert_ne!(snapshot_digest(&command, &result), snapshot);
    }
}
