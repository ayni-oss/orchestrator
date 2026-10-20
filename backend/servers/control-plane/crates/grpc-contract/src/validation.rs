//! Request validation for the canonical M1 private-v1 workflow surface.

use crate::v1;

/// A stable validation failure suitable for mapping to `ErrorDetail`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError {
    /// Stable machine-readable reason.
    pub reason: v1::ErrorReason,
    /// Request field that failed validation.
    pub field: &'static str,
}

impl ValidationError {
    const fn new(reason: v1::ErrorReason, field: &'static str) -> Self {
        Self { reason, field }
    }
}

/// Validate a caller-generated mutation identity.
///
/// # Errors
/// Returns the violated identity constraint.
pub fn idempotency_key(value: &str) -> Result<(), ValidationError> {
    if !(1..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidIdentifier,
            "idempotency_key",
        ));
    }
    Ok(())
}

/// Validate an opaque resource identifier used by the workflow API.
///
/// # Errors
/// Returns the violated identity constraint.
pub fn resource_id(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if !(1..=128).contains(&value.len())
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidIdentifier,
            field,
        ));
    }
    Ok(())
}

mod projects;
pub use projects::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_identifiers_are_bounded_and_safe() {
        assert!(idempotency_key("diagnostic-01JABC").is_ok());
        assert!(idempotency_key("").is_err());
        assert!(idempotency_key(&"a".repeat(129)).is_err());
        assert!(idempotency_key("line\nbreak").is_err());

        assert!(resource_id("01JABC", "workspace_id").is_ok());
        assert!(resource_id("-invalid", "workspace_id").is_err());
        assert!(resource_id("invalid/path", "workspace_id").is_err());
    }
}
