use ayni_orchestrator_events::{MAX_EVENT_BYTES, decode, encode};
use serde_json::{Value, json};

const REPORT: &[u8] = include_bytes!("../../../../proto/testdata/verification/report.json");
const FAILURE: &[u8] = include_bytes!("../../../../proto/testdata/verification/failure.json");
const DIAGNOSTIC: &[u8] = include_bytes!("../../../../proto/testdata/verification/diagnostic.json");

#[test]
fn shared_protojson_fixtures_round_trip_and_replay_identity()
-> Result<(), Box<dyn std::error::Error>> {
    for fixture in [REPORT, FAILURE, DIAGNOSTIC] {
        let (identity, payload) = decode(fixture)?;
        let encoded = encode(&identity, &payload)?;
        assert_eq!(decode(&encoded)?, (identity.clone(), payload.clone()));
        assert_eq!(
            serde_json::from_slice::<Value>(&encoded)?,
            serde_json::from_slice::<Value>(fixture)?
        );
        assert_eq!(decode(&encode(&identity, &payload)?)?, (identity, payload));
    }
    Ok(())
}

#[test]
fn rejects_unsupported_context_and_unbound_execution() -> Result<(), Box<dyn std::error::Error>> {
    for (pointer, value) in [
        ("/specversion", json!("0.3")),
        ("/type", json!("other")),
        (
            "/dataschema",
            json!("ayni://orchestrator/schemas/runtime-verification-result/v2"),
        ),
        ("/datacontenttype", json!("text/plain")),
        ("/id", json!("new-id")),
        ("/source", json!("https://untrusted.example")),
        ("/subject", json!("other")),
        ("/operationid", json!("other")),
        ("/executionid", json!(42)),
        ("/data/binding/podUid", json!("")),
        ("/data/binding/requestDigest", json!("sha256:bad")),
        ("/data/binding/sourceRevision", json!("main")),
        ("/data/binding/platform", json!("unknown")),
        (
            "/data/reportAvailable/evidence/evidenceId",
            json!("../../report.json"),
        ),
        ("/data/reportAvailable/evidence/sha256", json!("invalid")),
        ("/data/reportAvailable/evidence/sizeBytes", json!("131073")),
        ("/data/reportAvailable/evidence/contract", json!("unknown")),
        ("/data/reportAvailable/evidence", Value::Null),
        ("/data/binding", Value::Null),
        ("/data", json!("serialized body")),
    ] {
        let mut event: Value = serde_json::from_slice(REPORT)?;
        let slot = event.pointer_mut(pointer).ok_or("missing fixture field")?;
        *slot = value;
        assert!(decode(&serde_json::to_vec(&event)?).is_err(), "{pointer}");
    }
    Ok(())
}

#[test]
fn rejects_unknown_or_unspecified_failure_codes_and_missing_or_multiple_outcomes()
-> Result<(), Box<dyn std::error::Error>> {
    for code in [
        json!(0),
        json!(100),
        json!("VERIFICATION_FAILURE_CODE_UNSPECIFIED"),
        json!("FUTURE_CODE"),
    ] {
        let mut event: Value = serde_json::from_slice(FAILURE)?;
        event["data"]["executionFailed"]["code"] = code;
        assert!(decode(&serde_json::to_vec(&event)?).is_err());
    }
    let mut event: Value = serde_json::from_slice(REPORT)?;
    event["data"]["executionFailed"] = json!({"code": 1});
    assert!(decode(&serde_json::to_vec(&event)?).is_err());
    event["data"] = json!({"binding": event["data"]["binding"]});
    assert!(decode(&serde_json::to_vec(&event)?).is_err());
    Ok(())
}

#[test]
fn protojson_accepts_known_numeric_enum_and_preserves_absent_diagnostics()
-> Result<(), Box<dyn std::error::Error>> {
    let mut event: Value = serde_json::from_slice(FAILURE)?;
    event["data"]["executionFailed"]["code"] = json!(6);
    let (identity, payload) = decode(&serde_json::to_vec(&event)?)?;
    let encoded: Value = serde_json::from_slice(&encode(&identity, &payload)?)?;
    assert!(
        encoded["data"]["executionFailed"]
            .get("diagnostic")
            .is_none()
    );
    assert_eq!(
        encoded["data"]["executionFailed"]["code"],
        "VERIFICATION_FAILURE_CODE_EVIDENCE_UNAVAILABLE"
    );
    Ok(())
}

#[test]
fn bounds_untrusted_json_before_parsing() {
    assert!(decode(&vec![b' '; MAX_EVENT_BYTES + 1]).is_err());
    assert!(decode(b"{").is_err());
    assert!(decode(b"{}").is_err());
}

#[test]
fn rejects_binary_data_unknown_payload_fields_and_oversized_extensions()
-> Result<(), Box<dyn std::error::Error>> {
    let mut event: Value = serde_json::from_slice(REPORT)?;
    event["data"]["accepted"] = json!(true);
    assert!(decode(&serde_json::to_vec(&event)?).is_err());
    let mut event: Value = serde_json::from_slice(REPORT)?;
    event
        .as_object_mut()
        .ok_or("expected object")?
        .remove("data");
    event["data_base64"] = json!("e30=");
    assert!(decode(&serde_json::to_vec(&event)?).is_err());
    let mut event: Value = serde_json::from_slice(REPORT)?;
    event["diagnostic"] = json!("x".repeat(MAX_EVENT_BYTES));
    assert!(decode(&serde_json::to_vec(&event)?).is_err());
    Ok(())
}

#[test]
fn encoding_enforces_the_same_contract_as_decoding() -> Result<(), Box<dyn std::error::Error>> {
    let (mut identity, mut payload) = decode(REPORT)?;
    identity.id.clear();
    assert!(encode(&identity, &payload).is_err());
    let (identity, _) = decode(REPORT)?;
    payload.outcome = None;
    assert!(encode(&identity, &payload).is_err());
    Ok(())
}
