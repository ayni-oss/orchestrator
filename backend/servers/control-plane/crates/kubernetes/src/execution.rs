//! Kubernetes execution transport. Secret contents are never read back.
use crate::{
    configuration::{ExecutionConfiguration, WorkloadRuntime},
    workload,
};
use ayni_orchestrator_runtime::runtime_execution::ExecutionRequest;
use k8s_openapi::api::{
    batch::v1::Job,
    core::v1::{ConfigMap, Pod, Secret},
};
use kube::{
    Api, Client,
    api::{DeleteParams, ListParams, PostParams},
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

pub struct RuntimeKubernetes {
    pub(crate) client: Client,
    pub(crate) configuration: ExecutionConfiguration,
}

pub use ayni_orchestrator_application::execution::{ExecutionState, Observation};

impl RuntimeKubernetes {
    /// Refresh projected repository authority without exposing existing Secret values.
    ///
    /// # Errors
    /// Returns a bounded reason when Kubernetes rejects the scoped credential update.
    pub async fn refresh_credentials(&self, name: &str, token: &str) -> Result<(), &'static str> {
        let secrets: Api<Secret> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        secrets
            .patch(
                name,
                &kube::api::PatchParams::default(),
                &kube::api::Patch::Merge(json!({"stringData":{"token":token}})),
            )
            .await
            .map_err(|_| "runtime_source_credential_unavailable")?;
        Ok(())
    }

    /// Creates an executor using only the deployed pod's Kubernetes identity.
    ///
    /// # Errors
    ///
    /// Returns a bounded error code if in-cluster configuration or client setup fails.
    pub fn new(configuration: ExecutionConfiguration) -> Result<Self, &'static str> {
        // Runtime execution is a deployed controller feature. Never fall back
        // to a developer's current kubeconfig context when run on a host.
        let config = kube::Config::incluster().map_err(|_| "kubernetes_incluster_required")?;
        let client = Client::try_from(config).map_err(|_| "kubernetes_unavailable")?;
        Ok(Self {
            client,
            configuration,
        })
    }

    /// Starts or adopts a verification Job with an immutable execution request.
    ///
    /// # Errors
    ///
    /// Returns a bounded error code for invalid requests, Kubernetes failures,
    /// or a conflicting operation identity.
    pub async fn start<Profile: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<Profile>,
        token: &str,
    ) -> Result<(), &'static str> {
        let job = verification_job(name, &self.configuration, runtime)?;
        self.create(
            name,
            std::collections::BTreeMap::from([(
                "execution.json".into(),
                serde_json::to_string(request).map_err(|_| "runtime_request_invalid")?,
            )]),
            token,
            &job,
        )
        .await
    }

    pub(crate) async fn create(
        &self,
        name: &str,
        data: std::collections::BTreeMap<String, String>,
        token: &str,
        job: &Job,
    ) -> Result<(), &'static str> {
        let mut job = job.clone();
        let identity = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&job.spec).map_err(|_| "runtime_job_invalid")?)
        );
        job.metadata
            .annotations
            .get_or_insert_default()
            .insert("ayni.io/spec-digest".into(), identity.clone());
        let namespace = &self.configuration.namespace;
        let maps: Api<ConfigMap> = Api::namespaced(self.client.clone(), namespace);
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), namespace);
        let map: ConfigMap = serde_json::from_value(json!({"apiVersion":"v1", "kind":"ConfigMap", "metadata":{"name":name,"labels":{"ayni.io/runtime-operation":name}}, "immutable":true, "data":data})).map_err(|_| "runtime_request_invalid")?;
        match maps.create(&PostParams::default(), &map).await {
            Ok(_) => {}
            Err(kube::Error::Api(error)) if error.code == 409 => {
                let existing = maps.get(name).await.map_err(|_| "kubernetes_unavailable")?;
                if !request_data_matches(existing.data.as_ref(), map.data.as_ref()) {
                    return Err("runtime_request_conflict");
                }
            }
            Err(_) => return Err("kubernetes_unavailable"),
        }
        let secret: Secret = serde_json::from_value(json!({"apiVersion":"v1", "kind":"Secret", "metadata":{"name":name,"labels":{"ayni.io/runtime-operation":name}},"stringData":{"token":token}})).map_err(|_| "runtime_request_invalid")?;
        match secrets.create(&PostParams::default(), &secret).await {
            Ok(_) => {}
            Err(kube::Error::Api(error)) if error.code == 409 => {}
            Err(_) => return Err("runtime_source_credential_unavailable"),
        }
        match jobs.create(&PostParams::default(), &job).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(error)) if error.code == 409 => {
                let existing = jobs.get(name).await.map_err(|_| "kubernetes_unavailable")?;
                if existing
                    .metadata
                    .annotations
                    .as_ref()
                    .and_then(|a| a.get("ayni.io/spec-digest"))
                    != Some(&identity)
                {
                    return Err("runtime_job_conflict");
                }
                let actual = existing
                    .spec
                    .as_ref()
                    .and_then(|s| s.template.spec.as_ref());
                let expected = job.spec.as_ref().and_then(|s| s.template.spec.as_ref());
                if actual.zip(expected).is_none_or(|(actual, expected)| {
                    actual
                        .containers
                        .iter()
                        .map(|c| (&c.name, &c.image, &c.args))
                        .ne(expected
                            .containers
                            .iter()
                            .map(|c| (&c.name, &c.image, &c.args)))
                }) {
                    return Err("runtime_job_conflict");
                }
                Ok(())
            }
            Err(_) => Err("runtime_job_creation_failed"),
        }
    }

    /// Reads a verification Job and validates its completed pod identity.
    ///
    /// # Errors
    ///
    /// Returns a bounded error code if Kubernetes is unavailable or the completed
    /// pod does not contain a valid verification result and image digest.
    pub async fn observe(&self, name: &str) -> Result<Observation, &'static str> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let Some(job) = jobs
            .get_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
        else {
            return Ok(Observation::Missing);
        };
        if job.status.as_ref().is_some_and(|s| {
            s.failed.unwrap_or(0) > 0
                || s.conditions.as_ref().is_some_and(|conditions| {
                    conditions
                        .iter()
                        .any(|c| c.type_ == "Failed" && c.status == "True")
                })
        }) {
            let code = self
                .operation_message(name, "verify")
                .await
                .unwrap_or_default();
            return Ok(Observation::Failed(match code.as_str() {
                "runtime_lock_mismatch" => "runtime_lock_mismatch",
                "runtime_platform_mismatch" => "runtime_platform_mismatch",
                "runtime_version_unsupported" => "runtime_version_unsupported",
                "runtime_certificate_invalid" => "runtime_certificate_invalid",
                _ => "runtime_verification_failed",
            }));
        }
        if job.status.as_ref().and_then(|s| s.succeeded).unwrap_or(0) != 1 {
            return Ok(Observation::Running);
        }
        let pod = match self.owned_operation_pod(name, &job).await {
            Ok(pod) => pod,
            Err("runtime_pod_identity_invalid") => {
                return Ok(Observation::Failed("runtime_pod_identity_invalid"));
            }
            Err(reason) => return Err(reason),
        };
        let status =
            crate::observation::container_status(&pod, "verify").ok_or("runtime_result_missing")?;
        status
            .state
            .as_ref()
            .and_then(|s| s.terminated.as_ref())
            .filter(|s| s.exit_code == 0)
            .ok_or("runtime_verification_failed")?;
        let message = crate::observation::termination_message(status, true)
            .ok_or("runtime_result_invalid")?;
        let report = message.to_owned();
        let digest = status
            .image_id
            .rsplit_once("@sha256:")
            .map(|(_, digest)| format!("sha256:{digest}"))
            .filter(|value| {
                value.len() == 71
                    && value[7..]
                        .bytes()
                        .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
            })
            .ok_or("runtime_digest_unavailable")?;
        Ok(Observation::Completed { digest, report })
    }

    /// Reports whether every pod belonging to a coding Job has terminated.
    ///
    /// # Errors
    ///
    /// Returns a bounded error code if the Job's pods cannot be inspected.
    pub async fn coding_stopped(&self, name: &str) -> Result<bool, &'static str> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let pods = pods
            .list(&ListParams::default().labels(&format!("job-name={name}")))
            .await
            .map_err(|_| "kubernetes_unavailable")?;
        Ok(pods.items.iter().all(|pod| {
            pod.status
                .as_ref()
                .and_then(|s| s.phase.as_deref())
                .is_some_and(|phase| matches!(phase, "Succeeded" | "Failed"))
        }))
    }

    /// Removes an operation's Job, immutable request, and source credential.
    ///
    /// # Errors
    ///
    /// Returns a bounded error code if any Kubernetes deletion fails.
    pub async fn cleanup(&self, name: &str) -> Result<(), &'static str> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let maps: Api<ConfigMap> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let secrets: Api<Secret> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        match jobs.delete(name, &DeleteParams::foreground()).await {
            Ok(_) => {}
            Err(kube::Error::Api(error)) if error.code == 404 => {}
            Err(_) => return Err("runtime_cleanup_failed"),
        }
        if jobs
            .get_opt(name)
            .await
            .map_err(|_| "runtime_cleanup_failed")?
            .is_some()
        {
            return Err("runtime_cleanup_pending");
        }
        for result in [
            maps.delete(name, &DeleteParams::default())
                .await
                .map(|_| ()),
            secrets
                .delete(name, &DeleteParams::default())
                .await
                .map(|_| ()),
        ] {
            match result {
                Ok(()) => {}
                Err(kube::Error::Api(error)) if error.code == 404 => {}
                Err(_) => return Err("runtime_cleanup_failed"),
            }
        }
        self.cleanup_engine(name).await?;
        Ok(())
    }
}

