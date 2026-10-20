use super::v1::{
    ConversationReportDecision, ConversationReportDecisionKind, ConversationTurnBinding,
    ConversationTurnReport, ConversationTurnReportKind, ConversationTurnRequested,
};
use super::{ContractError, EventIdentity, digest, resource_identifier, validate_identity};
use cloudevents::event::{ExtensionValue, SpecVersion};
use cloudevents::{AttributesReader, Data, Event, EventBuilder, EventBuilderV10};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use prost::Message as _;
use sha2::{Digest as _, Sha256};

pub const APPLICATION_SOURCE: &str = "ayni://orchestrator/supervisors/workflow-application";
pub const COMMAND_SUBJECT: &str = "orchestrator.workflow.command.conversation.v1";
pub const REPORT_SUBJECT: &str = "orchestrator.workflow.report.conversation.v1";
pub const DECISION_SUBJECT_PREFIX: &str = "orchestrator.workflow.accepted.conversation.v1";

pub const TURN_REQUESTED_TYPE: &str = "build.ayni.orchestrator.conversation.turn.requested.v1";
pub const TURN_STARTED_TYPE: &str = "build.ayni.orchestrator.conversation.turn.started.v1";
pub const TURN_COMPLETED_TYPE: &str = "build.ayni.orchestrator.conversation.turn.completed.v1";
pub const TURN_FAILED_TYPE: &str = "build.ayni.orchestrator.conversation.turn.failed.v1";
pub const REPORT_DECIDED_TYPE: &str = "build.ayni.orchestrator.conversation.report.decided.v1";

pub const COMMAND_SCHEMA: &str = "ayni://orchestrator/schemas/conversation-turn-requested/v1";
pub const REPORT_SCHEMA: &str = "ayni://orchestrator/schemas/conversation-turn-report/v1";
pub const DECISION_SCHEMA: &str = "ayni://orchestrator/schemas/conversation-report-decision/v1";
pub const MAX_COMMAND_BYTES: usize = 256 * 1024;
pub const MAX_REPORT_BYTES: usize = 32 * 1024;
pub const MAX_DECISION_BYTES: usize = 16 * 1024;
pub const MAX_CONTENT_BYTES: usize = 16 * 1024;

/// Required transport metadata retained unchanged across delivery retries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowEventMetadata {
    pub produced_at: String,
    pub traceparent: String,
}

#[derive(Clone, Copy)]
struct EventContract<'a> {
    event_type: &'a str,
    schema: &'static str,
    maximum: usize,
}

/// Build the one-token decision subject for a retained executor instance.
/// # Errors
/// Rejects an empty, oversized, or non-token executor instance ID.
pub fn decision_subject(executor_instance_id: &str) -> Result<String, ContractError> {
    if !simple_identifier(executor_instance_id) {
        return Err(ContractError::Invalid("executor instance ID"));
    }
    Ok(format!("{DECISION_SUBJECT_PREFIX}.{executor_instance_id}"))
}

/// Encode an immutable conversation command with its application-issued identity.
/// # Errors
/// Rejects invalid scope, context, identity, or an oversized `CloudEvent`.
pub fn encode_command(
    identity: &EventIdentity,
    metadata: &WorkflowEventMetadata,
    payload: &ConversationTurnRequested,
) -> Result<Vec<u8>, ContractError> {
    validate_application_identity(identity)?;
    let binding = validate_command(payload, identity)?;
    encode_event(
        identity,
        metadata,
        EventContract {
            event_type: TURN_REQUESTED_TYPE,
            schema: COMMAND_SCHEMA,
            maximum: MAX_COMMAND_BYTES,
        },
        workflow_subject(binding),
        &binding_extensions(binding),
        payload,
    )
}

/// Decode and validate a bounded conversation command.
/// # Errors
/// Rejects malformed, unsupported, mismatched, or oversized input.
pub fn decode_command(
    bytes: &[u8],
) -> Result<
    (
        EventIdentity,
        WorkflowEventMetadata,
        ConversationTurnRequested,
    ),
    ContractError,
> {
    let (identity, metadata, payload, event) = decode_event(
        bytes,
        TURN_REQUESTED_TYPE,
        COMMAND_SCHEMA,
        MAX_COMMAND_BYTES,
    )?;
    validate_application_identity(&identity)?;
    let binding = validate_command(&payload, &identity)?;
    if event.subject() != Some(workflow_subject(binding).as_str()) {
        return Err(ContractError::Invalid("command subject"));
    }
    validate_binding_extensions(&event, binding)?;
    Ok((identity, metadata, payload))
}

