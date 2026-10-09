//! Product diagnostics from bounded internal codes, never raw provider output.
use ayni_orchestrator_api::v1;

pub fn finding(code: &str) -> v1::RuntimeFinding {
    let (message, remediation) = match code {
        "policy_missing" => (
            "Committed Ayni configuration is missing.",
            "Commit .ayni.toml through an owner-reviewed PR, then analyze again.",
        ),
        "lock_missing" => (
            "The committed Ayni lock is missing.",
            "Generate and commit .ayni.lock with the supported Ayni CLI, then analyze again.",
        ),
        "lock_incompatible" | "repository_lock_invalid" => (
            "The committed lock is invalid or incompatible.",
            "Refresh the lock with the supported Ayni CLI and commit it through review.",
        ),
        "runtime_version_unsupported" => (
            "The repository selects an unsupported Ayni contract.",
            "Upgrade the committed policy and lock with Ayni 0.15.0, review the changes, then analyze again.",
        ),
        "runtime_resolver_unsupported" => (
            "The lock selects an unsupported resolver.",
            "Refresh the lock with Ayni 0.15.0 and mise 2026.6.14, review and commit it, then analyze again.",
        ),
        "official_builder_unavailable" => (
            "The official builder is unavailable for the selected contract and platform.",
            "Restore GHCR access or choose a supported platform, then retry analysis.",
        ),
        "certificate_trust_pull_closed" => (
            "The certificate trust proposal was closed without merging.",
            "Choose Retry to reopen the proposal, then review and merge it.",
        ),
        "runtime_lock_mismatch" => (
            "The environment does not match the frozen lock.",
            "Review configuration and lock consistency, then rebuild.",
        ),
        "trust_policy_missing" | "certificate_trust_lock_failed" => (
            "Repository signing trust needs attention.",
            "Review the certificate trust update PR and its regenerated lock; merge it before building.",
        ),
        "certificate_setup_required" | "certificate_not_ready" => (
            "A usable signing certificate is required.",
            "Select an enabled, ready signing certificate.",
        ),
        "certificate_verification_mismatch" | "runtime_certificate_invalid" => (
            "The image certificate does not match trusted signing configuration.",
            "Review the selected certificate and repository trust, then rebuild.",
        ),
        "runtime_platform_mismatch" => (
            "The image platform does not match this runtime.",
            "Check builder architecture and rebuild for the selected platform.",
        ),
        "runtime_tag_drift" | "runtime_digest_mismatch" => (
            "The image identity changed unexpectedly.",
            "Review registry access and the changed image, then explicitly rebuild.",
        ),
        "runtime_verification_failed" | "runtime_result_mismatch" => (
            "Image certification or protected content verification failed.",
            "Review the image, committed environment, and signing trust, then rebuild.",
        ),
        "registry_access_denied"
        | "registry_credentials_invalid"
        | "registry_connection_not_ready"
        | "registry_connection_missing" => (
            "Registry access is unavailable.",
            "Repair and test the selected registry connection, then retry analysis.",
        ),
        "trust_lock_builder_missing" | "trust_lock_image_unpinned" => (
            "Public certificate trust preparation tooling is unavailable.",
            "Deploy the trusted launcher and approved resolver, then retry analysis. The signing certificate itself is not the missing builder.",
        ),
        "runtime_builder_missing"
        | "runtime_build_configuration_invalid"
        | "runtime_source_missing" => (
            "Approved isolated build infrastructure is unavailable.",
            "Configure an explicitly approved isolated TLS engine and runtime registry.",
        ),
        "runtime_cleanup_pending" => (
            "Temporary preparation resources are being removed.",
            "Preparation resumes automatically after cleanup completes.",
        ),
        "runtime_build_failed" => (
            "The environment build failed.",
            "Review committed environment inputs and builder health, then rebuild.",
        ),
        "runtime_publication_failed" => (
            "The certified image could not be published.",
            "Repair registry access and retry; an evicted builder candidate is rebuilt from frozen inputs.",
        ),
        "registry_rate_limited" | "registry_unavailable" | "registry_discovery_timeout" => (
            "The registry could not be inspected reliably.",
            "Wait for registry access to recover, then retry analysis. No missing-image build was authorized.",
        ),
        _ => (
            "Runtime preparation could not complete.",
            "Review source access and runtime infrastructure, then retry analysis.",
        ),
    };
    v1::RuntimeFinding {
        code: code.into(),
        message: message.into(),
        remediation: remediation.into(),
    }
}

pub fn setup_required(code: &str) -> bool {
    matches!(
        code,
        "policy_missing"
            | "lock_missing"
            | "lock_incompatible"
            | "repository_lock_invalid"
            | "runtime_version_unsupported"
            | "runtime_resolver_unsupported"
            | "runtime_lock_contract_unsupported"
            | "certificate_setup_required"
            | "certificate_not_ready"
            | "trust_policy_missing"
            | "certificate_trust_lock_failed"
            | "trust_lock_builder_missing"
            | "trust_lock_image_unpinned"
            | "runtime_builder_missing"
            | "runtime_source_missing"
            | "runtime_build_configuration_invalid"
            | "registry_connection_missing"
            | "registry_connection_not_ready"
    )
}

pub fn rebuild_allowed(code: &str) -> bool {
    matches!(
        code,
        "runtime_lock_mismatch"
            | "runtime_platform_mismatch"
            | "runtime_tag_drift"
            | "runtime_digest_mismatch"
            | "runtime_certificate_invalid"
            | "certificate_verification_mismatch"
            | "runtime_verification_failed"
            | "runtime_result_mismatch"
            | "runtime_build_failed"
            | "runtime_publication_failed"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn unsupported_contract_and_trust_prerequisites_require_remediation() {
        assert!(!super::rebuild_allowed("runtime_version_unsupported"));
        assert!(super::setup_required("runtime_version_unsupported"));
        assert!(super::setup_required("certificate_trust_lock_failed"));
        assert!(!super::rebuild_allowed("certificate_trust_lock_failed"));
    }
}
