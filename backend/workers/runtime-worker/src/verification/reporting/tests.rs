use super::*;
use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
};

fn setup(
    endpoint: String,
) -> Result<(tempfile::TempDir, ReportingAdmission), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let (identity, payload) = ayni_orchestrator_events::decode(include_bytes!(
        "../../../../../../proto/testdata/verification/report.json"
    ))?;
    let binding = payload.binding.ok_or("binding")?;
    let record = VerificationRecord {
        version: CONTRACT.into(),
        execution: binding.execution_id.clone(),
        pod_uid: binding.pod_uid.clone(),
        request_digest: binding.request_digest.clone(),
        outcome: VerificationOutcome::Report {
            report: serde_json::json!({"quality":"unavailable"}),
        },
    };
    fs::write(
        directory.path().join(RECORD_FILE),
        serde_json::to_vec(&record)?,
    )?;
    fs::write(directory.path().join("token"), "test-projected-token")?;
    Ok((
        directory,
        ReportingAdmission {
            configuration: ayni_orchestrator_runtime::verification::ReportingConfiguration {
                endpoint,
                source: identity.source,
            },
            binding,
        },
    ))
}

#[tokio::test]
async fn unavailable_ingress_and_lost_acknowledgement_replay_persisted_observation()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!("http://{}/events/verification", listener.local_addr()?);
    let (directory, admission) = setup(endpoint)?;
    let encoded = event(directory.path(), &admission, &admission.binding.pod_uid)?;
    let mut historical: serde_json::Value = serde_json::from_slice(&encoded)?;
    historical["type"] =
        serde_json::json!("io.ayni.orchestrator.runtime.verification.result.available.v1");
    let persisted = serde_json::to_vec(&historical)?;
    // Arrange a record retained by the earlier producer before this process resumes.
    fs::write(directory.path().join(EVENT_FILE), &persisted)?;
    assert_eq!(
        event(directory.path(), &admission, &admission.binding.pod_uid)?,
        persisted
    );
    let expected = ayni_orchestrator_events::event_types::canonicalize(persisted.clone())?;
    let server = std::thread::spawn(move || -> io::Result<()> {
        for attempt in 0..3 {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(10)))?;
            let mut raw = Vec::new();
            let split = loop {
                let mut byte = [0];
                stream.read_exact(&mut byte)?;
                raw.push(byte[0]);
                if raw.ends_with(b"\r\n\r\n") {
                    break raw.len();
                }
                if raw.len() > 16384 {
                    return Err(failed());
                }
            };
            let headers = String::from_utf8_lossy(&raw[..split]);
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-projected-token")
            );
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|v| v.parse::<usize>().ok())
                })
                .ok_or_else(failed)?;
            let mut body = vec![0; length];
            stream.read_exact(&mut body)?;
            assert_eq!(body, expected);
            if attempt == 0 {
                stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
            }
            if attempt == 2 {
                let (identity, _) =
                    ayni_orchestrator_events::decode(&body).map_err(|_| failed())?;
                let receipt = serde_json::to_vec(&ReportingReceipt {
                    source: identity.source,
                    id: identity.id,
                    status: "stored".into(),
                })
                .map_err(|_| failed())?;
                write!(
                    stream,
                    "HTTP/1.1 202 Accepted\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    receipt.len()
                )?;
                stream.write_all(&receipt)?;
            }
            // Attempt 1 deliberately loses the response after the server received the event.
        }
        Ok(())
    });
    deliver(
        directory.path(),
        &admission,
        &admission.binding.pod_uid,
        &directory.path().join("token"),
    )
    .await?;
    server.join().map_err(|_| "server panicked")??;
    assert_eq!(fs::read(directory.path().join(EVENT_FILE))?, persisted);
    // A later supervisor process needs neither ingress nor credentials after durable receipt.
    fs::remove_file(directory.path().join("token"))?;
    deliver(
        directory.path(),
        &admission,
        &admission.binding.pod_uid,
        &directory.path().join("token"),
    )
    .await?;
    Ok(())
}

#[test]
fn changed_admission_or_physical_execution_cannot_reuse_a_persisted_event()
-> Result<(), Box<dyn std::error::Error>> {
    let (directory, mut admission) = setup("http://localhost/events/verification".into())?;
    event(directory.path(), &admission, &admission.binding.pod_uid)?;
    admission.binding.preparation_id = "changed".into();
    assert!(event(directory.path(), &admission, &admission.binding.pod_uid).is_err());
    assert!(event(directory.path(), &admission, "different-pod").is_err());
    Ok(())
}