/// Canonical bytes covered by one executor signature. The signature field is
/// omitted and every variable-length component is length-prefixed.
/// # Errors
/// Rejects invalid report metadata before signing.
pub fn report_signing_bytes(
    identity: &EventIdentity,
    payload: &ConversationTurnReport,
) -> Result<Vec<u8>, ContractError> {
    validate_identity(identity)?;
    if !payload.signature.is_empty() && payload.signature.len() != 64 {
        return Err(ContractError::Invalid("report signature"));
    }
    let kind = report_kind(payload)?;
    let mut unsigned = payload.clone();
    unsigned.signature.clear();
    validate_report(&unsigned, false)?;
    validate_report_identity(identity, &unsigned)?;
    let report_binding = binding(&unsigned)?;
    let encoded = unsigned.encode_to_vec();
    let mut result = b"ayni-orchestrator-conversation-report-signature-v1".to_vec();
    for value in [
        kind.as_bytes(),
        identity.source.as_bytes(),
        identity.id.as_bytes(),
        report_binding.assignment_id.as_bytes(),
    ] {
        result.extend_from_slice(
            &u64::try_from(value.len())
                .map_err(|_| ContractError::Oversized)?
                .to_be_bytes(),
        );
        result.extend_from_slice(value);
    }
    result.extend_from_slice(&report_binding.executor_generation.to_be_bytes());
    result.extend_from_slice(&unsigned.report_sequence.to_be_bytes());
    let causation = report_binding.command_event_id.as_bytes();
    result.extend_from_slice(
        &u64::try_from(causation.len())
            .map_err(|_| ContractError::Oversized)?
            .to_be_bytes(),
    );
    result.extend_from_slice(causation);
    result.extend_from_slice(
        &u64::try_from(encoded.len())
            .map_err(|_| ContractError::Oversized)?
            .to_be_bytes(),
    );
    result.extend_from_slice(&encoded);
    Ok(result)
}

/// Domain-separated SHA-256 digest signed by the executor.
/// # Errors
/// Rejects invalid report metadata before hashing.
pub fn report_signing_digest(
    identity: &EventIdentity,
    payload: &ConversationTurnReport,
) -> Result<[u8; 32], ContractError> {
    Ok(Sha256::digest(report_signing_bytes(identity, payload)?).into())
}

/// Return the contract-formatted digest for one Ed25519 public key.
#[must_use]
pub fn signing_key_digest(public_key: &[u8; 32]) -> String {
    let digest = Sha256::digest(public_key);
    format!("sha256:{}", lowercase_hexadecimal(&digest))
}

/// Sign one otherwise complete report using a retained Ed25519 seed.
/// # Errors
/// Rejects metadata that is inconsistent with the signing key or report kind.
pub fn sign_report(
    identity: &EventIdentity,
    payload: &mut ConversationTurnReport,
    signing_seed: &[u8; 32],
) -> Result<(), ContractError> {
    let signing_key = SigningKey::from_bytes(signing_seed);
    let public_key = signing_key.verifying_key().to_bytes();
    if payload.signing_key_digest != signing_key_digest(&public_key)
        || (payload.kind == ConversationTurnReportKind::Started as i32
            && payload.signing_public_key != public_key)
    {
        return Err(ContractError::Invalid("report signing key"));
    }
    payload.signature.clear();
    let digest = report_signing_digest(identity, payload)?;
    payload.signature = signing_key.sign(&digest).to_bytes().to_vec();
    Ok(())
}

/// Verify a report under either its ownership-claim key or the key already bound
/// to the accepted claimant.
/// # Errors
/// Rejects malformed keys/signatures, digest mismatches, or failed verification.
pub fn verify_report_signature(
    identity: &EventIdentity,
    payload: &ConversationTurnReport,
    bound_public_key: Option<&[u8; 32]>,
) -> Result<(), ContractError> {
    validate_report(payload, true)?;
    let report_public_key: [u8; 32] = match report_kind(payload)? {
        TURN_STARTED_TYPE => payload
            .signing_public_key
            .as_slice()
            .try_into()
            .map_err(|_| ContractError::Invalid("report public key"))?,
        TURN_COMPLETED_TYPE | TURN_FAILED_TYPE => bound_public_key
            .copied()
            .ok_or(ContractError::Invalid("missing bound signing key"))?,
        _ => return Err(ContractError::Invalid("report kind")),
    };
    if payload.signing_key_digest != signing_key_digest(&report_public_key) {
        return Err(ContractError::Invalid("report signing-key digest"));
    }
    let signature = Signature::from_slice(&payload.signature)
        .map_err(|_| ContractError::Invalid("report signature"))?;
    let verifying_key = strict_verifying_key(&report_public_key)?;
    verifying_key
        .verify_strict(&report_signing_digest(identity, payload)?, &signature)
        .map_err(|_| ContractError::Invalid("report signature"))
}

