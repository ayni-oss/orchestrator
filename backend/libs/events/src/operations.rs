//! Typed runtime observations, authenticated and accepted outside this codec.
use super::v1::{RuntimeOperationBinding, RuntimeOperationKind, RuntimeOperationResultAvailable};
use super::{
    ContractError, EventIdentity, MAX_EVENT_BYTES, digest, identifier, resource_identifier,
    validate_identity,
};
use cloudevents::event::{ExtensionValue, SpecVersion};
use cloudevents::{AttributesReader, Data, Event, EventBuilder, EventBuilderV10};

pub const EVENT_TYPE: &str = "build.ayni.orchestrator.runtime.operation.result.available.v1";
pub const DATA_SCHEMA: &str = "ayni://orchestrator/schemas/runtime-operation-result/v1";
pub const CONTRACT: &str = "runtime-operation-v1";
pub const MAX_RECORD_BYTES: usize = 2_200_000;

fn validate(
    payload: &RuntimeOperationResultAvailable,
) -> Result<&RuntimeOperationBinding, ContractError> {
    let b = payload
        .binding
        .as_ref()
        .ok_or(ContractError::Invalid("missing operation binding"))?;
    if ![
        &b.repository_connection_id,
        &b.runtime_profile_id,
        &b.runtime_profile_revision_id,
    ]
    .into_iter()
    .all(|v| resource_identifier(v))
        || ![&b.preparation_id, &b.execution_id, &b.pod_uid]
            .into_iter()
            .all(|v| identifier(v))
        || ![&b.request_digest, &b.lock_digest, &b.policy_digest]
            .into_iter()
            .all(|v| digest(v))
        || b.source_revision.len() != 40
        || !b.source_revision.bytes().all(|v| v.is_ascii_hexdigit())
        || !matches!(b.platform.as_str(), "linux/amd64" | "linux/arm64")
        || RuntimeOperationKind::try_from(payload.kind)
            .ok()
            .is_none_or(|k| k == RuntimeOperationKind::Unspecified)
    {
        return Err(ContractError::Invalid("operation binding"));
    }
    let e = payload
        .evidence
        .as_ref()
        .ok_or(ContractError::Invalid("missing operation evidence"))?;
    if e.evidence_id != "runtime-operation-json"
        || !digest(&e.sha256)
        || e.size_bytes == 0
        || e.size_bytes > MAX_RECORD_BYTES as u64
        || e.contract != CONTRACT
    {
        return Err(ContractError::Invalid("operation evidence"));
    }
    Ok(b)
}

fn subject(b: &RuntimeOperationBinding) -> String {
    format!(
        "repositories/{}/runtime-profiles/{}",
        b.repository_connection_id, b.runtime_profile_id
    )
}

/// Encode one observation with its persisted delivery identity.
/// # Errors
/// Rejects unsupported bindings, evidence and oversized envelopes.
pub fn encode(
    identity: &EventIdentity,
    payload: &RuntimeOperationResultAvailable,
) -> Result<Vec<u8>, ContractError> {
    validate_identity(identity)?;
    let b = validate(payload)?;
    let event = EventBuilderV10::new()
        .id(identity.id.clone())
        .source(identity.source.clone())
        .ty(EVENT_TYPE)
        .subject(subject(b))
        .extension("operationid", b.preparation_id.clone())
        .extension("executionid", b.execution_id.clone())
        .data_with_schema(
            "application/json",
            DATA_SCHEMA,
            serde_json::to_value(payload)?,
        )
        .build()
        .map_err(|_| ContractError::Invalid("operation envelope"))?;
    let bytes = serde_json::to_vec(&event)?;
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    Ok(bytes)
}

/// Decode a structurally valid observation. Identity confers no execution authority.
/// # Errors
/// Rejects unknown contracts, missing presence and mismatched envelope metadata.
pub fn decode(
    bytes: &[u8],
) -> Result<(EventIdentity, RuntimeOperationResultAvailable), ContractError> {
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    let e: Event = serde_json::from_slice(bytes)?;
    if e.specversion() != SpecVersion::V10
        || !crate::event_types::matches(e.ty(), EVENT_TYPE)
        || e.datacontenttype() != Some("application/json")
        || e.dataschema().is_none_or(|s| s.as_str() != DATA_SCHEMA)
    {
        return Err(ContractError::Invalid("operation envelope"));
    }
    let identity = EventIdentity {
        source: e.source().clone(),
        id: e.id().into(),
    };
    validate_identity(&identity)?;
    let Some(Data::Json(data)) = e.data() else {
        return Err(ContractError::Invalid("operation data"));
    };
    let payload: RuntimeOperationResultAvailable = serde_json::from_value(data.clone())?;
    let b = validate(&payload)?;
    if e.subject() != Some(subject(b).as_str())
        || e.extension("operationid") != Some(&ExtensionValue::String(b.preparation_id.clone()))
        || e.extension("executionid") != Some(&ExtensionValue::String(b.execution_id.clone()))
    {
        return Err(ContractError::Invalid("operation metadata"));
    }
    Ok((identity, payload))
}
