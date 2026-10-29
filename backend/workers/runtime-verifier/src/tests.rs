#![allow(clippy::unwrap_used)]
use super::*;
use ayni_core::{EnvironmentCertificate, EnvironmentCertificateTrustPolicy};
use ed25519_dalek::SigningKey;
use std::collections::BTreeMap;

fn certificate() -> (Vec<u8>, EnvironmentCertificateTrustPolicy) {
    let key = SigningKey::from_bytes(&[42_u8; 32]);
    let trust = EnvironmentCertificateTrustPolicy::new(BTreeMap::from([(
        "fixture".into(),
        ayni_core::lower_hex(key.verifying_key().as_bytes()),
    )]))
    .unwrap();
    let payload = EnvironmentCertificate::new(
        format!("sha256:{}", "a".repeat(64)),
        "linux/arm64",
        VERSION,
        format!("sha256:{}", "b".repeat(64)),
        format!("sha256:{}", "c".repeat(64)),
    )
    .unwrap();
    let envelope = EnvironmentCertificateEnvelope::sign(payload, "fixture", &key).unwrap();
    let mut bytes = serde_json::to_vec(&envelope).unwrap();
    bytes.push(b'\n');
    (bytes, trust)
}

#[test]
fn genuine_signature_rejects_modified_claims_and_wrong_trust() {
    let (bytes, trust) = certificate();
    let mut envelope = decode_certificate(&bytes).unwrap();
    envelope.verify(&trust).unwrap();
    envelope.certificate.protected_content_root = format!("sha256:{}", "d".repeat(64));
    assert!(envelope.verify(&trust).is_err());
    let wrong_trust = {
        let key = SigningKey::from_bytes(&[43_u8; 32]);
        EnvironmentCertificateTrustPolicy::new(BTreeMap::from([(
            "fixture".into(),
            ayni_core::lower_hex(key.verifying_key().as_bytes()),
        )]))
        .unwrap()
    };
    assert!(
        decode_certificate(&bytes)
            .unwrap()
            .verify(&wrong_trust)
            .is_err()
    );
}

#[test]
fn malformed_noncanonical_and_wrong_schema_metadata_are_rejected() {
    let (bytes, _) = certificate();
    assert_eq!(
        decode_certificate(b"{}").unwrap_err(),
        Reason::CertificateInvalid
    );
    let mut spaced = bytes.clone();
    spaced.push(b'\n');
    assert_eq!(
        decode_certificate(&spaced).unwrap_err(),
        Reason::CertificateInvalid
    );
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["certificate"]["schema_version"] = "99".into();
    assert_eq!(
        decode_certificate(&serde_json::to_vec(&value).unwrap()).unwrap_err(),
        Reason::CertificateInvalid
    );
}

#[test]
fn metadata_reads_reject_oversized_files_and_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("runtime.json");
    let (bytes, _) = certificate();
    fs::write(&path, &bytes).unwrap();
    assert_eq!(bounded(&path, bytes.len() as u64, false).unwrap(), bytes);
    assert_eq!(
        bounded(&path, 1, false).unwrap_err(),
        Reason::InputBoundsExceeded
    );
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(ayni_environment::MAX_CERTIFICATE_BYTES + 1)
        .unwrap();
    assert_eq!(
        bounded(&path, ayni_environment::MAX_CERTIFICATE_BYTES, false).unwrap_err(),
        Reason::InputBoundsExceeded
    );
    #[cfg(unix)]
    {
        let link = root.join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(
            bounded(&link, 1024, false).unwrap_err(),
            Reason::InputPathUnsupported
        );
        let directory = root.join("nested");
        std::os::unix::fs::symlink(&root, &directory).unwrap();
        assert_eq!(
            bounded(&directory.join("runtime.json"), 1024, false).unwrap_err(),
            Reason::InputPathUnsupported
        );
    }
}

#[test]
fn executable_is_hashed_without_execution_and_requires_the_trusted_exact_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("never-execute");
    fs::write(&path, b"candidate bytes, not an executable").unwrap();
    let expected = sha256_fingerprint(b"candidate bytes, not an executable");
    assert_eq!(
        executable_digest(&path, &expected, false).unwrap(),
        expected
    );
    fs::write(&path, b"modified candidate").unwrap();
    assert_eq!(
        executable_digest(&path, &expected, false).unwrap_err(),
        Reason::CliDigestMismatch
    );
}

#[test]
fn trusted_cli_identity_is_required_and_unknown_flags_are_rejected() {
    assert_eq!(
        Options::parse(Vec::new()).unwrap_err(),
        Reason::ExpectedCliDigestRequired
    );
    let args = [
        "--expected-cli-sha256".into(),
        format!("sha256:{}", "0".repeat(64)),
    ];
    assert!(Options::parse(args).is_ok());
    assert_eq!(
        Options::parse(["--execute".into(), "candidate".into()]).unwrap_err(),
        Reason::InvalidArguments
    );
}

#[test]
fn real_official_lock_fixture_does_not_grant_portable_trust() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../dev/fixtures/task-quality/source")
        .canonicalize()
        .unwrap();
    let lock = ayni_environment::read_lock(&root).unwrap();
    assert_eq!(lock.ayni_version(), VERSION);
    let evidence = verify(&Options {
        source: root,
        expected_platform: "linux/arm64".into(),
        expected_cli_sha256: format!("sha256:{}", "0".repeat(64)),
    });
    assert_eq!(evidence.state, "invalid");
    assert_eq!(evidence.reason, Reason::TrustRequired);
    assert!(evidence.executable_digest.is_empty());
    assert!(evidence.policy_digest.starts_with("sha256:"));
}

#[test]
fn source_preflight_rejects_oversized_inputs_before_upstream_reads() {
    let temp = tempfile::tempdir().unwrap();
    let file = fs::File::create(temp.path().join("input")).unwrap();
    file.set_len(SOURCE_FILE_BYTES + 1).unwrap();
    assert_eq!(
        bound_source(temp.path()).unwrap_err(),
        Reason::InputBoundsExceeded
    );
}

#[test]
fn official_lock_rejects_changed_frozen_source_bytes() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../dev/fixtures/task-quality/source");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for name in [
        ".ayni.toml",
        ".ayni.lock",
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
    ] {
        fs::copy(fixture.join(name), root.join(name)).unwrap();
    }
    ayni_environment::read_lock(&root).unwrap();
    let mut manifest = fs::read(root.join("Cargo.toml")).unwrap();
    manifest.push(b'\n');
    fs::write(root.join("Cargo.toml"), manifest).unwrap();
    assert!(ayni_environment::read_lock(&root).is_err());
}
