use super::{RequestContext, WorkflowGrpc, profile::RuntimeConfiguration, v1};
use ayni_core::{AyniPolicy, EnvironmentLock, sha256_fingerprint};
use ayni_orchestrator_events::runtime_preparation::{FreezeInputs, FrozenFile, Operation};
use ayni_orchestrator_workflow_postgres::store::{
    runtime_preparations::Start, runtime_signers::Signer,
};
use tonic::Status;

pub(super) fn public(value: Operation) -> v1::RuntimePreparation {
    v1::RuntimePreparation {
        id: value.id,
        idempotency_key: value.idempotency_key,
        repository_id: value.repository_id,
        inspection_id: value.inspection_id,
        registry_connection_id: value.registry_connection_id,
        signer_id: value.signer_id,
        platform: value.platform,
        state: value.state,
        stage: value.stage,
        safe_reason: value.safe_reason,
        generation: value.generation,
        allow_build: value.allow_build,
        allow_replace: value.allow_replace,
        max_builds: value.max_builds,
        builds_started: value.builds_started,
        deadline_unix_ms: value.deadline_unix_ms,
        frozen_input_digest: value.frozen_input_digest,
        image_digest: value.evidence.image_digest,
        candidate_image: value.evidence.candidate_image,
        protected_content_root: value.evidence.protected_content_root,
        certificate_digest: value.evidence.certificate_digest,
        content_verified: value.evidence.content_verified,
        managed_compatible: value.evidence.managed_compatible,
        cleanup_complete: value.evidence.cleanup_complete,
        accepted_at_unix_ms: value.accepted_at_unix_ms,
        updated_at_unix_ms: value.updated_at_unix_ms,
        expected_manifest_digest: value.evidence.expected_manifest_digest,
        source_commit: value.frozen.source_commit,
        source_tree: value.frozen.source_tree,
        policy_digest: value.frozen.policy_digest,
        lock_digest: value.frozen.lock_digest,
        lock_fingerprint: value.frozen.lock_fingerprint,
        registry_destination: value.frozen.registry_destination,
        managed_check_digest: value.evidence.managed_check_digest,
        unsigned_manifest_digest: value.evidence.unsigned_manifest_digest,
        certification_layer_digest: value.evidence.certification_layer_digest,
    }
}

fn start_command(value: &v1::StartRuntimePreparationRequest) -> Start<'_> {
    Start {
        key: &value.idempotency_key,
        repository_id: &value.repository_id,
        inspection_id: &value.inspection_id,
        registry_connection_id: &value.registry_connection_id,
        signer_id: &value.signer_id,
        platform: &value.platform,
        max_seconds: value.max_seconds,
        max_builds: value.max_builds,
        allow_build: value.allow_build,
        allow_replace: value.allow_replace,
    }
}

pub(super) async fn start(
    service: &WorkflowGrpc,
    context: &RequestContext,
    value: &v1::StartRuntimePreparationRequest,
) -> Result<Operation, Status> {
    let command = start_command(value);
    if let Some(operation) = service
        .store
        .replay_runtime_start(&command)
        .await
        .map_err(|e| context.store(&e))?
    {
        return Ok(operation);
    }
    let profile = RuntimeConfiguration::from_environment()?;
    profile.admit(&value.platform, value.max_seconds, value.max_builds)?;
    let inputs = admission_source(service, context, value).await?;
    let frozen = freeze_inputs(service, context, value, &profile, inputs).await?;
    service
        .store
        .start_runtime_preparation(command, frozen)
        .await
        .map_err(|e| context.store(&e))
}

async fn admission_source(
    service: &WorkflowGrpc,
    context: &RequestContext,
    value: &v1::StartRuntimePreparationRequest,
) -> Result<super::super::repositories::RuntimeSourceInputs, Status> {
    let inspected = service
        .store
        .get_runtime_inspection_by_id(&value.inspection_id)
        .await
        .map_err(|e| context.store(&e))?;
    validate_inspection(&inspected, &value.repository_id)?;
    let repository = service
        .store
        .inspection_repository(&value.repository_id)
        .await
        .map_err(|e| context.store(&e))?;
    let credential = service
        .store
        .repository_source(&repository.connection_id, inspected.connection_revision)
        .await
        .map_err(|e| context.store(&e))?;
    tokio::time::timeout(
        std::time::Duration::from_secs(25),
        super::super::runtime_source_inputs(&credential, &repository, &inspected.commit_sha),
    )
    .await
    .map_err(|_| Status::unavailable("github_unavailable"))?
    .map_err(source_error)
}

async fn freeze_inputs(
    service: &WorkflowGrpc,
    context: &RequestContext,
    value: &v1::StartRuntimePreparationRequest,
    profile: &RuntimeConfiguration,
    inputs: super::super::repositories::RuntimeSourceInputs,
) -> Result<FreezeInputs, Status> {
    let signer = service
        .store
        .get_runtime_signer(&value.signer_id)
        .await
        .map_err(|e| context.store(&e))?;
    let policy = std::str::from_utf8(&inputs.policy)
        .map_err(|_| Status::failed_precondition("committed policy is invalid"))?;
    let lock = std::str::from_utf8(&inputs.lock)
        .map_err(|_| Status::failed_precondition("committed lock is invalid"))?;
    validate_trust(policy, lock, &signer)?;
    Ok(FreezeInputs {
        expected_signer_revision: signer.revision,
        source_tree: inputs.tree_sha,
        policy: policy.into(),
        lock: lock.into(),
        sources: inputs
            .files
            .into_iter()
            .map(|(path, content)| FrozenFile {
                path,
                digest: sha256_fingerprint(&content),
                content,
            })
            .collect(),
        profile_json: serde_json::to_string(profile)
            .map_err(|_| Status::internal("runtime profile serialization failed"))?,
    })
}

