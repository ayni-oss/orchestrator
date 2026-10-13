//! One canonical event namespace; compatibility readers never emit legacy names.
use crate::{ContractError, MAX_EVENT_BYTES};

const LEGACY_PREFIXES: [&str; 2] = ["io.ayni.", "build.ayni."];
const PREFIX: &str = "ayni.build.";
const TYPES: [&str; 6] = [
    crate::EVENT_TYPE,
    crate::decisions::EVENT_TYPE,
    crate::operations::EVENT_TYPE,
    crate::operation_decisions::EVENT_TYPE,
    crate::coding::EVENT_TYPE,
    crate::coding_decisions::EVENT_TYPE,
];

pub(crate) fn matches(actual: &str, expected: &str) -> bool {
    actual == expected
        || expected.strip_prefix(PREFIX).is_some_and(|suffix| {
            LEGACY_PREFIXES
                .iter()
                .any(|prefix| actual.strip_prefix(prefix) == Some(suffix))
        })
}

/// Adapt retained envelopes for delivery without changing identity or evidence.
///
/// Current envelopes remain byte-identical; legacy adaptation changes only `type`.
/// Callers still validate the typed contract and its authority independently.
/// # Errors
/// Rejects unknown types, malformed JSON and envelopes exceeding the wire limit.
pub fn canonicalize(bytes: Vec<u8>) -> Result<Vec<u8>, ContractError> {
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    let mut envelope: serde_json::Value = serde_json::from_slice(&bytes)?;
    let actual = envelope
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or(ContractError::Invalid("event type"))?;
    let canonical = TYPES
        .into_iter()
        .find(|expected| matches(actual, expected))
        .ok_or(ContractError::Invalid("event type"))?;
    if actual == canonical {
        return Ok(bytes);
    }
    envelope["type"] = serde_json::Value::String(canonical.into());
    let bytes = serde_json::to_vec(&envelope)?;
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_emitted_type_uses_the_same_domain_and_legacy_delivery_is_canonical()
    -> Result<(), ContractError> {
        for kind in TYPES {
            let suffix = kind
                .strip_prefix(PREFIX)
                .ok_or(ContractError::Invalid("domain"))?;
            for prefix in LEGACY_PREFIXES {
                let old = serde_json::to_vec(&serde_json::json!({
                    "type":format!("{prefix}{suffix}"), "id":"stable-id",
                    "traceparent":"retained-context", "data":{"sha256":"retained-evidence"}
                }))?;
                let bytes = canonicalize(old)?;
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                assert_eq!(value["type"], kind);
                assert_eq!(value["id"], "stable-id");
                assert_eq!(value["traceparent"], "retained-context");
                assert_eq!(value["data"]["sha256"], "retained-evidence");
                assert_eq!(canonicalize(bytes.clone())?, bytes);
            }
        }
        assert!(canonicalize(br#"{"type":"other.example.event"}"#.to_vec()).is_err());
        Ok(())
    }
}
