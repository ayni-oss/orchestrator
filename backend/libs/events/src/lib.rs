//! Verification wire contracts. Successful decoding conveys no admission authority.
use cloudevents::event::{ExtensionValue, SpecVersion};
use cloudevents::{AttributesReader, Data, Event, EventBuilder, EventBuilderV10};
use v1::{
    VerificationEvidenceReference, VerificationExecutionBinding, VerificationFailureCode,
    VerificationResultAvailable, verification_result_available::Outcome,
};

/// Generated Protobuf and ProtoJSON bindings.
#[allow(clippy::all, clippy::pedantic, clippy::nursery)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/ayni.orchestrator.events.v1.rs"));
    include!(concat!(
        env!("OUT_DIR"),
        "/ayni.orchestrator.events.v1.serde.rs"
    ));
}

pub mod coding;
pub mod coding_decisions;
pub mod decisions;
pub mod event_types;
pub mod operations;

pub const EVENT_TYPE: &str = "ayni.build.orchestrator.runtime.verification.result.available.v1";
pub const DATA_SCHEMA: &str = "ayni://orchestrator/schemas/runtime-verification-result/v1";
pub const MAX_EVENT_BYTES: usize = 16_384;
pub const MAX_EVIDENCE_BYTES: u64 = 131_072;

/// Durable producer identity, supplied by the supervisor and reused for delivery retries.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventIdentity {
    pub source: String,
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("verification event exceeds the wire size limit")]
    Oversized,
    #[error("invalid verification event: {0}")]
    Invalid(&'static str),
    #[error("invalid verification JSON")]
    Json(#[from] serde_json::Error),
}

/// Encode an observation using a previously persisted identity.
///
/// # Errors
/// Returns an error for an unsupported or incomplete contract or oversized encoding.
pub fn encode(
    identity: &EventIdentity,
    payload: &VerificationResultAvailable,
) -> Result<Vec<u8>, ContractError> {
    validate_identity(identity)?;
    let binding = validate_payload(payload)?;
    let event = EventBuilderV10::new()
        .id(identity.id.clone())
        .source(identity.source.clone())
        .ty(EVENT_TYPE)
        .subject(subject(binding))
        .extension("operationid", binding.preparation_id.clone())
        .extension("executionid", binding.execution_id.clone())
        .data_with_schema(
            "application/json",
            DATA_SCHEMA,
            serde_json::to_value(payload)?,
        )
        .build()
        .map_err(|_| ContractError::Invalid("CloudEvents envelope"))?;
    let bytes = serde_json::to_vec(&event)?;
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    Ok(bytes)
}

/// Decode a bounded structured `CloudEvent`. Authentication and current-state checks
/// belong to ingress and the controller, respectively.
///
/// # Errors
/// Rejects unsupported context, invalid bindings, outcomes, evidence or identity.
pub fn decode(bytes: &[u8]) -> Result<(EventIdentity, VerificationResultAvailable), ContractError> {
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    let event: Event = serde_json::from_slice(bytes)?;
    if event.specversion() != SpecVersion::V10
        || !crate::event_types::matches(event.ty(), EVENT_TYPE)
        || event.datacontenttype() != Some("application/json")
        || event
            .dataschema()
            .is_none_or(|schema| schema.as_str() != DATA_SCHEMA)
    {
        return Err(ContractError::Invalid("unsupported envelope contract"));
    }
    let identity = EventIdentity {
        source: event.source().clone(),
        id: event.id().to_owned(),
    };
    validate_identity(&identity)?;
    let Some(Data::Json(data)) = event.data() else {
        return Err(ContractError::Invalid("structured JSON data required"));
    };
    let payload: VerificationResultAvailable = serde_json::from_value(data.clone())?;
    let binding = validate_payload(&payload)?;
    if event.subject() != Some(subject(binding).as_str())
        || event.extension("operationid")
            != Some(&ExtensionValue::String(binding.preparation_id.clone()))
        || event.extension("executionid")
            != Some(&ExtensionValue::String(binding.execution_id.clone()))
    {
        return Err(ContractError::Invalid("envelope binding mismatch"));
    }
    Ok((identity, payload))
}

fn subject(binding: &VerificationExecutionBinding) -> String {
    format!(
        "repositories/{}/runtime-profiles/{}",
        binding.repository_connection_id, binding.runtime_profile_id
    )
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn resource_identifier(value: &str) -> bool {
    identifier(value)
        || (value.len() <= 128
            && value.split_once(':').is_some_and(|(prefix, id)| {
                identifier(prefix) && id.len() == 36 && uuid::Uuid::parse_str(id).is_ok()
            }))
}

fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn validate_identity(identity: &EventIdentity) -> Result<(), ContractError> {
    let source_id = identity
        .source
        .strip_prefix("ayni://orchestrator/supervisors/");
    if !source_id.is_some_and(identifier)
        || uuid::Uuid::parse_str(&identity.id).is_err()
        || identity.id.len() != 36
    {
        return Err(ContractError::Invalid("producer identity"));
    }
    Ok(())
}

fn validate_evidence(evidence: &VerificationEvidenceReference) -> Result<(), ContractError> {
    if !identifier(&evidence.evidence_id)
        || !digest(&evidence.sha256)
        || evidence.size_bytes == 0
        || evidence.size_bytes > MAX_EVIDENCE_BYTES
        || evidence.contract != "retained-v1"
    {
        return Err(ContractError::Invalid("retained evidence reference"));
    }
    Ok(())
}

fn validate_payload(
    payload: &VerificationResultAvailable,
) -> Result<&VerificationExecutionBinding, ContractError> {
    let binding = payload
        .binding
        .as_ref()
        .ok_or(ContractError::Invalid("missing execution binding"))?;
    if ![
        &binding.repository_connection_id,
        &binding.runtime_profile_id,
        &binding.runtime_profile_revision_id,
    ]
    .into_iter()
    .all(|id| resource_identifier(id))
        || ![
            &binding.preparation_id,
            &binding.execution_id,
            &binding.pod_uid,
        ]
        .into_iter()
        .all(|id| identifier(id))
        || ![
            &binding.request_digest,
            &binding.lock_digest,
            &binding.policy_digest,
            &binding.resolved_image_digest,
        ]
        .into_iter()
        .all(|d| digest(d))
        || binding.source_revision.len() != 40
        || !binding
            .source_revision
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
        || !matches!(binding.platform.as_str(), "linux/amd64" | "linux/arm64")
    {
        return Err(ContractError::Invalid("execution binding"));
    }
    match payload.outcome.as_ref() {
        Some(Outcome::ReportAvailable(report)) => validate_evidence(
            report
                .evidence
                .as_ref()
                .ok_or(ContractError::Invalid("missing report evidence"))?,
        )?,
        Some(Outcome::ExecutionFailed(failure)) => {
            let code = VerificationFailureCode::try_from(failure.code)
                .map_err(|_| ContractError::Invalid("unknown failure code"))?;
            if code == VerificationFailureCode::Unspecified {
                return Err(ContractError::Invalid("unspecified failure code"));
            }
            if let Some(diagnostic) = &failure.diagnostic {
                validate_evidence(diagnostic)?;
            }
        }
        None => return Err(ContractError::Invalid("missing outcome")),
    }
    Ok(binding)
}

pub mod operation_decisions;
