//! Accepted runtime operation facts; emitted only by a committed controller decision.
use crate::{
    ContractError, EventIdentity, MAX_EVENT_BYTES, identifier, operations,
    v1::RuntimeOperationDecisionRecorded, validate_identity,
};
use cloudevents::event::SpecVersion;
use cloudevents::{AttributesReader, Data, Event, EventBuilder, EventBuilderV10};
pub const EVENT_TYPE: &str = "ayni.build.orchestrator.runtime.operation.accepted.v1";
pub const DATA_SCHEMA: &str = "ayni://orchestrator/schemas/runtime-operation-accepted/v1";

fn validate(
    identity: &EventIdentity,
    decision: &RuntimeOperationDecisionRecorded,
) -> Result<String, ContractError> {
    if !identity
        .source
        .strip_prefix("ayni://orchestrator/controllers/")
        .is_some_and(identifier)
        || uuid::Uuid::parse_str(&identity.id).is_err()
        || identity.id.len() != 36
        || decision.aggregate_revision == 0
    {
        return Err(ContractError::Invalid("operation decision identity"));
    }
    let observation = decision
        .observation
        .as_ref()
        .ok_or(ContractError::Invalid("missing observation"))?;
    let causation = EventIdentity {
        source: decision.observation_source.clone(),
        id: decision.observation_id.clone(),
    };
    validate_identity(&causation)?;
    operations::encode(&causation, observation)?;
    let b = observation
        .binding
        .as_ref()
        .ok_or(ContractError::Invalid("binding"))?;
    Ok(format!(
        "repositories/{}/runtime-profiles/{}",
        b.repository_connection_id, b.runtime_profile_id
    ))
}
/// Encode an accepted fact with its durable controller identity.
/// # Errors
/// Rejects invalid observations, incomplete causation and oversized envelopes.
pub fn encode(
    identity: &EventIdentity,
    decision: &RuntimeOperationDecisionRecorded,
) -> Result<Vec<u8>, ContractError> {
    let subject = validate(identity, decision)?;
    let e = EventBuilderV10::new()
        .id(identity.id.clone())
        .source(identity.source.clone())
        .ty(EVENT_TYPE)
        .subject(subject)
        .data_with_schema(
            "application/json",
            DATA_SCHEMA,
            serde_json::to_value(decision)?,
        )
        .build()
        .map_err(|_| ContractError::Invalid("operation decision envelope"))?;
    let bytes = serde_json::to_vec(&e)?;
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    Ok(bytes)
}
/// Decode an accepted fact; delivery adds no state-transition authority.
/// # Errors
/// Rejects incompatible envelopes and invalid controller causation.
pub fn decode(
    bytes: &[u8],
) -> Result<(EventIdentity, RuntimeOperationDecisionRecorded), ContractError> {
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    let e: Event = serde_json::from_slice(bytes)?;
    if e.specversion() != SpecVersion::V10
        || !crate::event_types::matches(e.ty(), EVENT_TYPE)
        || e.datacontenttype() != Some("application/json")
        || e.dataschema().is_none_or(|s| s.as_str() != DATA_SCHEMA)
    {
        return Err(ContractError::Invalid("operation decision envelope"));
    }
    let identity = EventIdentity {
        source: e.source().clone(),
        id: e.id().into(),
    };
    let Some(Data::Json(data)) = e.data() else {
        return Err(ContractError::Invalid("operation decision data"));
    };
    let decision = serde_json::from_value(data.clone())?;
    if e.subject() != Some(validate(&identity, &decision)?.as_str()) {
        return Err(ContractError::Invalid("operation decision subject"));
    }
    Ok((identity, decision))
}
