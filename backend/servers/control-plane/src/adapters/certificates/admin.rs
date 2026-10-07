#![allow(
    clippy::collapsible_if,
    clippy::single_match_else,
    reason = "administrator reconciliation keeps external observation, snapshots, and failure state together"
)]
use super::*;
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdminCertificate {
    id: String,
    display_name: String,
    secret_name: String,
}
struct AdminConfiguration {
    certificates: Vec<AdminCertificate>,
    legacy_profiles: Vec<(String, String)>,
}
fn admin_configuration() -> Result<AdminConfiguration, Status> {
    let mut configured: Vec<AdminCertificate> = serde_json::from_str(
        &std::env::var("ORCHESTRATOR_ADMIN_CERTIFICATES").unwrap_or_else(|_| "[]".into()),
    )
    .map_err(|_| Status::failed_precondition("admin_certificate_configuration_invalid"))?;
    let mut legacy_profiles = Vec::new();
    if let Some(configuration) =
        crate::process::runtime_configuration::RuntimeConfiguration::from_environment()
            .map_err(|_| Status::failed_precondition("runtime_configuration_invalid"))?
    {
        legacy_profiles.extend(configuration.profiles.iter().filter_map(|(id, profile)| {
            profile
                .build
                .as_ref()
                .filter(|build| !build.signing_secret.is_empty())
                .map(|build| (id.clone(), build.signing_secret.clone()))
        }));
        if let Some(build) = configuration
            .managed_runtime
            .as_ref()
            .and_then(|runtime| runtime.build.as_ref())
            .filter(|build| !build.signing_secret.is_empty())
        {
            if !configured
                .iter()
                .any(|certificate| certificate.secret_name == build.signing_secret)
            {
                configured.insert(
                    0,
                    AdminCertificate {
                        id: format!(
                            "certificate-admin-{}",
                            &hex(&Sha256::digest(build.signing_secret.as_bytes()))[..16]
                        ),
                        display_name: build.signing_secret.clone(),
                        secret_name: build.signing_secret.clone(),
                    },
                );
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for build in configuration
            .profiles
            .values()
            .filter_map(|profile| profile.build.as_ref())
            .chain(
                configuration
                    .managed_runtime
                    .as_ref()
                    .and_then(|runtime| runtime.build.as_ref()),
            )
        {
            if !build.signing_secret.is_empty() {
                names.insert(build.signing_secret.clone());
            }
        }
        for name in names {
            if !configured.iter().any(|c| c.secret_name == name) {
                configured.push(AdminCertificate {
                    id: format!(
                        "certificate-admin-{}",
                        &hex(&Sha256::digest(name.as_bytes()))[..16]
                    ),
                    display_name: name.clone(),
                    secret_name: name,
                });
            }
        }
    }
    Ok(AdminConfiguration {
        certificates: configured,
        legacy_profiles,
    })
}
impl AppState {
    /// Reconciles only explicitly configured administrator Secrets; no discovery.
    ///
    /// # Errors
    /// Returns bounded configuration, persistence, and Kubernetes failures.
    pub async fn reconcile_certificates(&self) -> Result<(), Status> {
        let _guard = self.connection_lock.lock().await;
        let AdminConfiguration {
            certificates: configured,
            legacy_profiles,
        } = admin_configuration()?;
        let configured_ids: std::collections::BTreeSet<String> =
            configured.iter().map(|value| value.id.clone()).collect();
        if configured_ids.len() != configured.len() {
            return Err(Status::failed_precondition(
                "admin_certificate_configuration_invalid",
            ));
        }
        for admin in configured {
            self.sync_admin_certificate(admin).await?;
        }
        self.sync_removed_certificates(&configured_ids).await?;
        self.import_legacy_certificates(legacy_profiles).await?;
        self.sync_ui_certificate_health().await
    }
    async fn sync_admin_certificate(&self, admin: AdminCertificate) -> Result<(), Status> {
        validate_name(&admin.display_name)?;
        if admin.id.is_empty()
            || admin.id.len() > 40
            || !admin
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || admin.secret_name.is_empty()
        {
            return Err(Status::failed_precondition(
                "admin_certificate_configuration_invalid",
            ));
        }
        let previous = with_store_async(self, {
            let id = admin.id.clone();
            move |store| store.get_certificate(&id)
        })
        .await;
        let existed = previous.is_ok();
        let mut value = match previous {
            Ok(value) => {
                if value.management_source != "admin" {
                    return Err(Status::failed_precondition(
                        "admin_certificate_identity_conflict",
                    ));
                }
                value
            }
            Err(crate::adapters::sqlite::CleanStoreError::NotFound { .. }) => v1::Certificate {
                id: admin.id,
                display_name: admin.display_name.clone(),
                management_source: "admin".into(),
                enabled: true,
                resource_version: 1,
                ..Default::default()
            },
            Err(_) => {
                return Err(store_error(
                    crate::adapters::sqlite::CleanStoreError::CorruptState,
                ));
            }
        };
        let old = value.clone();
        value.source_secret_name = admin.secret_name.clone();
        value.display_name = admin.display_name;
        let observed = async {
            let credential = secrets()?
                .signing(&admin.secret_name, None)
                .await
                .map_err(Status::unavailable)?;
            validate_key(&credential.key_id)?;
            let public = public_key(&credential.seed)?;
            Ok::<_, Status>((credential, public))
        }
        .await;
        match observed {
            Ok((credential, public)) => {
                match self
                    .activate_admin_snapshot(value.clone(), credential, public)
                    .await
                {
                    Ok(snapshot) => value = snapshot,
                    Err(_) => {
                        value.status = "failed".into();
                        value.failure_reason = "certificate_snapshot_unavailable".into();
                    }
                }
            }
            Err(_) => {
                value.status = "failed".into();
                value.failure_reason = "certificate_secret_invalid_or_unavailable".into();
            }
        }
        if value != old {
            if existed {
                value.resource_version += 1;
            }
            let saved = value;
            with_store_async(self, move |store| {
                store.save_certificate(&saved)?;
                store.invalidate_certificate_runtime_profiles()?;
                Ok(())
            })
            .await
            .map_err(store_error)?;
        }
        Ok(())
    }
    async fn activate_admin_snapshot(
        &self,
        mut value: v1::Certificate,
        credential: ayni_orchestrator_kubernetes::connections::SigningCredential,
        public: String,
    ) -> Result<v1::Certificate, Status> {
        if value.public_key != public
            || value.key_id != credential.key_id
            || value.source_secret_uid != credential.uid
            || value.source_revision != credential.revision
            || value.secret_name.is_empty()
        {
            let digest = hex(&Sha256::digest(
                format!("{}:{}:{}", credential.uid, credential.revision, public).as_bytes(),
            ));
            let name = format!("{}-{}", value.id, &digest[..12]);
            let (uid, revision, public) = self
                .revision_secret(&value.id, &name, &credential.key_id, credential.seed, false)
                .await?;
            if !value.secret_name.is_empty() {
                value
                    .retired_secret_revisions
                    .push(v1::CertificateSecretRevision {
                        secret_name: value.secret_name,
                        secret_uid: value.secret_uid,
                    });
            }
            value.secret_name = name;
            value.secret_uid = uid;
            value.revision = revision;
            value.public_key = public;
            value.key_id = credential.key_id;
            value.source_secret_uid = credential.uid;
            value.source_revision = credential.revision;
        }
        value.status = "ready".into();
        value.failure_reason.clear();
        if !self.certificates().await?.iter().any(|c| c.is_default) {
            value.is_default = true;
        }
        Ok(value)
    }
    async fn sync_removed_certificates(
        &self,
        configured_ids: &std::collections::BTreeSet<String>,
    ) -> Result<(), Status> {
        for mut value in self
            .certificates()
            .await?
            .into_iter()
            .filter(|c| c.management_source == "admin" && !configured_ids.contains(&c.id))
        {
            if value.status != "failed"
                || value.failure_reason != "certificate_admin_reference_removed"
            {
                value.status = "failed".into();
                value.failure_reason = "certificate_admin_reference_removed".into();
                value.resource_version += 1;
                with_store_async(self, move |store| {
                    store.save_certificate(&value)?;
                    store.invalidate_certificate_runtime_profiles()?;
                    Ok(())
                })
                .await
                .map_err(store_error)?;
            }
        }
        Ok(())
    }
    async fn import_legacy_certificates(
        &self,
        legacy_profiles: Vec<(String, String)>,
    ) -> Result<(), Status> {
        let certificates = self.certificates().await?;
        for (profile_id, secret_name) in legacy_profiles {
            if let Some(certificate) = certificates.iter().find(|certificate| {
                certificate.management_source == "admin"
                    && certificate.source_secret_name == secret_name
                    && certificate.status == "ready"
            }) {
                let certificate_id = certificate.id.clone();
                with_store_async(self, move |store| {
                    store.import_legacy_runtime_certificate(&profile_id, &certificate_id)
                })
                .await
                .map_err(store_error)?;
            }
        }
        Ok(())
    }
    async fn sync_ui_certificate_health(&self) -> Result<(), Status> {
        // UI revision health is observed without rotating or overwriting private material.
        for mut value in self
            .certificates()
            .await?
            .into_iter()
            .filter(|c| c.management_source == "ui")
        {
            let healthy = secrets()?
                .signing(&value.secret_name, Some((&value.id, &value.secret_uid)))
                .await
                .is_ok_and(|credential| {
                    public_key(&credential.seed).is_ok_and(|public| public == value.public_key)
                        && credential.key_id == value.key_id
                });
            let status = if healthy { "ready" } else { "failed" };
            if value.status != status {
                value.status = status.into();
                value.failure_reason = if healthy {
                    String::new()
                } else {
                    "certificate_secret_invalid_or_unavailable".into()
                };
                value.resource_version += 1;
                with_store_async(self, move |store| {
                    store.save_certificate(&value)?;
                    store.invalidate_certificate_runtime_profiles()?;
                    Ok(())
                })
                .await
                .map_err(store_error)?;
            }
        }
        Ok(())
    }
}
