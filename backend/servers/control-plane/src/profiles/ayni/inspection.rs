//! Managed onboarding observations, distinct from certificate verification.
use ayni_orchestrator_domain::digest;
#[derive(Default)]
pub struct LockInspection {
    pub ayni_version: String,
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
    if bytes.len() > 1_048_576 {
        result.reason = "lock_invalid";
        return result;
    }
    let version = lock["ayni_version"].as_str().unwrap_or_default();
    result.ayni_version = version.into();
    if !version.is_empty()
        && ayni_orchestrator_profile_ayni::compatibility::contract(version).is_err()
    {
        result.reason = "runtime_version_unsupported";
        return result;
    }
    if let Ok(identity) = super::environment::parse_lock(bytes) {
        result.lock_compatible = policy_present;
        result.lock_fingerprint = identity.fingerprint;
        result.ayni_version = identity.ayni_version;
    }
    // This is a pinned-policy observation, never certificate verification.
    result.trust_policy_pinned = lock["certificate_trust_policy"]["fingerprint"]
        .as_str()
        .is_some_and(|p| digest::digest(p) && p != digest::sha256(b"{}"));
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
        let mut lock = serde_json::json!({"schema_version":"0.9.0","recipe_version":"2","ayni_version":"0.15.0","fingerprint":format!("sha256:{}","a".repeat(64)),"certificate_trust_policy":{"fingerprint":ayni_orchestrator_domain::digest::sha256(b"{}")}});
        let result = inspect_lock(&serde_json::to_vec(&lock)?, true);
        assert!(result.lock_compatible);
        assert!(!result.trust_policy_pinned);
        assert_eq!(result.reason, "trust_policy_missing");
        lock["ayni_version"] = "unsupported".into();
        let result = inspect_lock(&serde_json::to_vec(&lock)?, true);
        assert!(!result.lock_compatible);
        assert_eq!(result.reason, "runtime_version_unsupported");
        Ok(())
    }
}
