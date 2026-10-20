use super::{ValidationError, idempotency_key, resource_id};
use crate::v1;

/// Validates a workflow display name.
///
/// # Errors
/// Returns invalid input for blank, oversized, whitespace-padded, or control-bearing names.
pub fn display_name(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(ValidationError::new(v1::ErrorReason::InvalidInput, field));
    }
    Ok(())
}

/// Validates workspace creation.
///
/// # Errors
/// Returns the first violated public field bound.
pub fn create_workspace(value: &v1::CreateWorkspaceRequest) -> Result<(), ValidationError> {
    idempotency_key(&value.idempotency_key)?;
    display_name(&value.display_name, "display_name")
}

/// Validates project creation and its authoritative parent scope.
///
/// # Errors
/// Returns the first violated public field bound.
pub fn create_project(value: &v1::CreateProjectRequest) -> Result<(), ValidationError> {
    idempotency_key(&value.idempotency_key)?;
    display_name(&value.display_name, "display_name")?;
    resource_id(&value.workspace_id, "workspace_id")?;
    bounded_text(&value.goal, 16_384, true, "goal")
}

/// Validates conversation creation and complete parent scope.
///
/// # Errors
/// Returns the first violated public field bound.
pub fn create_conversation(value: &v1::CreateConversationRequest) -> Result<(), ValidationError> {
    idempotency_key(&value.idempotency_key)?;
    resource_id(&value.workspace_id, "workspace_id")?;
    resource_id(&value.project_id, "project_id")?;
    display_name(&value.title, "title")
}

/// Validates durable user-message admission.
///
/// # Errors
/// Returns the first violated public field bound.
pub fn send_message(value: &v1::SendMessageRequest) -> Result<(), ValidationError> {
    idempotency_key(&value.idempotency_key)?;
    conversation_scope(
        &value.workspace_id,
        &value.project_id,
        &value.conversation_id,
    )?;
    bounded_text(&value.content, 16_384, false, "content")
}

/// Validates a complete conversation scope before any child lookup.
///
/// # Errors
/// Returns the malformed scope field.
pub fn conversation_scope(
    workspace_id: &str,
    project_id: &str,
    conversation_id: &str,
) -> Result<(), ValidationError> {
    resource_id(workspace_id, "workspace_id")?;
    resource_id(project_id, "project_id")?;
    resource_id(conversation_id, "conversation_id")
}

/// Validates bounded history pagination.
///
/// # Errors
/// Returns invalid input for a missing snapshot or an excessive page.
pub fn history(value: &v1::ListConversationHistoryRequest) -> Result<(), ValidationError> {
    conversation_scope(
        &value.workspace_id,
        &value.project_id,
        &value.conversation_id,
    )?;
    resource_id(&value.snapshot_id, "snapshot_id")?;
    if value.limit > 100 {
        return Err(ValidationError::new(
            v1::ErrorReason::BoundExceeded,
            "limit",
        ));
    }
    Ok(())
}

fn bounded_text(
    value: &str,
    maximum: usize,
    allow_empty: bool,
    field: &'static str,
) -> Result<(), ValidationError> {
    if (!allow_empty && value.is_empty())
        || value.len() > maximum
        || value
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(ValidationError::new(v1::ErrorReason::InvalidInput, field));
    }
    Ok(())
}
