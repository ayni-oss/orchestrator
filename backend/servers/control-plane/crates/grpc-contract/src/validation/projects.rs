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
    if value.repository_connection_ids.is_empty() {
        resource_id(&value.repository_connection_id, "repository_connection_id")
    } else {
        if !value.repository_connection_id.is_empty() {
            return Err(ValidationError::new(
                v1::ErrorReason::InvalidInput,
                "repository_connection_ids",
            ));
        }
        project_members(&value.repository_connection_ids)
    }
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

/// Validates a nonempty set of shared repository members.
///
/// # Errors
/// Returns invalid input for duplicates, invalid identities or an excessive list.
pub fn project_members(values: &[String]) -> Result<(), ValidationError> {
    if values.is_empty()
        || values.len() > 100
        || values
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != values.len()
    {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidInput,
            "repository_connection_ids",
        ));
    }
    for value in values {
        resource_id(value, "repository_connection_ids")?;
    }
    Ok(())
}
