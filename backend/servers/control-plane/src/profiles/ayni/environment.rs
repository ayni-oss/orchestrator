//! Bounded lock envelope observation; Ayni owns full lock and signature validation.
use thiserror::Error;

pub struct LockIdentity {
    pub(crate) fingerprint: String,
    pub(crate) ayni_version: String,
}

#[derive(Debug, Error)]
pub enum AdmissionError {
    #[error("Ayni lock is malformed or outside the supported schema/recipe")]
    InvalidLock,
    #[error("Ayni lock fingerprint is not a complete SHA-256 identity")]
    InvalidFingerprint,
}

pub fn parse_lock(bytes: &[u8]) -> Result<LockIdentity, AdmissionError> {
    let lock = ayni_orchestrator_profile_ayni::compatibility::decode_lock_envelope(bytes)
        .map_err(|_| AdmissionError::InvalidLock)?;
    let contract = ayni_orchestrator_profile_ayni::compatibility::contract(&lock.ayni_version)
        .map_err(|_| AdmissionError::InvalidLock)?;
    if (lock.schema_version.as_str(), lock.recipe_version.as_str())
        != (contract.lock_schema, contract.recipe_version)
    {
        return Err(AdmissionError::InvalidLock);
    }
    let fingerprint =
        ayni_orchestrator_runtime::runtime_contract::Sha256Digest::parse(&lock.fingerprint)
            .map_err(|_| AdmissionError::InvalidFingerprint)?;
    Ok(LockIdentity {
        fingerprint: fingerprint.hex().into(),
        ayni_version: lock.ayni_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_supported_envelope_and_full_fingerprint() {
        let lock = format!(
            r#"{{"ayni_version":"0.15.0","schema_version":"0.9.0","recipe_version":"2","fingerprint":"sha256:{}"}}"#,
            "a".repeat(64)
        );
        assert!(parse_lock(lock.as_bytes()).is_ok());
        assert!(parse_lock(lock.replace("sha256:", "").as_bytes()).is_err());
        assert!(parse_lock(lock.replace("0.9.0", "0.8.0").as_bytes()).is_err());
        assert!(parse_lock(&vec![b'x'; 1_048_577]).is_err());
    }
}
