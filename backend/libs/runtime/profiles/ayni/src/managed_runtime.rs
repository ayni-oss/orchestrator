//! Admission of results from the trusted Ayni executable.
//!
//! Ayni owns signature
//! and protected-content verification; this module enforces mandatory signing
//! and binds its result to the controller's frozen source and runtime identity.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Maximum bytes accepted from Ayni's runtime metadata or lock.
pub const MAX_METADATA_BYTES: usize = 1_048_576;
/// Maximum size of an Ayni signal artifact inspected by the launcher.
pub const MAX_SIGNALS_BYTES: usize = 8_388_608;

/// Controller-owned expectations, fixed before running repository code.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeExpectation {
    pub lock_fingerprint: String,
    pub platform: String,
    pub ayni_version: String,
}

/// Bounded provenance retained only after the trusted Ayni invocation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VerifiedRuntime {
    pub lock_fingerprint: String,
    pub platform: String,
    pub ayni_version: String,
    pub certificate_key_id: String,
    pub metadata_digest: String,
    pub protected_content_root: String,
}

/// The trusted verifier did not establish the expected signed runtime.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum RuntimeVerificationError {
    #[error("runtime verification input is malformed or exceeds its bound")]
    Malformed,
    #[error("managed runtime requires a signed certificate")]
    Unsigned,
    #[error("runtime identity differs from the frozen expectation")]
    LockMismatch,
    #[error("runtime platform differs from the frozen expectation")]
    PlatformMismatch,
    #[error("runtime Ayni version is unsupported")]
    UnsupportedVersion,
    #[error("Ayni did not report verification of this runtime")]
    MissingVerification,
}

/// Reads a signed certificate's structural identity before invoking Ayni.
/// This is deliberately not a cryptographic verifier and cannot grant readiness.
///
/// # Errors
/// Returns an error for unsigned, malformed, or incompatible metadata.
pub fn signed_identity(
    metadata: &[u8],
    expected: &RuntimeExpectation,
) -> Result<VerifiedRuntime, RuntimeVerificationError> {
    let (metadata_schema, _) = evidence_contract(&expected.ayni_version)?;
    let value = bounded_json(metadata, MAX_METADATA_BYTES)?;
    let key = field(&value, "key_id")?;
    let signature = field(&value, "signature");
    if key == "unsigned" || signature.as_ref().is_ok_and(String::is_empty) {
        return Err(RuntimeVerificationError::Unsigned);
    }
    let signature = signature?;
    if key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        || !hex(&signature, 128)
    {
        return Err(RuntimeVerificationError::Malformed);
    }
    let claims = &value["certificate"];
    let identity = VerifiedRuntime {
        lock_fingerprint: field(claims, "lock_fingerprint")?,
        platform: field(claims, "platform")?,
        ayni_version: field(claims, "ayni_version")?,
        certificate_key_id: key,
        metadata_digest: format!("sha256:{:x}", Sha256::digest(metadata)),
        protected_content_root: field(claims, "protected_content_root")?,
    };
    if claims["schema_version"] != metadata_schema
        || !digest(&identity.lock_fingerprint)
        || !digest(&identity.protected_content_root)
        || !digest(&field(claims, "tool_inventory_digest")?)
    {
        return Err(RuntimeVerificationError::Malformed);
    }
    if identity.lock_fingerprint != expected.lock_fingerprint {
        return Err(RuntimeVerificationError::LockMismatch);
    }
    if identity.platform != expected.platform
        || !matches!(expected.platform.as_str(), "linux/amd64" | "linux/arm64")
    {
        return Err(RuntimeVerificationError::PlatformMismatch);
    }
    if identity.ayni_version != expected.ayni_version {
        return Err(RuntimeVerificationError::UnsupportedVersion);
    }
    Ok(identity)
}

/// Binds freshly emitted Ayni signals to signed runtime metadata.
///
/// The caller
/// must remove old signals first and run the trusted Ayni executable itself.
/// Quality failures may still have valid runtime provenance; host results may not.
///
/// # Errors
/// Returns an error when fresh signals do not identify the signed environment.
pub fn verified_result(
    metadata: &[u8],
    signals: &[u8],
    expected: &RuntimeExpectation,
) -> Result<VerifiedRuntime, RuntimeVerificationError> {
    let identity = signed_identity(metadata, expected)?;
    let certificate = bounded_json(metadata, MAX_METADATA_BYTES)?;
    let result = bounded_json(signals, MAX_SIGNALS_BYTES)?;
    if result["schema_version"] != evidence_contract(&expected.ayni_version)?.1
        || result["execution_mode"] != "managed"
        || result["environment_lock_fingerprint"] != identity.lock_fingerprint
        || result["ayni_version"] != identity.ayni_version
        || result["prebuilt_runtime"]["metadata_path"] != "/etc/ayni/runtime.json"
        || result["prebuilt_runtime"]["metadata_digest"] != identity.metadata_digest
        || result["prebuilt_runtime"]["certificate"] != certificate
    {
        return Err(RuntimeVerificationError::MissingVerification);
    }
    Ok(identity)
}

