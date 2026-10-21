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

#[path = "workflow/workflow_hex.rs"]
mod workflow_hex;
use workflow_hex::{canonical_hex, lowercase_hexadecimal};

pub mod transport;

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
pub const MAX_CLAIM_BYTES: usize = 16 * 1024;
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
            maximum: if kind == TURN_STARTED_TYPE {
                MAX_CLAIM_BYTES
            } else {
                MAX_REPORT_BYTES
            },
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
    if kind == TURN_STARTED_TYPE && bytes.len() > MAX_CLAIM_BYTES {
        return Err(ContractError::Oversized);
    }
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

fn provider_counters_valid(call: &super::v1::ProviderCallEvidence) -> bool {
    if [call.input_tokens, call.output_tokens, call.total_tokens]
        .iter()
        .flatten()
        .any(|value| *value > i64::MAX.cast_unsigned())
    {
        return false;
    }
    if call.total_tokens.is_some_and(|total| {
        call.input_tokens.is_some_and(|input| input > total)
            || call.output_tokens.is_some_and(|output| output > total)
    }) {
        return false;
    }
    !matches!((call.input_tokens,call.output_tokens,call.total_tokens),(Some(input),Some(output),Some(total)) if input.checked_add(output)!=Some(total))
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
        || !matches!(
            binding.execution_binding_revision.as_str(),
            "scripted-v1" | "openai-responses-v1"
        )
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
    validate_execution_configuration(payload)?;
    if let Some(session) = &payload.session
        && (!resource_identifier(&session.session_id)
            || session.generation == 0
            || session.generation != binding.executor_generation
            || !(30..=86_400).contains(&session.idle_timeout_seconds))
    {
        return Err(ContractError::Invalid("session execution scope"));
    }
    if let Some(manifest) = &payload.context_manifest {
        let inline_count =
            u64::try_from(payload.context.len()).map_err(|_| ContractError::Oversized)?;
        if manifest.id != binding.turn_id
            || manifest.record_count == 0
            || manifest.record_count < inline_count
            || manifest.inline_complete != (manifest.record_count == inline_count)
            || manifest.digest.len() != 71
            || !manifest.digest.starts_with("sha256:")
            || !manifest.digest.as_bytes()[7..]
                .iter()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
        {
            return Err(ContractError::Invalid("conversation context manifest"));
        }
    }
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
    validate_provider_evidence(payload, binding, kind)?;
    if !sequence_valid
        || !outcome_valid
        || uuid::Uuid::parse_str(&payload.executor_instance_id).is_err()
        || !digest(&payload.signing_key_digest)
        || (signed && payload.signature.len() != 64)
        || (!signed && !payload.signature.is_empty())
    {
        return Err(ContractError::Invalid("conversation report"));
    }
    Ok(())
}

fn validate_provider_evidence(
    payload: &ConversationTurnReport,
    binding: &ConversationTurnBinding,
    kind: ConversationTurnReportKind,
) -> Result<(), ContractError> {
    let Some(call) = payload.provider_call.as_ref() else {
        if binding.execution_binding_revision == "openai-responses-v1"
            && kind != ConversationTurnReportKind::Started
            && !(kind == ConversationTurnReportKind::Failed
                && matches!(
                    payload.failure_reason.as_str(),
                    "provider_credentials_unavailable" | "provider_input_limit" | "cancelled"
                ))
        {
            return Err(ContractError::Invalid("missing provider evidence"));
        }
        return Ok(());
    };
    if binding.execution_binding_revision != "openai-responses-v1"
        || kind == ConversationTurnReportKind::Started
        || call.call_id != binding.command_event_id
        || call.provider != "openai"
        || call.model.is_empty()
        || call.model.len() > 256
        || !resource_identifier(&call.configuration_revision_id)
        || !digest(&call.context_digest)
        || !digest(&call.request_digest)
        || call.response_id.len() > 256
        || call.reported_model.len() > 256
        || !provider_counters_valid(call)
        || [&call.model, &call.response_id, &call.reported_model]
            .iter()
            .any(|value| value.chars().any(char::is_control))
        || call.reserved_input_tokens == 0
        || call.reserved_output_tokens == 0
        || call.reserved_total_tokens == 0
        || !matches!(
            call.state.as_str(),
            "completed" | "failed" | "uncertain" | "cancelled"
        )
        || (kind == ConversationTurnReportKind::Completed
            && (call.state != "completed"
                || call.response_id.is_empty()
                || call.reported_model.is_empty()))
        || (call.state == "uncertain" && payload.failure_reason != "provider_uncertain")
    {
        return Err(ContractError::Invalid("provider evidence"));
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
            "queued" | "processing" | "completed" | "failed"
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

fn decision_reason_valid(decision: ConversationReportDecisionKind, reason: &str) -> bool {
    match decision {
        ConversationReportDecisionKind::Accepted => reason.is_empty(),
        ConversationReportDecisionKind::Rejected => decision_rejection_reason(reason),
        ConversationReportDecisionKind::Unspecified => false,
    }
}

fn report_failure_reason(value: &str) -> bool {
    matches!(
        value,
        "script_failed"
            | "provider_uncertain"
            | "provider_rejected"
            | "provider_input_limit"
            | "provider_output_limit"
            | "provider_credentials_unavailable"
            | "cancelled"
    )
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

fn validate_execution_configuration(
    payload: &ConversationTurnRequested,
) -> Result<(), ContractError> {
    let Some(config) = &payload.frozen_configuration else {
        return Ok(());
    };
    if !resource_identifier(&config.revision_id)
        || config.instructions.len() > MAX_CONTENT_BYTES
        || config
            .instructions
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        || !matches!(config.provider.as_str(), "scripted" | "openai")
        || config.model.is_empty()
        || config.model.len() > 256
        || config.model.trim() != config.model
        || config.model.chars().any(char::is_control)
        || (config.provider == "scripted" && config.model != "scripted")
        || !(1..=1_000_000).contains(&config.max_input_tokens)
        || !(1..=100_000).contains(&config.max_output_tokens)
        || config.max_total_tokens < config.max_input_tokens + config.max_output_tokens
        || config.max_total_tokens > 1_100_000
        || !(30..=86_400).contains(&config.idle_timeout_seconds)
    {
        return Err(ContractError::Invalid("frozen execution configuration"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "workflow/tests.rs"]
mod tests;
