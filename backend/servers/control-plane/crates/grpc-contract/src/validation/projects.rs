use super::{ValidationError, idempotency_key, resource_id, v1};
/// Validates a project display name.
///
/// # Errors
/// Returns an invalid input error for blank, oversized or control-bearing names.
pub fn project_name(value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidInput,
            "display_name",
        ));
    }
    Ok(())
}

/// Validates project creation.
///
/// # Errors
/// Returns the violated public field bound.
pub fn create_project(value: &v1::CreateProjectRequest) -> Result<(), ValidationError> {
    idempotency_key(&value.idempotency_key)?;
    project_name(&value.display_name)?;
    resource_id(&value.repository_connection_id, "repository_connection_id")
}

/// Validates project mutation identity and optimistic concurrency.
///
/// # Errors
/// Returns the violated public field bound.
pub fn project_mutation(key: &str, id: &str, version: u64) -> Result<(), ValidationError> {
    idempotency_key(key)?;
    resource_id(id, "project_id")?;
    if version == 0 || version > i64::MAX as u64 {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidInput,
            "expected_resource_version",
        ));
    }
    Ok(())
}
