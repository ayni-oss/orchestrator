//! Namespace-scoped connector Secrets; API errors never expose credential contents.
use k8s_openapi::api::core::v1::Secret;
use kube::{
    Api, Client,
    api::{DeleteParams, PostParams, Preconditions},
};
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct ConnectionSecrets {
    secrets: Api<Secret>,
    jobs: kube::Api<k8s_openapi::api::batch::v1::Job>,
}
pub struct GithubCredential {
    pub app_id: String,
    pub private_key: Vec<u8>,
    pub uid: String,
    pub revision: String,
}
impl ConnectionSecrets {
    /// Constructs a namespace-scoped in-cluster Secret adapter.
    ///
    /// # Errors
    /// Returns a bounded reason code when configuration, identity, or Kubernetes access fails.
    pub fn new(namespace: &str) -> Result<Self, &'static str> {
        let config = kube::Config::incluster().map_err(|_| "kubernetes_incluster_required")?;
        let client = Client::try_from(config).map_err(|_| "kubernetes_unavailable")?;
        Ok(Self {
            secrets: Api::namespaced(client.clone(), namespace),
            jobs: Api::namespaced(client, namespace),
        })
    }
    /// Creates an immutable, explicitly owned credential revision.
    ///
    /// # Errors
    /// Returns a bounded reason code when configuration, identity, or Kubernetes access fails.
    pub async fn create(
        &self,
        name: &str,
        id: &str,
        values: BTreeMap<String, String>,
        docker: bool,
    ) -> Result<(String, String), &'static str> {
        let secret = Secret {
            metadata: kube::core::ObjectMeta {
                name: Some(name.into()),
                labels: Some(BTreeMap::from([
                    ("ayni.dev/connection-id".into(), id.into()),
                    ("ayni.dev/management-source".into(), "ui".into()),
                    ("ayni.dev/purpose".into(), "connection".into()),
                ])),
                ..Default::default()
            },
            string_data: Some(values),
            type_: Some(
                if docker {
                    "kubernetes.io/dockerconfigjson"
                } else {
                    "Opaque"
                }
                .into(),
            ),
            immutable: Some(true),
            ..Default::default()
        };
        let created = self
            .secrets
            .create(&PostParams::default(), &secret)
            .await
            .map_err(|_| "connection_secret_create_failed")?;
        Ok((
            created
                .metadata
                .uid
                .ok_or("connection_secret_identity_missing")?,
            created
                .metadata
                .resource_version
                .ok_or("connection_secret_identity_missing")?,
        ))
    }
    /// Checks whether an unfinished namespaced Job references a Secret.
    ///
    /// # Errors
    /// Returns a bounded reason code when configuration, identity, or Kubernetes access fails.
    pub async fn in_use(&self, name: &str) -> Result<bool, &'static str> {
        if name.is_empty() {
            return Ok(false);
        }
        let jobs = self
            .jobs
            .list(&kube::api::ListParams::default())
            .await
            .map_err(|_| "connection_jobs_unavailable")?;
        for job in jobs {
            let finished = job
                .status
                .as_ref()
                .and_then(|status| status.conditions.as_ref())
                .is_some_and(|conditions| {
                    conditions.iter().any(|condition| {
                        condition.status == "True"
                            && matches!(condition.type_.as_str(), "Complete" | "Failed")
                    })
                });
            if finished {
                continue;
            }
            if let Some(spec) = job.spec {
                let value = serde_json::to_value(spec).map_err(|_| "connection_job_invalid")?;
                if references(&value, name) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    /// Reads and verifies safe Secret identity and ownership metadata.
    ///
    /// # Errors
    /// Returns a bounded reason code when configuration, identity, or Kubernetes access fails.
    pub async fn metadata(
        &self,
        name: &str,
        owner: Option<(&str, &str)>,
    ) -> Result<(String, String), &'static str> {
        let secret = self
            .secrets
            .get_metadata(name)
            .await
            .map_err(|_| "connection_secret_unavailable")?;
        let uid = secret
            .metadata
            .uid
            .ok_or("connection_secret_identity_missing")?;
        if let Some((id, expected_uid)) = owner {
            let labels = secret
                .metadata
                .labels
                .ok_or("connection_secret_ownership_mismatch")?;
            if (!expected_uid.is_empty() && expected_uid != uid)
                || labels.get("ayni.dev/connection-id").map(String::as_str) != Some(id)
                || labels.get("ayni.dev/management-source").map(String::as_str) != Some("ui")
            {
                return Err("connection_secret_ownership_mismatch");
            }
        }
        Ok((
            uid,
            secret
                .metadata
                .resource_version
                .ok_or("connection_secret_identity_missing")?,
        ))
    }
    /// Reads the referenced GitHub credential for application authentication.
    ///
    /// # Errors
    /// Returns a bounded reason code when configuration, identity, or Kubernetes access fails.
    pub async fn github(
        &self,
        name: &str,
        owner: Option<(&str, &str)>,
    ) -> Result<GithubCredential, &'static str> {
        let secret = self
            .secrets
            .get(name)
            .await
            .map_err(|_| "connection_secret_unavailable")?;
        let uid = secret
            .metadata
            .uid
            .ok_or("connection_secret_identity_missing")?;
        if let Some((id, expected_uid)) = owner {
            let labels = secret
                .metadata
                .labels
                .as_ref()
                .ok_or("connection_secret_ownership_mismatch")?;
            if (!expected_uid.is_empty() && uid != expected_uid)
                || labels.get("ayni.dev/connection-id").map(String::as_str) != Some(id)
                || labels.get("ayni.dev/management-source").map(String::as_str) != Some("ui")
            {
                return Err("connection_secret_ownership_mismatch");
            }
        }
        let mut data = secret.data.ok_or("github_credentials_invalid")?;
        Ok(GithubCredential {
            app_id: String::from_utf8(data.remove("app-id").ok_or("github_credentials_invalid")?.0)
                .map_err(|_| "github_credentials_invalid")?,
            private_key: data
                .remove("private-key.pem")
                .or_else(|| data.remove("private-key"))
                .ok_or("github_credentials_invalid")?
                .0,
            uid,
            revision: secret
                .metadata
                .resource_version
                .ok_or("connection_secret_identity_missing")?,
        })
    }
    /// Deletes an owned revision with UID and resource-version preconditions.
    ///
    /// # Errors
    /// Returns a bounded reason code when configuration, identity, or Kubernetes access fails.
    pub async fn delete(&self, name: &str, id: &str, uid: &str) -> Result<(), &'static str> {
        let Some(secret) = self
            .secrets
            .get_metadata_opt(name)
            .await
            .map_err(|_| "connection_secret_unavailable")?
        else {
            return Ok(());
        };
        let labels = secret
            .metadata
            .labels
            .ok_or("connection_secret_ownership_mismatch")?;
        if (!uid.is_empty() && secret.metadata.uid.as_deref() != Some(uid))
            || labels.get("ayni.dev/connection-id").map(String::as_str) != Some(id)
            || labels.get("ayni.dev/management-source").map(String::as_str) != Some("ui")
        {
            return Err("connection_secret_ownership_mismatch");
        }
        self.secrets
            .delete(
                name,
                &DeleteParams {
                    preconditions: Some(Preconditions {
                        uid: secret.metadata.uid,
                        resource_version: secret.metadata.resource_version,
                    }),
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| "connection_secret_delete_failed")?;
        Ok(())
    }
}

fn references(value: &serde_json::Value, name: &str) -> bool {
    match value {
        serde_json::Value::Object(fields) => fields.iter().any(|(key, value)| {
            ((key == "secretName" || key == "name") && value.as_str() == Some(name))
                || references(value, name)
        }),
        serde_json::Value::Array(values) => values.iter().any(|value| references(value, name)),
        _ => false,
    }
}
