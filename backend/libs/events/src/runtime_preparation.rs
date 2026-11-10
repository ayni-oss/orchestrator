//! Frozen runtime preparation contracts. Evidence is admitted only by the fenced authority.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Immutable operator-supplied tooling image identity. Registry destinations for
/// prepared artifacts have a separate, stricter authority contract.
#[must_use]
pub fn valid_tool_image_reference(value: &str) -> bool {
    let Some((repository, hash)) = value.split_once('@') else {
        return false;
    };
    let Some(hex) = hash.strip_prefix("sha256:") else {
        return false;
    };
    !repository.is_empty()
        && repository.len() <= 512
        && !repository.contains(['@', ':'])
        && repository.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        })
        && hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub const QUALIFIED_PROFILE_ID: &str = "rootless-podman-vfs-userns-v1";
pub const DEVELOPMENT_PROFILE_ID: &str = "rootless-podman-vfs-userns-local-development-v1";

/// Development is an explicit operator opt-in, never a claim of topology qualification.
#[must_use]
pub fn admitted_profile_identity(id: &str, qualified: bool) -> bool {
    matches!(
        (id, qualified),
        (QUALIFIED_PROFILE_ID, true) | (DEVELOPMENT_PROFILE_ID, false)
    )
}

pub const STAGES: &[&str] = &[
    "acquire_source",
    "discover",
    "pull_existing",
    "build",
    "freeze_candidate",
    "certify",
    "finalize",
    "publish",
    "pull_final",
    "verify_content",
    "verify_compatibility",
    "cleanup",
];
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrozenFile {
    pub path: String,
    pub digest: String,
    pub content: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeProfile {
    pub profile_id: String,
    pub qualified: bool,
    pub platform: String,
    pub worker_image: String,
    pub executor_image: String,
    pub verifier_image: String,
    pub trusted_cli_sha256: String,
    pub max_cpu_millis: u32,
    pub max_memory_mib: u32,
    pub max_storage_mib: u32,
    pub max_seconds: u32,
    pub max_builds: u32,
}
/// Trusted acquisition inputs; identity and credential revision references are filled by admission.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FreezeInputs {
    pub expected_signer_revision: u64,
    pub source_tree: String,
    pub policy: String,
    pub lock: String,
    pub sources: Vec<FrozenFile>,
    pub profile_json: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrozenInputs {
    pub repository_id: String,
    pub inspection_id: String,
    pub source_commit: String,
    pub source_tree: String,
    pub source_connection_id: String,
    pub source_connection_revision: u64,
    pub source_credential_revision: String,
    pub installation_id: u64,
    pub provider_repository_id: u64,
    pub repository_full_name: String,
    pub registry_connection_id: String,
    pub registry_connection_revision: u64,
    pub registry_credential_revision: String,
    pub registry_destination: String,
    pub discovery_tag: String,
    pub reusable_image: String,
    pub publication_tag: String,
    pub signer_id: String,
    pub signer_revision: u64,
    pub signer_active_revision: String,
    pub signer_key_id: String,
    pub signer_public_key: String,
    pub signer_secret_name: String,
    pub signer_secret_uid: String,
    pub policy: String,
    pub lock: String,
    pub policy_digest: String,
    pub lock_digest: String,
    pub lock_fingerprint: String,
    pub sources: Vec<FrozenFile>,
    pub profile_json: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Evidence {
    pub source_digest: String,
    pub unsigned_manifest_digest: String,
    pub certification_layer_digest: String,
    pub candidate_image: String,
    pub artifact_digest: String,
    pub artifact_reference: String,
    pub protected_content_root: String,
    pub certificate_digest: String,
    pub expected_manifest_digest: String,
    pub image_digest: String,
    pub content_verified: bool,
    pub managed_compatible: bool,
    pub managed_check_digest: String,
    pub cleanup_complete: bool,
    pub details: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Operation {
    pub id: String,
    pub idempotency_key: String,
    pub repository_id: String,
    pub inspection_id: String,
    pub registry_connection_id: String,
    pub signer_id: String,
    pub platform: String,
    pub state: String,
    pub stage: String,
    pub safe_reason: String,
    pub generation: u64,
    pub allow_build: bool,
    pub allow_replace: bool,
    pub max_builds: u32,
    pub builds_started: u32,
    pub deadline_unix_ms: u64,
    pub accepted_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    pub frozen_input_digest: String,
    pub frozen: FrozenInputs,
    pub evidence: Evidence,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageAttempt {
    pub id: String,
    pub operation_id: String,
    pub stage: String,
    pub generation: u64,
    pub command_digest: String,
    pub pod_uid: String,
    pub state: String,
    pub deadline_unix_ms: u64,
    pub accepted_at_unix_ms: u64,
    pub report_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageClaim {
    pub operation: Operation,
    pub attempt: StageAttempt,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub operation_id: String,
    pub stage: String,
    pub generation: u64,
    pub pod_uid: String,
    pub command_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StageReport {
    pub binding: Binding,
    /// completed, missing, invalid, failed, unavailable, `publication_uncertain` or `cleanup_pending`.
    pub outcome: String,
    pub safe_reason: String,
    pub evidence: Evidence,
}

pub const MAX_WORKER_REQUEST_BYTES: usize = 131_072;
pub const MAX_SUPERVISOR_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// The supervisor alone may adopt workload identities or confirm their removal.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SupervisorRequest {
    Poll,
    Bind {
        binding: Binding,
    },
    Stopped {
        binding: Binding,
    },
    FailedBeforeStart {
        binding: Binding,
        safe_reason: String,
    },
}

/// Worker-owned milestones. Repository command output is never part of this protocol.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum BuildMilestone {
    CopySource,
    VerifyInputs,
    BuildImage,
    ExportImage,
    InspectImage,
    ImageReady,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerRequest {
    Input,
    BuildProgress { milestone: BuildMilestone },
    PreparePublication { expected_manifest_digest: String },
    Report { report: Box<StageReport> },
}

/// Stage credentials use a separate no-store response and never enter this envelope.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerResponse {
    pub claim: Option<StageClaim>,
    pub accepted: bool,
}

/// Domain-separated per-stage capability. Possession grants no supervisor authority;
/// the receiver must still validate the exact Pod UID and retained command digest.
#[must_use]
pub fn worker_token(
    secret: &[u8],
    operation: &str,
    stage: &str,
    generation: u64,
    command_digest: &str,
) -> String {
    use hmac::{Hmac, Mac as _};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret)
        .unwrap_or_else(|_| unreachable!("HMAC accepts every key length"));
    mac.update(b"ayni.runtime-preparation.worker.v1\0");
    let generation = generation.to_string();
    for value in [operation, stage, generation.as_str(), command_digest] {
        mac.update(&(value.len() as u64).to_be_bytes());
        mac.update(value.as_bytes());
    }
    format!("{:x}", mac.finalize().into_bytes())
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    #[test]
    fn stage_capabilities_are_bound_to_every_authority_coordinate() {
        let token = worker_token(b"synthetic-authority", "operation", "build", 1, "digest");
        for (operation, stage, generation, digest) in [
            ("other", "build", 1, "digest"),
            ("operation", "publish", 1, "digest"),
            ("operation", "build", 2, "digest"),
            ("operation", "build", 1, "other"),
        ] {
            assert_ne!(
                token,
                worker_token(b"synthetic-authority", operation, stage, generation, digest)
            );
        }
        assert_ne!(
            token,
            crate::task_execution::worker_token(b"synthetic-authority", "operation", 1)
        );
    }
    #[test]
    fn tooling_refs_accept_local_and_fixed_registry_digests_without_mutable_or_unsafe_paths() {
        let hash = format!("sha256:{}", "a".repeat(64));
        for repository in [
            "local/ayni-runtime-worker",
            "docker.io/library/tool",
            "quay.io/ayni/tool",
            "ghcr.io/ayni-oss/executor",
        ] {
            assert!(valid_tool_image_reference(&format!("{repository}@{hash}")));
        }
        for reference in [
            "local/worker:latest".to_owned(),
            format!("local/../worker@{hash}"),
            format!("quay.io/tool:latest@{hash}"),
            format!("quay.io:5000/tool@{hash}"),
            format!("quay.io/tool@{hash}?token=x"),
        ] {
            assert!(!valid_tool_image_reference(&reference));
        }
    }
}
