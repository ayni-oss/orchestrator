//! Controller-owned accepted transition contracts, independent from observation receipt.
use crate::{
    ContractError, EventIdentity, MAX_EVENT_BYTES, digest, identifier, resource_identifier,
    v1::VerificationDecisionRecorded,
};
use cloudevents::event::SpecVersion;
use cloudevents::{AttributesReader, Data, Event, EventBuilder, EventBuilderV10};

pub const EVENT_TYPE: &str = "ayni.build.orchestrator.runtime.verification.accepted.v1";
pub const DATA_SCHEMA: &str = "ayni://orchestrator/schemas/runtime-verification-accepted/v1";

fn validate(
    identity: &EventIdentity,
    decision: &VerificationDecisionRecorded,
) -> Result<(), ContractError> {
    if !identity
        .source
        .strip_prefix("ayni://orchestrator/controllers/")
        .is_some_and(identifier)
        || identity.id.len() != 36
        || uuid::Uuid::parse_str(&identity.id).is_err()
        || ![
            &decision.repository_connection_id,
            &decision.runtime_profile_id,
            &decision.runtime_profile_revision_id,
        ]
        .into_iter()
        .all(|id| resource_identifier(id))
        || ![&decision.preparation_id, &decision.execution_id]
            .into_iter()
            .all(|id| identifier(id))
        || decision.aggregate_revision == 0
        || !digest(&decision.report_sha256)
    {
        return Err(ContractError::Invalid("accepted verification identity"));
    }
    match (&decision.observation_source, &decision.observation_id) {
        (Some(source), Some(id)) => crate::validate_identity(&EventIdentity {
            source: source.clone(),
            id: id.clone(),
        })?,
        (None, None) => (),
        _ => return Err(ContractError::Invalid("partial observation causation")),
    }
    Ok(())
}

fn subject(decision: &VerificationDecisionRecorded) -> String {
    format!(
        "repositories/{}/runtime-profiles/{}",
        decision.repository_connection_id, decision.runtime_profile_id
    )
}

/// Encode a controller decision using generated `ProtoJSON` and stable persisted identity.
/// # Errors
/// Rejects invalid authority metadata or oversized envelopes.
pub fn encode(
    identity: &EventIdentity,
    decision: &VerificationDecisionRecorded,
) -> Result<Vec<u8>, ContractError> {
    validate(identity, decision)?;
    let event = EventBuilderV10::new()
        .id(identity.id.clone())
        .source(identity.source.clone())
        .ty(EVENT_TYPE)
        .subject(subject(decision))
        .data_with_schema(
            "application/json",
            DATA_SCHEMA,
            serde_json::to_value(decision)?,
        )
        .build()
        .map_err(|_| ContractError::Invalid("accepted envelope"))?;
    let bytes = serde_json::to_vec(&event)?;
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    Ok(bytes)
}

/// Decode an accepted transition; broker delivery conveys no additional authority.
/// # Errors
/// Rejects unsupported envelopes or invalid controller decisions.
pub fn decode(
    bytes: &[u8],
) -> Result<(EventIdentity, VerificationDecisionRecorded), ContractError> {
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    let event: Event = serde_json::from_slice(bytes)?;
    if event.specversion() != SpecVersion::V10
        || !crate::event_types::matches(event.ty(), EVENT_TYPE)
        || event.datacontenttype() != Some("application/json")
        || event.dataschema().is_none_or(|s| s.as_str() != DATA_SCHEMA)
    {
        return Err(ContractError::Invalid("accepted envelope contract"));
    }
    let identity = EventIdentity {
        source: event.source().clone(),
        id: event.id().into(),
    };
    let Some(Data::Json(data)) = event.data() else {
        return Err(ContractError::Invalid("accepted JSON payload"));
    };
    let decision: VerificationDecisionRecorded = serde_json::from_value(data.clone())?;
    validate(&identity, &decision)?;
    if event.subject() != Some(subject(&decision).as_str()) {
        return Err(ContractError::Invalid("accepted subject"));
    }
    Ok((identity, decision))
}