/// Derives a deterministic Kubernetes operation name from a profile sequence.
#[must_use]
pub fn operation_name(profile: &str, sequence: u64) -> String {
    format!(
        "runtime-{:x}",
        Sha256::digest(format!("{profile}:{sequence}"))
    )[..48]
        .to_owned()
}

/// Renders a verification Job with source credentials confined to checkout.
///
/// # Errors
///
/// Returns a bounded error code if the Job representation cannot be decoded.
pub fn verification_job(
    name: &str,
    configuration: &ExecutionConfiguration,
    runtime: &WorkloadRuntime,
) -> Result<Job, &'static str> {
    let container =
        workload::worker_container("verify", &runtime.image, &workload::runtime_environment());
    serde_json::from_value(workload::source_job(
        name,
        configuration,
        runtime,
        &runtime.service_account,
        &container,
    ))
    .map_err(|_| "runtime_job_invalid")
}

// Kubernetes omits an empty data map when returning a ConfigMap.
fn request_data_matches(
    actual: Option<&std::collections::BTreeMap<String, String>>,
    expected: Option<&std::collections::BTreeMap<String, String>>,
) -> bool {
    actual
        .into_iter()
        .flatten()
        .eq(expected.into_iter().flatten())
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_discovery_requests_survive_api_normalization_without_accepting_changed_data() {
        use std::collections::BTreeMap;
        assert!(super::request_data_matches(None, Some(&BTreeMap::new())));
        let request = Some(BTreeMap::from([("execution.json".into(), "frozen".into())]));
        assert!(super::request_data_matches(
            request.as_ref(),
            request.as_ref()
        ));
        assert!(!super::request_data_matches(None, request.as_ref()));
        let changed = Some(BTreeMap::from([(
            "execution.json".into(),
            "changed".into(),
        )]));
        assert!(!super::request_data_matches(
            request.as_ref(),
            changed.as_ref()
        ));
    }

    use super::*;
    #[test]
    fn verification_preserves_entrypoint_and_limits_credentials_to_checkout()
    -> Result<(), &'static str> {
        let configuration = ExecutionConfiguration {
            namespace: "test".into(),
            worker_image: "worker".into(),
            output_claim: "output".into(),
            model_secret: "model".into(),
            allow_emulation: false,
            engine_mode: "external-isolated".into(),
        };
        let runtime = WorkloadRuntime {
            image: "registry/runtime:lock".into(),
            platform: "linux/amd64".into(),
            service_account: "runtime".into(),
            pull_secret: None,
            build: None,
        };
        let job = verification_job("verify-test", &configuration, &runtime)?;
        let pod = job
            .spec
            .and_then(|s| s.template.spec)
            .ok_or("pod_missing")?;
        for container in pod
            .containers
            .iter()
            .chain(pod.init_containers.iter().flatten())
        {
            for mount in container
                .volume_mounts
                .iter()
                .flatten()
                .filter(|m| m.name == "request")
            {
                assert_eq!(mount.mount_path, "/request/execution.json");
                assert_eq!(mount.sub_path.as_deref(), Some("execution.json"));
                assert_eq!(mount.read_only, Some(true));
            }
        }
        assert_eq!(pod.automount_service_account_token, Some(false));
        assert!(pod.containers[0].env.iter().flatten().any(|variable| {
            variable.name == "XDG_CACHE_HOME" && variable.value.as_deref() == Some("/scratch/cache")
        }));
        assert!(pod.containers[0].command.is_none());
        assert!(
            pod.containers[0]
                .env
                .as_ref()
                .is_some_and(|env| env.iter().all(|e| e.value_from.is_none()))
        );
        assert_eq!(pod.service_account_name.as_deref(), Some("runtime"));
        assert_eq!(
            pod.node_selector
                .as_ref()
                .and_then(|selectors| selectors.get("kubernetes.io/arch"))
                .map(String::as_str),
            Some("amd64")
        );
        assert!(
            pod.volumes
                .ok_or("volumes_missing")?
                .iter()
                .all(|v| v.host_path.is_none())
        );
        Ok(())
    }

    #[test]
    fn local_emulation_omits_only_the_platform_node_selector() -> Result<(), &'static str> {
        let configuration = ExecutionConfiguration {
            namespace: "test".into(),
            worker_image: "worker".into(),
            output_claim: "output".into(),
            model_secret: "model".into(),
            allow_emulation: true,
            engine_mode: "external-isolated".into(),
        };
        let runtime = WorkloadRuntime {
            image: "registry/runtime:lock".into(),
            platform: "linux/amd64".into(),
            service_account: "runtime".into(),
            pull_secret: None,
            build: None,
        };
        let job = verification_job("verify-test", &configuration, &runtime)?;
        let pod = job
            .spec
            .and_then(|s| s.template.spec)
            .ok_or("pod_missing")?;
        assert_eq!(pod.node_selector, Some(std::collections::BTreeMap::new()));
        Ok(())
    }
}
