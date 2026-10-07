//! Trusted signing certificate operations. Durable state contains public metadata only.
#![allow(
    clippy::single_match_else,
    reason = "credential recovery separates existing immutable revisions from new writes"
)]
use crate::{AppState, coordinators::with_store_async};
use ayni_orchestrator_api::v1;
use ayni_orchestrator_kubernetes::connections::ConnectionSecrets;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use tonic::Status;
mod admin;

fn secrets() -> Result<ConnectionSecrets, Status> {
    let namespace = std::env::var("ORCHESTRATOR_CONNECTION_NAMESPACE")
        .map_err(|_| Status::failed_precondition("certificate_namespace_unconfigured"))?;
    ConnectionSecrets::new(&namespace).map_err(Status::unavailable)
}
fn store_error(_: crate::adapters::sqlite::CleanStoreError) -> Status {
    Status::failed_precondition("certificate_state_conflict")
}
fn validate_name(value: &str) -> Result<(), Status> {
    if value.trim().is_empty() || value.len() > 128 {
        Err(Status::invalid_argument("certificate_name_invalid"))
    } else {
        Ok(())
    }
}
fn validate_key(value: &str) -> Result<(), Status> {
    if value.is_empty()
        || value == "unsigned"
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        Err(Status::invalid_argument("certificate_key_id_invalid"))
    } else {
        Ok(())
    }
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| {
            [
                char::from(DIGITS[usize::from(byte >> 4)]),
                char::from(DIGITS[usize::from(byte & 15)]),
            ]
        })
        .collect()
}
fn public_key(seed: &str) -> Result<String, Status> {
    if seed.len() != 64
        || !seed
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return Err(Status::invalid_argument("certificate_seed_invalid"));
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&seed[index * 2..index * 2 + 2], 16)
            .map_err(|_| Status::invalid_argument("certificate_seed_invalid"))?;
    }
    Ok(hex(&ed25519_dalek::SigningKey::from_bytes(&bytes)
        .verifying_key()
        .to_bytes()))
}
fn seed_value(seed: String, generate: bool) -> Result<String, Status> {
    if generate {
        if !seed.is_empty() {
            return Err(Status::invalid_argument("certificate_seed_mode_invalid"));
        }
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)
            .map_err(|_| Status::unavailable("certificate_randomness_unavailable"))?;
        Ok(hex(&bytes))
    } else {
        public_key(&seed)?;
        Ok(seed)
    }
}
fn fingerprint(parts: &[&str]) -> Vec<u8> {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.len().to_le_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize().to_vec()
}
impl AppState {
    pub(crate) async fn certificates(&self) -> Result<Vec<v1::Certificate>, Status> {
        with_store_async(self, |store| {
            let mut values = store.list_certificates()?;
            for value in &mut values {
                value.usage_count = store.certificate_runtime_usage(&value.id)?;
            }
            Ok(values)
        })
        .await
        .map_err(store_error)
    }
    pub(crate) async fn certificate(&self, id: String) -> Result<v1::Certificate, Status> {
        with_store_async(self, move |store| {
            let mut value = store.get_certificate(&id)?;
            value.usage_count = store.certificate_runtime_usage(&id)?;
            Ok(value)
        })
        .await
        .map_err(store_error)
    }
    async fn replay(
        &self,
        key: String,
        id: String,
        operation: String,
        version: u64,
        hash: Vec<u8>,
    ) -> Result<bool, Status> {
        ayni_orchestrator_api::validation::idempotency_key(&key)
            .map_err(|_| Status::invalid_argument("idempotency_key_invalid"))?;
        with_store_async(self, move |store| {
            store.certificate_operation(&key, &id, &operation, version, &hash)
        })
        .await
        .map_err(store_error)
    }
    async fn finish_certificate(
        &self,
        value: v1::Certificate,
        key: String,
        op: &str,
        version: u64,
        hash: Vec<u8>,
    ) -> Result<v1::Certificate, Status> {
        let saved = value.clone();
        let op = op.to_owned();
        with_store_async(self, move |store| {
            store.certificate_commit(&saved, &key, &op, version, &hash)?;
            store.invalidate_certificate_runtime_profiles()?;
            Ok(())
        })
        .await
        .map_err(store_error)?;
        Ok(value)
    }
    async fn revision_secret(
        &self,
        id: &str,
        name: &str,
        key_id: &str,
        seed: String,
        generate: bool,
    ) -> Result<(String, String, String), Status> {
        let api = secrets()?;
        // A deterministic operation Secret recovers generated keys after a process crash.
        match api.signing(name, Some((id, ""))).await {
            Ok(existing) => {
                validate_key(&existing.key_id)?;
                if existing.key_id != key_id
                    || (!generate && public_key(&existing.seed)? != public_key(&seed)?)
                {
                    return Err(Status::already_exists("certificate_operation_conflict"));
                }
                Ok((existing.uid, existing.revision, public_key(&existing.seed)?))
            }
            Err(_) => {
                let seed = seed_value(seed, generate)?;
                let public = public_key(&seed)?;
                let (uid, revision) = api
                    .create(
                        name,
                        id,
                        BTreeMap::from([
                            ("signing-key".into(), seed),
                            ("key-id".into(), key_id.into()),
                        ]),
                        false,
                    )
                    .await
                    .map_err(Status::unavailable)?;
                Ok((uid, revision, public))
            }
        }
    }
    pub(crate) async fn create_certificate(
        &self,
        request: v1::CreateCertificateRequest,
    ) -> Result<v1::Certificate, Status> {
        let _guard = self.connection_lock.lock().await;
        validate_name(&request.display_name)?;
        validate_key(&request.key_id)?;
        let id = format!(
            "certificate-{}",
            &hex(&Sha256::digest(request.idempotency_key.as_bytes()))[..32]
        );
        let public = if request.generate {
            if !request.signing_seed.is_empty() {
                return Err(Status::invalid_argument("certificate_seed_mode_invalid"));
            }
            String::new()
        } else {
            public_key(&request.signing_seed)?
        };
        let hash = fingerprint(&[
            &request.display_name,
            &request.key_id,
            &public,
            if request.generate {
                "generate"
            } else {
                "import"
            },
        ]);
        if self
            .replay(
                request.idempotency_key.clone(),
                id.clone(),
                "create".into(),
                0,
                hash.clone(),
            )
            .await?
        {
            return self.certificate(id).await;
        }
        let name = format!("{id}-1");
        let (uid, revision, public_key) = self
            .revision_secret(
                &id,
                &name,
                &request.key_id,
                request.signing_seed,
                request.generate,
            )
            .await?;
        let value = v1::Certificate {
            id,
            management_source: "ui".into(),
            display_name: request.display_name,
            key_id: request.key_id,
            public_key,
            secret_name: name,
            secret_uid: uid,
            revision,
            enabled: true,
            is_default: !self.certificates().await?.iter().any(|c| c.is_default),
            resource_version: 1,
            status: "ready".into(),
            ..Default::default()
        };
        self.finish_certificate(value, request.idempotency_key, "create", 0, hash)
            .await
    }
    async fn mutable_certificate(
        &self,
        id: String,
        expected: u64,
    ) -> Result<v1::Certificate, Status> {
        let value = self.certificate(id).await?;
        if value.management_source != "ui" {
            return Err(Status::permission_denied("certificate_admin_managed"));
        }
        if value.resource_version != expected {
            return Err(Status::aborted("certificate_resource_version_conflict"));
        }
        Ok(value)
    }
    pub(crate) async fn replace_certificate(
        &self,
        request: v1::ReplaceCertificateRequest,
    ) -> Result<v1::Certificate, Status> {
        let _guard = self.connection_lock.lock().await;
        validate_key(&request.key_id)?;
        let public = if request.generate {
            if !request.signing_seed.is_empty() {
                return Err(Status::invalid_argument("certificate_seed_mode_invalid"));
            }
            String::new()
        } else {
            public_key(&request.signing_seed)?
        };
        let hash = fingerprint(&[
            &request.key_id,
            &public,
            if request.generate {
                "generate"
            } else {
                "import"
            },
        ]);
        if self
            .replay(
                request.idempotency_key.clone(),
                request.id.clone(),
                "replace".into(),
                request.expected_resource_version,
                hash.clone(),
            )
            .await?
        {
            return self.certificate(request.id).await;
        }
        let mut value = self
            .mutable_certificate(request.id, request.expected_resource_version)
            .await?;
        let name = format!(
            "{}-{}",
            value.id,
            &hex(&Sha256::digest(request.idempotency_key.as_bytes()))[..12]
        );
        let (uid, revision, public_key) = self
            .revision_secret(
                &value.id,
                &name,
                &request.key_id,
                request.signing_seed,
                request.generate,
            )
            .await?;
        value
            .retired_secret_revisions
            .push(v1::CertificateSecretRevision {
                secret_name: value.secret_name,
                secret_uid: value.secret_uid,
            });
        value.secret_name = name;
        value.secret_uid = uid;
        value.revision = revision;
        value.public_key = public_key;
        value.key_id = request.key_id;
        value.resource_version += 1;
        value.status = "ready".into();
        value.failure_reason.clear();
        self.finish_certificate(
            value,
            request.idempotency_key,
            "replace",
            request.expected_resource_version,
            hash,
        )
        .await
    }
    pub(crate) async fn mutate_certificate(
        &self,
        id: String,
        key: String,
        version: u64,
        operation: &str,
        text: String,
        enabled: bool,
    ) -> Result<v1::Certificate, Status> {
        let _guard = self.connection_lock.lock().await;
        let hash = fingerprint(&[&text, if enabled { "true" } else { "false" }]);
        if self
            .replay(
                key.clone(),
                id.clone(),
                operation.into(),
                version,
                hash.clone(),
            )
            .await?
        {
            return self.certificate(id).await;
        }
        let mut value = if operation == "default" {
            let value = self.certificate(id).await?;
            if value.resource_version != version {
                return Err(Status::aborted("certificate_resource_version_conflict"));
            }
            value
        } else {
            self.mutable_certificate(id, version).await?
        };
        match operation {
            "rename" => {
                validate_name(&text)?;
                value.display_name = text;
            }
            "enable" => {
                if !enabled && value.is_default {
                    return Err(Status::failed_precondition("certificate_default_required"));
                }
                value.enabled = enabled;
            }
            "default" => {
                if !value.enabled || value.status != "ready" {
                    return Err(Status::failed_precondition("certificate_unavailable"));
                }
                value.is_default = true;
            }
            _ => return Err(Status::invalid_argument("certificate_operation_invalid")),
        }
        value.resource_version += 1;
        self.finish_certificate(value, key, operation, version, hash)
            .await
    }
    pub(crate) async fn delete_certificate(
        &self,
        request: v1::DeleteCertificateRequest,
    ) -> Result<(), Status> {
        let _guard = self.connection_lock.lock().await;
        let hash = fingerprint(&[]);
        if self
            .replay(
                request.idempotency_key.clone(),
                request.id.clone(),
                "delete".into(),
                request.expected_resource_version,
                hash.clone(),
            )
            .await?
        {
            return Ok(());
        }
        let value = self
            .mutable_certificate(request.id, request.expected_resource_version)
            .await?;
        if value.is_default || value.usage_count > 0 {
            return Err(Status::failed_precondition("certificate_in_use"));
        }
        let api = secrets()?;
        let mut revisions = value.retired_secret_revisions.clone();
        revisions.push(v1::CertificateSecretRevision {
            secret_name: value.secret_name.clone(),
            secret_uid: value.secret_uid.clone(),
        });
        for revision in &revisions {
            if api
                .in_use(&revision.secret_name)
                .await
                .map_err(Status::unavailable)?
            {
                return Err(Status::failed_precondition("certificate_in_use"));
            }
        }
        for revision in revisions {
            api.delete(&revision.secret_name, &value.id, &revision.secret_uid)
                .await
                .map_err(Status::unavailable)?;
        }
        self.finish_certificate(
            value,
            request.idempotency_key,
            "delete",
            request.expected_resource_version,
            hash,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derives_public_key_from_rfc8032_seed() -> Result<(), Status> {
        assert_eq!(
            public_key("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")?,
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        Ok(())
    }
    #[test]
    fn rejects_seed_and_key_id_format() {
        assert!(public_key(&"A".repeat(64)).is_err());
        assert!(public_key(&"0".repeat(63)).is_err());
        assert!(validate_key("key id").is_err());
        assert!(validate_key("unsigned").is_err());
        assert!(validate_key("key-1.test").is_ok());
    }
    #[test]
    fn generates_valid_seed_without_import_material() -> Result<(), Status> {
        let seed = seed_value(String::new(), true)?;
        assert_eq!(public_key(&seed)?.len(), 64);
        assert!(seed_value("0".repeat(64), true).is_err());
        Ok(())
    }
}
