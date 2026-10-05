//! Managed onboarding observations, distinct from certificate verification.
use ayni_orchestrator_domain::digest as assessment;
use ayni_orchestrator_profile_ayni::ayni_release;
#[derive(Default)]
pub struct LockInspection {
    pub lock_compatible: bool,
    pub lock_fingerprint: String,
    pub trust_policy_pinned: bool,
    pub reason: &'static str,
}
pub fn inspect_lock(bytes: &[u8], policy_present: bool) -> LockInspection {
    let mut result = LockInspection::default();
    let Ok(lock) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        result.reason = "lock_invalid";
        return result;
    };
    let fingerprint = lock["fingerprint"].as_str().unwrap_or("");
    result.lock_compatible = assessment::digest(fingerprint)
        && lock["ayni_version"] == ayni_release::CLI_VERSION
        && lock["schema_version"] == "0.9.0";
    if result.lock_compatible {
        result.lock_fingerprint = fingerprint.trim_start_matches("sha256:").into();
    }
    // This is a pinned-policy observation, never certificate verification.
    result.trust_policy_pinned = lock["certificate_trust_policy"]["fingerprint"]
        .as_str()
        .is_some_and(|p| assessment::digest(p) && p != assessment::sha256(b"{}"));
    result.reason = if !policy_present {
        "policy_missing"
    } else if !result.lock_compatible {
        "lock_incompatible"
    } else if !result.trust_policy_pinned {
        "trust_policy_missing"
    } else {
        "signing_configuration_requires_operator"
    };
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_and_policy_pin_do_not_claim_certificate_verification()
    -> Result<(), serde_json::Error> {
        let mut lock = serde_json::json!({"schema_version":"0.9.0","ayni_version":ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION,"fingerprint":format!("sha256:{}","a".repeat(64)),"certificate_trust_policy":{"fingerprint":ayni_orchestrator_domain::digest::sha256(b"{}")}});
        let result = inspect_lock(&serde_json::to_vec(&lock)?, true);
        assert!(result.lock_compatible);
        assert!(!result.trust_policy_pinned);
        assert_eq!(result.reason, "trust_policy_missing");
        lock["ayni_version"] = "unsupported".into();
        let result = inspect_lock(&serde_json::to_vec(&lock)?, true);
        assert!(!result.lock_compatible);
        assert_eq!(result.reason, "lock_incompatible");
        Ok(())
    }
}