fn strict_verifying_key(public_key: &[u8; 32]) -> Result<VerifyingKey, ContractError> {
    let verifying_key = VerifyingKey::from_bytes(public_key)
        .map_err(|_| ContractError::Invalid("report public key"))?;
    if verifying_key.is_weak() {
        return Err(ContractError::Invalid("weak report public key"));
    }
    Ok(verifying_key)
}

/// Encode a signed started/completed/failed executor report.
/// # Errors
/// Rejects invalid binding, signing metadata, outcome, identity, or size.
pub fn encode_report(
    identity: &EventIdentity,
    metadata: &WorkflowEventMetadata,
    payload: &ConversationTurnReport,
) -> Result<Vec<u8>, ContractError> {
    validate_identity(identity)?;
    let kind = report_kind(payload)?;
    validate_report(payload, true)?;
    validate_report_identity(identity, payload)?;
    encode_event(
        identity,
        metadata,
        EventContract {
            event_type: kind,
            schema: REPORT_SCHEMA,
            maximum: MAX_REPORT_BYTES,
        },
        workflow_subject(binding(payload)?),
        &binding_extensions(binding(payload)?),
        payload,
    )
}

/// Decode a signed executor report without assigning application authority.
/// # Errors
/// Rejects malformed, unsupported, mismatched, or oversized input.
pub fn decode_report(
    bytes: &[u8],
) -> Result<(EventIdentity, WorkflowEventMetadata, ConversationTurnReport), ContractError> {
    if bytes.len() > MAX_REPORT_BYTES {
        return Err(ContractError::Oversized);
    }
    let event: Event = serde_json::from_slice(bytes)?;
    if event.specversion() != SpecVersion::V10
        || !matches!(
            event.ty(),
            TURN_STARTED_TYPE | TURN_COMPLETED_TYPE | TURN_FAILED_TYPE
        )
        || event.datacontenttype() != Some("application/json")
        || event
            .dataschema()
            .is_none_or(|schema| schema.as_str() != REPORT_SCHEMA)
    {
        return Err(ContractError::Invalid("report envelope"));
    }
    let identity = EventIdentity {
        source: event.source().clone(),
        id: event.id().to_owned(),
    };
    validate_identity(&identity)?;
    let metadata = decode_workflow_metadata(&event)?;
    let Some(Data::Json(data)) = event.data() else {
        return Err(ContractError::Invalid("report data"));
    };
    let payload: ConversationTurnReport = serde_json::from_value(data.clone())?;
    let kind = report_kind(&payload)?;
    validate_report(&payload, true)?;
    validate_report_identity(&identity, &payload)?;
    let report_binding = binding(&payload)?;
    if event.ty() != kind || event.subject() != Some(workflow_subject(report_binding).as_str()) {
        return Err(ContractError::Invalid("report binding"));
    }
    validate_binding_extensions(&event, report_binding)?;
    Ok((identity, metadata, payload))
}

/// Encode an application acceptance/rejection decision targeted to one executor.
/// # Errors
/// Rejects invalid report identity, assignment, target, or size.
pub fn encode_decision(
    identity: &EventIdentity,
    metadata: &WorkflowEventMetadata,
    payload: &ConversationReportDecision,
) -> Result<Vec<u8>, ContractError> {
    validate_application_identity(identity)?;
    validate_decision(payload)?;
    encode_event(
        identity,
        metadata,
        EventContract {
            event_type: REPORT_DECIDED_TYPE,
            schema: DECISION_SCHEMA,
            maximum: MAX_DECISION_BYTES,
        },
        format!("reports/{}", payload.report_id),
        &decision_extensions(payload),
        payload,
    )
}

/// Decode one targeted application decision.
/// # Errors
/// Rejects malformed, unsupported, mismatched, or oversized input.
pub fn decode_decision(
    bytes: &[u8],
) -> Result<
    (
        EventIdentity,
        WorkflowEventMetadata,
        ConversationReportDecision,
    ),
    ContractError,
