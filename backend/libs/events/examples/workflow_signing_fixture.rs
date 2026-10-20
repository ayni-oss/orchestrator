use ayni_orchestrator_events::EventIdentity;
use ayni_orchestrator_events::v1::{
    ConversationTurnBinding, ConversationTurnReport, ConversationTurnReportKind,
};
use ayni_orchestrator_events::workflow::{
    report_signing_bytes, report_signing_digest, sign_report, signing_key_digest,
};
use ed25519_dalek::SigningKey;
use serde_json::json;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let identity = EventIdentity {
        source: "ayni://orchestrator/supervisors/018f0f7b-6a4d-7a11-8d77-010203040506".into(),
        id: "018f0f7b-6a4d-7a11-8d77-111213141516".into(),
    };
    let signing_seed = [7_u8; 32];
    let public_key = SigningKey::from_bytes(&signing_seed)
        .verifying_key()
        .to_bytes();
    let mut payload = ConversationTurnReport {
        binding: Some(ConversationTurnBinding {
            workspace_id: "workspace:018f0f7b-6a4d-7a11-8d77-212223242526".into(),
            project_id: "project:018f0f7b-6a4d-7a11-8d77-313233343536".into(),
            conversation_id: "conversation:018f0f7b-6a4d-7a11-8d77-414243444546".into(),
            turn_id: "turn:018f0f7b-6a4d-7a11-8d77-515253545556".into(),
            assignment_id: "018f0f7b-6a4d-7a11-8d77-616263646566".into(),
            executor_generation: 1,
            execution_binding_revision: "scripted-v1".into(),
            context_revision: 3,
            command_event_id: "018f0f7b-6a4d-7a11-8d77-717273747576".into(),
            command_digest: format!("sha256:{}", "ab".repeat(32)),
        }),
        kind: ConversationTurnReportKind::Started.into(),
        report_sequence: 1,
        executor_instance_id: "018f0f7b-6a4d-7a11-8d77-010203040506".into(),
        signing_public_key: public_key.to_vec(),
        signing_key_digest: signing_key_digest(&public_key),
        ..Default::default()
    };
    let canonical = report_signing_bytes(&identity, &payload)?;
    let digest = report_signing_digest(&identity, &payload)?;
    sign_report(&identity, &mut payload, &signing_seed)?;
    let fixture = json!({
        "identity": identity,
        "payload": payload,
        "canonicalHex": hexadecimal(&canonical),
        "digestHex": hexadecimal(&digest),
        "publicKeyHex": hexadecimal(&public_key),
    });
    println!("{}", serde_json::to_string_pretty(&fixture)?);
    Ok(())
}

fn hexadecimal(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}