// Already-admitted tasks retain their frozen runtime through worker recreation.
// This evidence reader is intentionally separate from the admission/build/trust
// catalog: it grants no new runtime admission or builder mapping for 0.14.5.
fn evidence_contract(
    version: &str,
) -> Result<(&'static str, &'static str), RuntimeVerificationError> {
    match version {
        "0.14.5" => Ok(("2", "0.5.0")),
        _ => crate::compatibility::contract(version)
            .map(|contract| (contract.metadata_schema, contract.signals_schema))
            .map_err(|_| RuntimeVerificationError::UnsupportedVersion),
    }
}

fn bounded_json(bytes: &[u8], limit: usize) -> Result<Value, RuntimeVerificationError> {
    if bytes.len() > limit {
        return Err(RuntimeVerificationError::Malformed);
    }
    serde_json::from_slice(bytes).map_err(|_| RuntimeVerificationError::Malformed)
}

fn field(value: &Value, key: &str) -> Result<String, RuntimeVerificationError> {
    value[key]
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .map(str::to_owned)
        .ok_or(RuntimeVerificationError::Malformed)
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

fn digest(value: &str) -> bool {
    ayni_orchestrator_runtime::runtime_contract::Sha256Digest::parse(value).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (RuntimeExpectation, Vec<u8>, Value) {
        let fingerprint = format!("sha256:{}", "a".repeat(64));
        let expected = RuntimeExpectation {
            lock_fingerprint: fingerprint.clone(),
            platform: "linux/amd64".into(),
            ayni_version: "0.15.0".into(),
        };
        let certificate = json!({"key_id":"producer", "signature":"b".repeat(128), "certificate": {
            "schema_version":"2", "lock_fingerprint":fingerprint, "platform":"linux/amd64", "ayni_version":"0.15.0",
            "tool_inventory_digest": format!("sha256:{}", "c".repeat(64)), "protected_content_root":format!("sha256:{}", "d".repeat(64))
        }});
        let metadata = certificate.to_string().into_bytes();
        let signals = json!({"schema_version":"0.5.0", "execution_mode":"managed", "environment_lock_fingerprint":expected.lock_fingerprint,
            "ayni_version":"0.15.0", "prebuilt_runtime":{"metadata_path":"/etc/ayni/runtime.json", "metadata_digest":format!("sha256:{:x}", Sha256::digest(&metadata)), "certificate":certificate}});
        (expected, metadata, signals)
    }

    #[test]
    fn binds_the_trusted_verifier_result_to_exact_metadata() {
        let (expected, metadata, mut signals) = fixture();
        assert!(verified_result(&metadata, signals.to_string().as_bytes(), &expected).is_ok());
        signals["prebuilt_runtime"]["metadata_digest"] =
            json!(format!("sha256:{}", "f".repeat(64)));
        assert_eq!(
            verified_result(&metadata, signals.to_string().as_bytes(), &expected),
            Err(RuntimeVerificationError::MissingVerification)
        );
    }

    #[test]
    fn rejects_unsigned_host_wrong_lock_and_oversized_results() {
        let (mut expected, metadata, mut signals) = fixture();
        signals["execution_mode"] = json!("host");
        assert!(verified_result(&metadata, signals.to_string().as_bytes(), &expected).is_err());
        expected.lock_fingerprint = format!("sha256:{}", "f".repeat(64));
        assert_eq!(
            signed_identity(&metadata, &expected),
            Err(RuntimeVerificationError::LockMismatch)
        );
        assert_eq!(
            signed_identity(br#"{"key_id":"unsigned","signature":""}"#, &expected),
            Err(RuntimeVerificationError::Unsigned)
        );
        assert_eq!(
            signed_identity(&vec![b' '; MAX_METADATA_BYTES + 1], &expected),
            Err(RuntimeVerificationError::Malformed)
        );
    }

    #[test]
    fn retained_task_evidence_preserves_its_frozen_version_and_fingerprint()
    -> Result<(), Box<dyn std::error::Error>> {
        let (mut expected, metadata, mut signals) = fixture();
        expected.ayni_version = "0.14.5".into();
        let mut certificate: Value = serde_json::from_slice(&metadata)?;
        certificate["certificate"]["ayni_version"] = "0.14.5".into();
        let metadata = serde_json::to_vec(&certificate)?;
        signals["ayni_version"] = "0.14.5".into();
        signals["prebuilt_runtime"]["certificate"] = certificate;
        signals["prebuilt_runtime"]["metadata_digest"] =
            format!("sha256:{:x}", Sha256::digest(&metadata)).into();
        let verified = verified_result(&metadata, &serde_json::to_vec(&signals)?, &expected)?;
        assert_eq!(verified.ayni_version, "0.14.5");
        assert_eq!(verified.lock_fingerprint, expected.lock_fingerprint);
        assert!(crate::compatibility::contract("0.14.5").is_err());
        expected.lock_fingerprint = format!("sha256:{}", "f".repeat(64));
        assert_eq!(
            signed_identity(&metadata, &expected),
            Err(RuntimeVerificationError::LockMismatch)
        );
        for version in ["0.14.4", "0.15.1", "unknown"] {
            expected.ayni_version = version.into();
            assert_eq!(
                signed_identity(&metadata, &expected),
                Err(RuntimeVerificationError::UnsupportedVersion)
            );
        }
        Ok(())
    }
}
