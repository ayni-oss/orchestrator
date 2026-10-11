use super::*;
use serde_json::json;

fn status(exit: Option<i32>) -> Result<ContainerStatus, serde_json::Error> {
    serde_json::from_value(
        json!({"name":"verify","image":"runtime","imageID":format!("runtime@sha256:{}", "a".repeat(64)),"ready":false,"restartCount":0,
            "state": exit.map_or_else(|| json!({"running":{}}), |exit| json!({"terminated":{"exitCode":exit}}))
        }),
    )
}
fn record() -> serde_json::Value {
    json!({"version":CONTRACT,"execution":"runtime-test","pod_uid":"pod","request_digest":format!("sha256:{:x}", Sha256::digest(b"request")),
        "outcome":{"status":"report","report":{"quality":{"outcome":"failed","findings":["x".repeat(5000)]}}}})
}

#[test]
fn usable_evidence_is_independent_of_exit_and_repeated_observation()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec(&record())?;
    for exit in [None, Some(0), Some(1)] {
        for _ in 0..2 {
            let Observation::Completed { digest, report } =
                decode(&bytes, "runtime-test", "pod", "request", &status(exit)?)?
            else {
                return Err("evidence missing".into());
            };
            assert_eq!(digest, format!("sha256:{}", "a".repeat(64)));
            assert!(report.len() > 4096);
        }
    }
    Ok(())
}

#[test]
fn stale_conflicting_and_unsupported_records_cannot_supply_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    for field in ["execution", "pod_uid", "request_digest", "version"] {
        let mut value = record();
        value[field] = json!("different");
        assert!(
            decode(
                &serde_json::to_vec(&value)?,
                "runtime-test",
                "pod",
                "request",
                &status(Some(0))?
            )
            .is_err()
        );
    }
    assert!(decode(b"{", "runtime-test", "pod", "request", &status(Some(0))?).is_err());
    assert!(
        decode(
            &vec![b'x'; MAX_RECORD_BYTES + 1],
            "runtime-test",
            "pod",
            "request",
            &status(Some(0))?
        )
        .is_err()
    );
    let mut unknown = record();
    unknown["outcome"] = json!({"status":"failed","reason":"arbitrary private error"});
    assert!(
        decode(
            &serde_json::to_vec(&unknown)?,
            "runtime-test",
            "pod",
            "request",
            &status(Some(1))?
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn failure_classification_survives_failed_container() -> Result<(), Box<dyn std::error::Error>> {
    let mut value = record();
    value["outcome"] = json!({"status":"failed","reason":"evidence_unavailable"});
    assert!(matches!(
        decode(
            &serde_json::to_vec(&value)?,
            "runtime-test",
            "pod",
            "request",
            &status(Some(1))?
        )?,
        Observation::Failed("runtime_evidence_unavailable")
    ));
    Ok(())
}
