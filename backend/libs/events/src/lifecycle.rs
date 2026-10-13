//! Accepted lifecycle facts; emitted only by a committed controller decision.
use crate::{
    ContractError, EventIdentity, MAX_EVENT_BYTES, identifier, v1::LifecycleDecisionRecorded,
};
use cloudevents::event::SpecVersion;
use cloudevents::{AttributesReader, Data, Event, EventBuilder, EventBuilderV10};
pub const EVENT_TYPE: &str = "build.ayni.orchestrator.lifecycle.accepted.v1";
pub const DATA_SCHEMA: &str = "ayni://orchestrator/schemas/lifecycle-accepted/v1";

fn validate(
    identity: &EventIdentity,
    decision: &LifecycleDecisionRecorded,
) -> Result<String, ContractError> {
    if !identity
        .source
        .strip_prefix("ayni://orchestrator/controllers/")
        .is_some_and(identifier)
        || uuid::Uuid::parse_str(&identity.id).is_err()
        || identity.id.len() != 36
        || !matches!(
            decision.resource_type.as_str(),
            "connection"
                | "connection_deleted"
                | "certificate"
                | "certificate_deleted"
                | "project"
                | "repository_connection"
                | "repository_snapshot"
                | "repository_removed"
                | "runtime_profile_removed"
        )
        || !crate::resource_identifier(&decision.resource_id)
        || decision.aggregate_revision == 0
        || decision.event_cursor == 0
        || !crate::digest(&decision.projection_sha256)
        || decision.status.len() > 128
        || decision.reason.len() > 128
        || decision.active_revision.len() > 128
    {
        return Err(ContractError::Invalid("lifecycle decision identity"));
    }
    Ok(format!(
        "{}/{}",
        decision.resource_type, decision.resource_id
    ))
}
/// Encode an accepted fact with its durable controller identity.
/// # Errors
/// Rejects invalid resources, incomplete projection identity and oversized envelopes.
pub fn encode(
    identity: &EventIdentity,
    decision: &LifecycleDecisionRecorded,
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
        .map_err(|_| ContractError::Invalid("lifecycle decision envelope"))?;
    let bytes = serde_json::to_vec(&e)?;
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    Ok(bytes)
}
/// Decode an accepted fact; delivery adds no state-transition authority.
/// # Errors
/// Rejects incompatible envelopes and invalid controller identity.
pub fn decode(bytes: &[u8]) -> Result<(EventIdentity, LifecycleDecisionRecorded), ContractError> {
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(ContractError::Oversized);
    }
    let e: Event = serde_json::from_slice(bytes)?;
    if e.specversion() != SpecVersion::V10
        || !crate::event_types::matches(e.ty(), EVENT_TYPE)
        || e.datacontenttype() != Some("application/json")
        || e.dataschema().is_none_or(|s| s.as_str() != DATA_SCHEMA)
    {
        return Err(ContractError::Invalid("lifecycle decision envelope"));
    }
    let identity = EventIdentity {
        source: e.source().clone(),
        id: e.id().into(),
    };
    let Some(Data::Json(data)) = e.data() else {
        return Err(ContractError::Invalid("lifecycle decision data"));
    };
    let decision = serde_json::from_value(data.clone())?;
    if e.subject() != Some(validate(&identity, &decision)?.as_str()) {
        return Err(ContractError::Invalid("lifecycle decision subject"));
    }
    Ok((identity, decision))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_facts_bind_controller_revision_cursor_and_projection() -> Result<(), ContractError>
    {
        let identity = EventIdentity {
            source: "ayni://orchestrator/controllers/lifecycle".into(),
            id: uuid::Uuid::now_v7().to_string(),
        };
        let fact = LifecycleDecisionRecorded {
            resource_type: "connection".into(),
            resource_id: "registry".into(),
            aggregate_revision: 2,
            event_cursor: 13,
            projection_sha256: format!("sha256:{}", "a".repeat(64)),
            status: "ready".into(),
            reason: "connection_operation_failed".into(),
            active_revision: "previous-active".into(),
        };
        let bytes = encode(&identity, &fact)?;
        assert_eq!(decode(&bytes)?, (identity.clone(), fact.clone()));
        for (field, replacement) in [
            ("source", "ayni://orchestrator/supervisors/writer"),
            ("subject", "connection/other"),
            ("dataschema", "ayni://unsupported"),
        ] {
            let mut event: serde_json::Value = serde_json::from_slice(&bytes)?;
            event[field] = replacement.into();
            assert!(decode(&serde_json::to_vec(&event)?).is_err());
        }
        let mut oversized = fact.clone();
        oversized.reason = "a".repeat(129);
        assert!(encode(&identity, &oversized).is_err());
        assert!(decode(&vec![b'a'; MAX_EVENT_BYTES + 1]).is_err());
        let mut malformed = fact.clone();
        malformed.event_cursor = 0;
        assert!(encode(&identity, &malformed).is_err());
        let mut malformed = fact;
        malformed.resource_type = "credentials".into();
        assert!(encode(&identity, &malformed).is_err());
        let mut event: serde_json::Value = serde_json::from_slice(&bytes)?;
        event["data"]["credentials"] = "untrusted".into();
        assert!(decode(&serde_json::to_vec(&event)?).is_err());
        event
            .as_object_mut()
            .ok_or(ContractError::Invalid("object"))?
            .remove("data");
        assert!(decode(&serde_json::to_vec(&event)?).is_err());
        Ok(())
    }
}