> {
    let (identity, metadata, payload, event) = decode_event(
        bytes,
        REPORT_DECIDED_TYPE,
        DECISION_SCHEMA,
        MAX_DECISION_BYTES,
    )?;
    validate_application_identity(&identity)?;
    validate_decision(&payload)?;
    if event.subject() != Some(format!("reports/{}", payload.report_id).as_str()) {
        return Err(ContractError::Invalid("decision subject"));
    }
    validate_decision_extensions(&event, &payload)?;
    Ok((identity, metadata, payload))
}

fn encode_event<T: serde::Serialize>(
    identity: &EventIdentity,
    metadata: &WorkflowEventMetadata,
    contract: EventContract<'_>,
    subject: String,
    extensions: &[(&str, String)],
    payload: &T,
) -> Result<Vec<u8>, ContractError> {
    if !traceparent(&metadata.traceparent) {
        return Err(ContractError::Invalid("workflow trace reference"));
    }
    let mut builder = EventBuilderV10::new()
        .id(identity.id.clone())
        .source(identity.source.clone())
        .ty(contract.event_type)
        .subject(subject)
        .time(metadata.produced_at.as_str())
        .extension("traceparent", metadata.traceparent.clone());
    for (name, value) in extensions {
        builder = builder.extension(name, value.clone());
    }
    let event = builder
        .data_with_schema(
            "application/json",
            contract.schema,
            serde_json::to_value(payload)?,
        )
        .build()
        .map_err(|_| ContractError::Invalid("workflow CloudEvent"))?;
    let bytes = serde_json::to_vec(&event)?;
    if bytes.len() > contract.maximum {
        return Err(ContractError::Oversized);
    }
    Ok(bytes)
}

fn decode_event<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    event_type: &str,
    schema: &str,
    maximum: usize,
) -> Result<(EventIdentity, WorkflowEventMetadata, T, Event), ContractError> {
    if bytes.len() > maximum {
        return Err(ContractError::Oversized);
    }
    let event: Event = serde_json::from_slice(bytes)?;
    if event.specversion() != SpecVersion::V10
        || event.ty() != event_type
        || event.datacontenttype() != Some("application/json")
        || event
            .dataschema()
            .is_none_or(|value| value.as_str() != schema)
    {
        return Err(ContractError::Invalid("workflow envelope"));
    }
    let identity = EventIdentity {
        source: event.source().clone(),
        id: event.id().to_owned(),
    };
    validate_identity(&identity)?;
    let metadata = decode_workflow_metadata(&event)?;
    let Some(Data::Json(data)) = event.data() else {
        return Err(ContractError::Invalid("workflow data"));
    };
    let payload = serde_json::from_value(data.clone())?;
    Ok((identity, metadata, payload, event))
}

fn decode_workflow_metadata(event: &Event) -> Result<WorkflowEventMetadata, ContractError> {
    let produced_at = event
        .time()
        .copied()
        .ok_or(ContractError::Invalid("workflow produced time"))?;
    let Some(ExtensionValue::String(trace_reference)) = event.extension("traceparent") else {
        return Err(ContractError::Invalid("workflow trace reference"));
    };
    if !traceparent(trace_reference) {
        return Err(ContractError::Invalid("workflow trace reference"));
    }
    Ok(WorkflowEventMetadata {
        produced_at: produced_at.to_rfc3339(),
        traceparent: trace_reference.clone(),
    })
}

fn traceparent(value: &str) -> bool {
    let mut parts = value.split('-');
    let version = parts.next();
    let trace_id = parts.next();
    let parent_id = parts.next();
    let flags = parts.next();
    version == Some("00")
        && trace_id
            .is_some_and(|part| canonical_hex(part, 32) && part.bytes().any(|byte| byte != b'0'))
        && parent_id
            .is_some_and(|part| canonical_hex(part, 16) && part.bytes().any(|byte| byte != b'0'))
        && flags.is_some_and(|part| canonical_hex(part, 2))
        && parts.next().is_none()
}

fn canonical_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn validate_application_identity(identity: &EventIdentity) -> Result<(), ContractError> {
    validate_identity(identity)?;
    if identity.source.as_str() != APPLICATION_SOURCE {
        return Err(ContractError::Invalid("workflow application source"));
    }
    Ok(())
}

fn validate_report_identity(
    identity: &EventIdentity,
    payload: &ConversationTurnReport,
) -> Result<(), ContractError> {
    validate_identity(identity)?;
    let expected = format!(
        "ayni://orchestrator/supervisors/{}",
        payload.executor_instance_id
    );
    if identity.source.as_str() != expected {
        return Err(ContractError::Invalid("executor report source"));
    }
    Ok(())
}

