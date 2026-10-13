//! Typed runtime observations, authenticated and accepted outside this codec.
use super::v1::{CodingExecutionBinding, CodingReportAvailable, CodingReportKind};
use super::{
    ContractError, EventIdentity, MAX_EVENT_BYTES, digest, identifier, resource_identifier,
    validate_identity,
};
use cloudevents::event::{ExtensionValue, SpecVersion};
use cloudevents::{AttributesReader, Data, Event, EventBuilder, EventBuilderV10};

pub const EVENT_TYPE: &str = "build.ayni.orchestrator.coding.report.available.v1";
pub const DATA_SCHEMA: &str = "ayni://orchestrator/schemas/coding-report/v1";
pub const CONTRACT: &str = "coding-report-v1";
pub const MAX_RECORD_BYTES: usize = 524_288;

fn validate(payload: &CodingReportAvailable) -> Result<&CodingExecutionBinding, ContractError> {
    let b = payload
        .binding
        .as_ref()
        .ok_or(ContractError::Invalid("missing operation binding"))?;
    if ![&b.task_id, &b.attempt_id]
        .into_iter()
        .all(|v| resource_identifier(v))
        || ![&b.generation, &b.execution_id, &b.pod_uid]
            .into_iter()
            .all(|v| identifier(v))
        || ![&b.snapshot_digest, &b.request_digest]
            .into_iter()
            .all(|v| digest(v))
        || CodingReportKind::try_from(payload.kind)
            .ok()
            .is_none_or(|k| k == CodingReportKind::Unspecified)
        || payload.sequence == 0
        || payload.sequence > i64::MAX as u64
        || !digest(&payload.sha256)
        || payload.size_bytes == 0
        || payload.size_bytes > MAX_RECORD_BYTES as u64
        || payload.contract != CONTRACT
    {
        return Err(ContractError::Invalid("coding report"));
    }
    Ok(b)
}

fn subject(b: &CodingExecutionBinding) -> String {
    format!("tasks/{}/attempts/{}", b.task_id, b.attempt_id)
}

/// Encode one observation with its persisted delivery identity.
/// # Errors
/// Rejects unsupported bindings, evidence and oversized envelopes.
pub fn encode(
    identity: &EventIdentity,
    payload: &CodingReportAvailable,
) -> Result<Vec<u8>, ContractError> {
    validate_identity(identity)?;
    let b = validate(payload)?;
    let event = EventBuilderV10::new()
        .id(identity.id.clone())
        .source(identity.source.clone())
        .ty(EVENT_TYPE)
        .subject(subject(b))
        .extension("operationid", b.generation.clone())
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
pub fn decode(bytes: &[u8]) -> Result<(EventIdentity, CodingReportAvailable), ContractError> {
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
    let payload: CodingReportAvailable = serde_json::from_value(data.clone())?;
    let b = validate(&payload)?;
    if e.subject() != Some(subject(b).as_str())
        || e.extension("operationid") != Some(&ExtensionValue::String(b.generation.clone()))
        || e.extension("executionid") != Some(&ExtensionValue::String(b.execution_id.clone()))
    {
        return Err(ContractError::Invalid("operation metadata"));
    }
    Ok((identity, payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_protojson_fixture_roundtrips_and_unknown_kinds_and_metadata_fail_closed()
    -> Result<(), ContractError> {
        let bytes = include_bytes!("../../../../proto/testdata/coding/final.json");
        let (identity, payload) = decode(bytes)?;
        assert_eq!(decode(&encode(&identity, &payload)?)?, (identity, payload));
        let mut bad: serde_json::Value = serde_json::from_slice(bytes)?;
        bad["data"]["kind"] = serde_json::json!(99);
        assert!(decode(&serde_json::to_vec(&bad)?).is_err());
        let mut bad: serde_json::Value = serde_json::from_slice(bytes)?;
        bad["operationid"] = serde_json::json!("another-generation");
        assert!(decode(&serde_json::to_vec(&bad)?).is_err());
        let mut bad: serde_json::Value = serde_json::from_slice(bytes)?;
        bad["data"]
            .as_object_mut()
            .ok_or(ContractError::Invalid("data"))?
            .remove("binding");
        assert!(decode(&serde_json::to_vec(&bad)?).is_err());
        Ok(())
    }
}
