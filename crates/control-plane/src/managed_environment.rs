//! Ayni Managed lock and immutable GHCR artifact admission.
//!
//! The controller consumes trusted workflow observations; it does not implement
//! Ayni's certificate or protected-content verifier.

use serde::Deserialize;
use std::collections::BTreeMap;
use thiserror::Error;

const PREFIX: &str = "sha256:";

#[derive(Debug, Deserialize)]
struct Lock {
    schema_version: String,
    recipe_version: String,
    fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LockIdentity {
    pub(crate) fingerprint: String,
}
impl LockIdentity {
    pub(crate) fn discovery_tag(&self) -> String {
        format!("lock-{}", self.fingerprint)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnvironmentObservation {
    pub(crate) oci_repository: String,
    pub(crate) digest: String,
    pub(crate) platform: String,
    pub(crate) labels: BTreeMap<String, String>,
    pub(crate) workflow_revision: String,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub(crate) enum AdmissionError {
    #[error("Ayni lock is malformed or outside the supported schema/recipe")]
    InvalidLock,
    #[error("Ayni lock fingerprint is not a complete SHA-256 identity")]
    InvalidFingerprint,
    #[error("registry observation does not match the expected repository identity")]
    WrongRepository,
    #[error("registry digest is not an immutable SHA-256 identity")]
    InvalidDigest,
    #[error("runtime platform is unsupported")]
    UnsupportedPlatform,
    #[error("runtime metadata does not bind the expected lock fingerprint")]
    FingerprintMismatch,
    #[error("trusted workflow result is not bound to the default-branch revision")]
    StaleWorkflow,
}

pub(crate) fn parse_lock(bytes: &[u8]) -> Result<LockIdentity, AdmissionError> {
    let lock: Lock = serde_json::from_slice(bytes).map_err(|_| AdmissionError::InvalidLock)?;
    if lock.schema_version != "0.9.0" || lock.recipe_version != "2" {
        return Err(AdmissionError::InvalidLock);
    }
    let Some(fingerprint) = lock.fingerprint.strip_prefix(PREFIX) else {
        return Err(AdmissionError::InvalidFingerprint);
    };
    if !sha256_hex(fingerprint) {
        return Err(AdmissionError::InvalidFingerprint);
    }
    Ok(LockIdentity {
        fingerprint: fingerprint.to_owned(),
    })
}

pub(crate) fn admit(
    identity: &LockIdentity,
    expected_repository: &str,
    default_revision: &str,
    observation: &EnvironmentObservation,
) -> Result<(), AdmissionError> {
    if observation.oci_repository != expected_repository {
        return Err(AdmissionError::WrongRepository);
    }
    if !observation.digest.starts_with(PREFIX) || !sha256_hex(&observation.digest[PREFIX.len()..]) {
        return Err(AdmissionError::InvalidDigest);
    }
    if !matches!(observation.platform.as_str(), "linux/amd64" | "linux/arm64") {
        return Err(AdmissionError::UnsupportedPlatform);
    }
    if observation.labels.get("io.ayni.lock-fingerprint") != Some(&identity.fingerprint) {
        return Err(AdmissionError::FingerprintMismatch);
    }
    if observation.workflow_revision != default_revision {
        return Err(AdmissionError::StaleWorkflow);
    }
    Ok(())
}

fn sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    const FINGERPRINT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DIGEST: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    fn observation() -> EnvironmentObservation {
        EnvironmentObservation {
            oci_repository: "ghcr.io/acme/repo/ayni-environment".into(),
            digest: DIGEST.into(),
            platform: "linux/amd64".into(),
            labels: BTreeMap::from([("io.ayni.lock-fingerprint".into(), FINGERPRINT.into())]),
            workflow_revision: "c".repeat(40),
        }
    }
    #[test]
    fn derives_complete_tag_from_committed_lock() {
        let lock = format!(
            r#"{{"schema_version":"0.9.0","recipe_version":"2","fingerprint":"sha256:{FINGERPRINT}"}}"#
        );
        assert_eq!(
            parse_lock(lock.as_bytes()).unwrap().discovery_tag(),
            format!("lock-{FINGERPRINT}")
        );
    }
    #[test]
    fn rejects_stale_observation() {
        let identity = LockIdentity {
            fingerprint: FINGERPRINT.into(),
        };
        assert!(
            admit(
                &identity,
                "ghcr.io/acme/repo/ayni-environment",
                &"c".repeat(40),
                &observation()
            )
            .is_ok()
        );
        let mut stale = observation();
        stale.workflow_revision = "d".repeat(40);
        assert!(
            admit(
                &identity,
                "ghcr.io/acme/repo/ayni-environment",
                &"c".repeat(40),
                &stale
            )
            .is_err()
        );
    }
}