fn binding(payload: &ConversationTurnReport) -> Result<&ConversationTurnBinding, ContractError> {
    payload
        .binding
        .as_ref()
        .ok_or(ContractError::Invalid("missing workflow binding"))
}

fn binding_extensions(binding: &ConversationTurnBinding) -> Vec<(&'static str, String)> {
    vec![
        ("workspaceid", binding.workspace_id.clone()),
        ("projectid", binding.project_id.clone()),
        ("conversationid", binding.conversation_id.clone()),
        ("turnid", binding.turn_id.clone()),
        ("assignmentid", binding.assignment_id.clone()),
        (
            "executorgeneration",
            binding.executor_generation.to_string(),
        ),
        ("causationid", binding.command_event_id.clone()),
        ("correlationid", binding.conversation_id.clone()),
    ]
}

fn decision_extensions(payload: &ConversationReportDecision) -> Vec<(&'static str, String)> {
    vec![
        ("workspaceid", payload.workspace_id.clone()),
        ("projectid", payload.project_id.clone()),
        ("conversationid", payload.conversation_id.clone()),
        ("turnid", payload.turn_id.clone()),
        ("assignmentid", payload.assignment_id.clone()),
        (
            "executorgeneration",
            payload.executor_generation.to_string(),
        ),
        ("causationid", payload.report_id.clone()),
        ("correlationid", payload.conversation_id.clone()),
    ]
}

fn validate_binding_extensions(
    event: &Event,
    binding: &ConversationTurnBinding,
) -> Result<(), ContractError> {
    validate_extensions(event, &binding_extensions(binding))
}

fn validate_decision_extensions(
    event: &Event,
    payload: &ConversationReportDecision,
) -> Result<(), ContractError> {
    validate_extensions(event, &decision_extensions(payload))
}

fn validate_extensions(event: &Event, expected: &[(&str, String)]) -> Result<(), ContractError> {
    if expected
        .iter()
        .any(|(name, value)| event.extension(name) != Some(&ExtensionValue::String(value.clone())))
    {
        return Err(ContractError::Invalid("workflow envelope binding"));
    }
    Ok(())
}

fn validate_binding(binding: &ConversationTurnBinding, report: bool) -> Result<(), ContractError> {
    if ![
        &binding.workspace_id,
        &binding.project_id,
        &binding.conversation_id,
        &binding.turn_id,
    ]
    .into_iter()
    .all(|value| resource_identifier(value))
        || !simple_identifier(&binding.assignment_id)
        || binding.executor_generation == 0
        || binding.execution_binding_revision != "scripted-v1"
        || binding.context_revision == 0
        || binding.command_event_id.parse::<uuid::Uuid>().is_err()
        || (report && !digest(&binding.command_digest))
        || (!report && !binding.command_digest.is_empty())
    {
        return Err(ContractError::Invalid("workflow binding"));
    }
    Ok(())
}

fn validate_command<'a>(
    payload: &'a ConversationTurnRequested,
    identity: &EventIdentity,
) -> Result<&'a ConversationTurnBinding, ContractError> {
    let binding = payload
        .binding
        .as_ref()
        .ok_or(ContractError::Invalid("missing workflow binding"))?;
    validate_binding(binding, false)?;
    if binding.command_event_id != identity.id
        || !resource_identifier(&payload.input_message_id)
        || payload.input_content.trim().is_empty()
        || payload.input_content.len() > MAX_CONTENT_BYTES
        || payload.context.len() > 100
        || payload.context.iter().any(|record| {
            !resource_identifier(&record.message_id)
                || record.sequence == 0
                || !matches!(record.role.as_str(), "user" | "assistant")
                || record.content.len() > MAX_CONTENT_BYTES
        })
    {
        return Err(ContractError::Invalid("conversation command"));
    }
    Ok(binding)
}

fn report_kind(payload: &ConversationTurnReport) -> Result<&'static str, ContractError> {
    match ConversationTurnReportKind::try_from(payload.kind)
        .map_err(|_| ContractError::Invalid("report kind"))?
    {
        ConversationTurnReportKind::Started => Ok(TURN_STARTED_TYPE),
        ConversationTurnReportKind::Completed => Ok(TURN_COMPLETED_TYPE),
        ConversationTurnReportKind::Failed => Ok(TURN_FAILED_TYPE),
        ConversationTurnReportKind::Unspecified => Err(ContractError::Invalid("report kind")),
    }
}