fn validate_inspection(
    inspection: &ayni_orchestrator_workflow_postgres::store::repository_inspections::Inspection,
    repository_id: &str,
) -> Result<(), Status> {
    if inspection.repository_id != repository_id || inspection.state != "completed" {
        return Err(Status::failed_precondition(
            "completed frozen repository inspection required",
        ));
    }
    let result = inspection.result.as_ref().ok_or_else(|| {
        Status::failed_precondition("completed frozen repository inspection required")
    })?;
    if result.ayni_version != "0.17.1" {
        return Err(Status::failed_precondition(
            "regenerate and commit the lock with Ayni 0.17.1, then inspect that commit before runtime preparation",
        ));
    }
    if result.findings.iter().any(|finding| {
        !matches!(
            finding.code.as_str(),
            "adapter_discovery_pending" | "runtime_verification_pending"
        )
    }) {
        return Err(Status::failed_precondition(
            "resolve retained static inspection findings before runtime preparation",
        ));
    }
    Ok(())
}

fn validate_trust(policy: &str, lock: &str, signer: &Signer) -> Result<(), Status> {
    let invalid = || {
        Status::failed_precondition(
            "selected signer must match the committed nonempty certificate trust policy and lock",
        )
    };
    if !signer.enabled || signer.key_id == "unsigned" {
        return Err(invalid());
    }
    let policy = AyniPolicy::parse(policy).map_err(|_| invalid())?;
    let trust = policy
        .environment_certificate_trust_policy()
        .map_err(|_| invalid())?;
    let lock: EnvironmentLock = serde_json::from_str(lock).map_err(|_| invalid())?;
    if lock.ayni_version() != "0.17.1" {
        return Err(Status::failed_precondition(
            "regenerate and commit the lock with Ayni 0.17.1, then inspect that commit",
        ));
    }
    let key = decode_public_key(&signer.public_key).ok_or_else(invalid)?;
    if trust.is_empty()
        || !trust.trusts_key(&signer.key_id, &key)
        || trust.fingerprint() != lock.certificate_trust_policy().fingerprint
    {
        return Err(invalid());
    }
    Ok(())
}

fn decode_public_key(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return None;
    }
    let mut bytes = [0; 32];
    for (index, output) in bytes.iter_mut().enumerate() {
        *output = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

fn source_error(reason: &'static str) -> Status {
    match reason {
        "github_unavailable" | "github_rate_limited" => Status::unavailable(reason),
        "github_access_denied"
        | "github_authentication_failed"
        | "github_key_invalid"
        | "github_repository_unavailable" => Status::permission_denied(reason),
        _ => Status::failed_precondition(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_keys_require_exact_canonical_bytes() {
        assert_eq!(decode_public_key(&"01".repeat(32)), Some([1; 32]));
        for value in [
            "a".repeat(63),
            "A".repeat(64),
            "g".repeat(64),
            "é".repeat(32),
        ] {
            assert!(decode_public_key(&value).is_none());
        }
    }

    #[test]
    fn admission_does_not_treat_completed_inspection_as_resolved_findings() {
        use ayni_orchestrator_workflow_postgres::store::repository_inspections::{
            Finding, Inspection, InspectionResult,
        };
        let mut inspection = Inspection {
            id: "inspection".into(),
            idempotency_key: "request".into(),
            repository_id: "repository".into(),
            connection_revision: 1,
            commit_sha: "a".repeat(40),
            state: "completed".into(),
            safe_reason: String::new(),
            result: Some(InspectionResult {
                policy_digest: String::new(),
                lock_digest: String::new(),
                lock_fingerprint: String::new(),
                ayni_version: "0.17.1".into(),
                lock_schema: String::new(),
                tools: vec![],
                trust_key_ids: vec![],
                findings: vec![Finding {
                    code: "adapter_discovery_pending".into(),
                    path: ".ayni.lock".into(),
                    action: String::new(),
                }],
            }),
            accepted_at_unix_ms: 0,
            updated_at_unix_ms: 0,
        };
        assert!(validate_inspection(&inspection, "repository").is_ok());
        assert!(validate_inspection(&inspection, "another-repository").is_err());
        for unsupported in ["0.16.2", "0.17.0", "0.18.0"] {
            inspection
                .result
                .as_mut()
                .unwrap_or_else(|| panic!("fixture result"))
                .ayni_version = unsupported.into();
            assert!(validate_inspection(&inspection, "repository").is_err());
        }
        inspection
            .result
            .as_mut()
            .unwrap_or_else(|| panic!("fixture result"))
            .ayni_version = "0.17.1".into();
        inspection
            .result
            .as_mut()
            .unwrap_or_else(|| panic!("fixture result"))
            .findings
            .push(Finding {
                code: "trust_required".into(),
                path: ".ayni.toml".into(),
                action: String::new(),
            });
        assert!(validate_inspection(&inspection, "repository").is_err());
        inspection.result = None;
        assert!(validate_inspection(&inspection, "repository").is_err());
    }
}