fn validate_report(payload: &ConversationTurnReport, signed: bool) -> Result<(), ContractError> {
    let binding = binding(payload)?;
    validate_binding(binding, true)?;
    let kind = ConversationTurnReportKind::try_from(payload.kind)
        .map_err(|_| ContractError::Invalid("report kind"))?;
    let sequence_valid = match kind {
        ConversationTurnReportKind::Started => payload.report_sequence == 1,
        ConversationTurnReportKind::Completed | ConversationTurnReportKind::Failed => {
            payload.report_sequence == 2
        }
        ConversationTurnReportKind::Unspecified => false,
    };
    let outcome_valid = match kind {
        ConversationTurnReportKind::Started => {
            payload.response_content.is_empty()
                && payload.failure_reason.is_empty()
                && payload.signing_public_key.len() == 32
        }
        ConversationTurnReportKind::Completed => {
            !payload.response_content.trim().is_empty()
                && payload.response_content.len() <= MAX_CONTENT_BYTES
                && payload.failure_reason.is_empty()
                && payload.signing_public_key.is_empty()
        }
        ConversationTurnReportKind::Failed => {
            payload.response_content.is_empty()
                && report_failure_reason(&payload.failure_reason)
                && payload.signing_public_key.is_empty()
        }
        ConversationTurnReportKind::Unspecified => false,
    };
    if !sequence_valid
        || !outcome_valid
        || !simple_identifier(&payload.executor_instance_id)
        || !digest(&payload.signing_key_digest)
        || (signed && payload.signature.len() != 64)
        || (!signed && !payload.signature.is_empty())
    {
        return Err(ContractError::Invalid("conversation report"));
    }
    Ok(())
}

fn validate_decision(payload: &ConversationReportDecision) -> Result<(), ContractError> {
    let decision = ConversationReportDecisionKind::try_from(payload.decision)
        .map_err(|_| ContractError::Invalid("decision kind"))?;
    let report_instance = payload
        .report_source
        .strip_prefix("ayni://orchestrator/supervisors/");
    if ![
        &payload.workspace_id,
        &payload.project_id,
        &payload.conversation_id,
        &payload.turn_id,
    ]
    .into_iter()
    .all(|value| resource_identifier(value))
        || !report_instance.is_some_and(simple_identifier)
        || report_instance != Some(payload.target_executor_instance_id.as_str())
        || payload.report_id.parse::<uuid::Uuid>().is_err()
        || !digest(&payload.report_digest)
        || !simple_identifier(&payload.target_executor_instance_id)
        || (!payload.accepted_executor_instance_id.is_empty()
            && !simple_identifier(&payload.accepted_executor_instance_id))
        || (decision == ConversationReportDecisionKind::Accepted
            && payload.accepted_executor_instance_id != payload.target_executor_instance_id)
        || !simple_identifier(&payload.assignment_id)
        || payload.executor_generation == 0
        || decision == ConversationReportDecisionKind::Unspecified
        || !decision_reason_valid(decision, &payload.safe_reason)
        || !matches!(
            payload.turn_state.as_str(),
            "processing" | "completed" | "failed"
        )
    {
        return Err(ContractError::Invalid("report decision"));
    }
    Ok(())
}

fn workflow_subject(binding: &ConversationTurnBinding) -> String {
    format!(
        "conversations/{}/turns/{}",
        binding.conversation_id, binding.turn_id
    )
}

fn lowercase_hexadecimal(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

fn decision_reason_valid(decision: ConversationReportDecisionKind, reason: &str) -> bool {
    match decision {
        ConversationReportDecisionKind::Accepted => reason.is_empty(),
        ConversationReportDecisionKind::Rejected => decision_rejection_reason(reason),
        ConversationReportDecisionKind::Unspecified => false,
    }
}

fn report_failure_reason(value: &str) -> bool {
    matches!(value, "script_failed")
}

fn decision_rejection_reason(value: &str) -> bool {
    matches!(
        value,
        "claim_lost"
            | "stale_generation"
            | "wrong_scope"
            | "wrong_binding"
            | "invalid_signature"
            | "event_conflict"
            | "terminal_conflict"
    )
}

fn simple_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> EventIdentity {
        EventIdentity {
            source: "ayni://orchestrator/supervisors/workflow-application".into(),
            id: uuid::Uuid::now_v7().to_string(),
        }
    }

    fn metadata() -> WorkflowEventMetadata {
        WorkflowEventMetadata {
            produced_at: "2026-10-20T08:00:00+00:00".into(),
            traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into(),
        }
    }

    fn executor_identity(executor_instance_id: &str) -> EventIdentity {
        EventIdentity {
            source: format!("ayni://orchestrator/supervisors/{executor_instance_id}"),
            id: uuid::Uuid::now_v7().to_string(),
        }
    }

    fn binding(event_id: &str) -> ConversationTurnBinding {
        ConversationTurnBinding {
            workspace_id: format!("workspace:{}", uuid::Uuid::now_v7()),
            project_id: format!("project:{}", uuid::Uuid::now_v7()),
            conversation_id: format!("conversation:{}", uuid::Uuid::now_v7()),
            turn_id: format!("turn:{}", uuid::Uuid::now_v7()),
            assignment_id: uuid::Uuid::now_v7().to_string(),
            executor_generation: 1,
            execution_binding_revision: "scripted-v1".into(),
            context_revision: 1,
            command_event_id: event_id.into(),
            command_digest: String::new(),
        }
    }

    #[test]
    fn command_round_trip_preserves_identity_and_scope() -> Result<(), ContractError> {
        let identity = identity();
        let payload = ConversationTurnRequested {
            binding: Some(binding(&identity.id)),
            input_message_id: format!("message:{}", uuid::Uuid::now_v7()),
            input_content: "hello".into(),
            context: vec![],
        };
        let metadata = metadata();
        let bytes = encode_command(&identity, &metadata, &payload)?;
        assert_eq!(decode_command(&bytes)?, (identity, metadata, payload));
        let mut changed_extension: serde_json::Value = serde_json::from_slice(&bytes)?;
        changed_extension["workspaceid"] =
            serde_json::json!(format!("workspace:{}", uuid::Uuid::now_v7()));
        assert!(decode_command(&serde_json::to_vec(&changed_extension)?).is_err());
        let mut missing_time: serde_json::Value = serde_json::from_slice(&bytes)?;
        missing_time
            .as_object_mut()
            .ok_or(ContractError::Invalid("command JSON object"))?
            .remove("time");
        assert!(decode_command(&serde_json::to_vec(&missing_time)?).is_err());
        let mut invalid_trace: serde_json::Value = serde_json::from_slice(&bytes)?;
        invalid_trace["traceparent"] = serde_json::json!("unscoped-trace");
        assert!(decode_command(&serde_json::to_vec(&invalid_trace)?).is_err());
        Ok(())
    }

    #[test]
    fn report_signature_bytes_are_stable_and_signature_is_excluded() -> Result<(), ContractError> {
        let executor_instance_id = uuid::Uuid::now_v7().to_string();
        let identity = executor_identity(&executor_instance_id);
        let mut report_binding = binding(&uuid::Uuid::now_v7().to_string());
        report_binding.command_digest = format!("sha256:{}", "a".repeat(64));
        let signing_seed = [7; 32];
        let public_key = SigningKey::from_bytes(&signing_seed)
            .verifying_key()
            .to_bytes();
        let mut payload = ConversationTurnReport {
            binding: Some(report_binding),
            kind: ConversationTurnReportKind::Started.into(),
            report_sequence: 1,
            executor_instance_id,
            signing_public_key: public_key.to_vec(),
            signing_key_digest: signing_key_digest(&public_key),
            signature: vec![],
            ..Default::default()
        };
        let expected = report_signing_bytes(&identity, &payload)?;
        sign_report(&identity, &mut payload, &signing_seed)?;
        verify_report_signature(&identity, &payload, None)?;
        let metadata = metadata();
        let bytes = encode_report(&identity, &metadata, &payload)?;
        let (actual_identity, actual_metadata, decoded) = decode_report(&bytes)?;
        assert_eq!(actual_identity, identity);
        assert_eq!(actual_metadata, metadata);
        assert_eq!(report_signing_bytes(&identity, &decoded)?, expected);
        Ok(())
    }

    #[test]
    fn tampered_report_signature_is_rejected() -> Result<(), ContractError> {
        let executor_instance_id = uuid::Uuid::now_v7().to_string();
        let identity = executor_identity(&executor_instance_id);
        let signing_seed = [11; 32];
        let public_key = SigningKey::from_bytes(&signing_seed)
            .verifying_key()
            .to_bytes();
        let mut report_binding = binding(&uuid::Uuid::now_v7().to_string());
        report_binding.command_digest = format!("sha256:{}", "d".repeat(64));
        let mut payload = ConversationTurnReport {
            binding: Some(report_binding),
            kind: ConversationTurnReportKind::Started.into(),
            report_sequence: 1,
            executor_instance_id,
            signing_public_key: public_key.to_vec(),
            signing_key_digest: signing_key_digest(&public_key),
            ..Default::default()
        };
        sign_report(&identity, &mut payload, &signing_seed)?;
        payload.executor_instance_id.push_str("-tampered");
        assert!(verify_report_signature(&identity, &payload, None).is_err());
        Ok(())
    }

    #[test]
    fn weak_report_public_key_is_rejected() -> Result<(), ContractError> {
        let mut weak_public_key = [0; 32];
        weak_public_key[0] = 1;
        let parsed = VerifyingKey::from_bytes(&weak_public_key)
            .map_err(|_| ContractError::Invalid("weak-key test fixture"))?;
        assert!(parsed.is_weak());
        assert!(strict_verifying_key(&weak_public_key).is_err());
        Ok(())
    }

    #[test]
    fn shared_signing_fixture_matches_canonical_bytes() -> Result<(), Box<dyn std::error::Error>> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Fixture {
            identity: EventIdentity,
            payload: ConversationTurnReport,
            canonical_hex: String,
            digest_hex: String,
            public_key_hex: String,
        }

        let fixture: Fixture = serde_json::from_str(include_str!(
            "../../../../proto/testdata/workflow/report-signing.json"
        ))?;
        assert_eq!(
            hexadecimal(&report_signing_bytes(&fixture.identity, &fixture.payload)?),
            fixture.canonical_hex
        );
        assert_eq!(
            hexadecimal(&report_signing_digest(&fixture.identity, &fixture.payload)?),
            fixture.digest_hex
        );
        let public_key: [u8; 32] = decode_hexadecimal(&fixture.public_key_hex)?
            .try_into()
            .map_err(|_| "invalid public key length")?;
        verify_report_signature(&fixture.identity, &fixture.payload, Some(&public_key))?;
        Ok(())
    }

    fn hexadecimal(bytes: &[u8]) -> String {
        lowercase_hexadecimal(bytes)
    }

    fn decode_hexadecimal(value: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        if !value.len().is_multiple_of(2) {
            return Err("odd hexadecimal length".into());
        }
        (0..value.len())
            .step_by(2)
            .map(|index| Ok(u8::from_str_radix(&value[index..index + 2], 16)?))
            .collect()
    }

    #[test]
    fn safe_reason_allowlists_reject_free_form_diagnostics() {
        assert!(report_failure_reason("script_failed"));
        assert!(!report_failure_reason("token=secret"));
        assert!(!report_failure_reason("raw_exception_text"));
        assert!(!report_failure_reason(&"a".repeat(64)));
        for reason in [
            "claim_lost",
            "stale_generation",
            "wrong_scope",
            "wrong_binding",
            "invalid_signature",
            "event_conflict",
            "terminal_conflict",
        ] {
            assert!(decision_reason_valid(
                ConversationReportDecisionKind::Rejected,
                reason
            ));
        }
        assert!(!decision_reason_valid(
            ConversationReportDecisionKind::Rejected,
            "credential_leaked_in_raw_diagnostic"
        ));
        assert!(decision_reason_valid(
            ConversationReportDecisionKind::Accepted,
            ""
        ));
        assert!(!decision_reason_valid(
            ConversationReportDecisionKind::Accepted,
            "accepted"
        ));
    }

    #[test]
    fn decisions_are_targeted_and_bounded() -> Result<(), ContractError> {
        let identity = identity();
        let target_executor_instance_id = uuid::Uuid::now_v7().to_string();
        let payload = ConversationReportDecision {
            report_source: format!("ayni://orchestrator/supervisors/{target_executor_instance_id}"),
            report_id: uuid::Uuid::now_v7().to_string(),
            report_digest: format!("sha256:{}", "c".repeat(64)),
            target_executor_instance_id: target_executor_instance_id.clone(),
            accepted_executor_instance_id: target_executor_instance_id,
            assignment_id: uuid::Uuid::now_v7().to_string(),
            executor_generation: 1,
            decision: ConversationReportDecisionKind::Accepted.into(),
            safe_reason: String::new(),
            turn_state: "processing".into(),
            workspace_id: format!("workspace:{}", uuid::Uuid::now_v7()),
            project_id: format!("project:{}", uuid::Uuid::now_v7()),
            conversation_id: format!("conversation:{}", uuid::Uuid::now_v7()),
            turn_id: format!("turn:{}", uuid::Uuid::now_v7()),
        };
        let metadata = metadata();
        let bytes = encode_decision(&identity, &metadata, &payload)?;
        assert_eq!(decode_decision(&bytes)?, (identity, metadata, payload));
        Ok(())
    }
}
