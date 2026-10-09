//! Pull-only manifest discovery; no repository code or engine authority.
use crate::observation::{container_status, termination_message};
use crate::{execution::RuntimeKubernetes, workload};
use ayni_orchestrator_application::execution::{ImageDiscovery, WorkloadRuntime};
use k8s_openapi::api::batch::v1::Job;
use kube::Api;
use serde_json::json;

impl RuntimeKubernetes {
    pub(crate) async fn operation_message(
        &self,
        name: &str,
        container: &str,
    ) -> Result<String, &'static str> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let job = jobs.get(name).await.map_err(|_| "kubernetes_unavailable")?;
        let pod = self.owned_operation_pod(name, &job).await?;
        container_status(&pod, container)
            .and_then(|status| termination_message(status, false))
            .map(str::to_owned)
            .ok_or("runtime_result_missing")
    }
    /// Read the owned build Job’s immutable local candidate identity.
    /// # Errors
    /// Rejects missing, oversized or invalid completion metadata.
    pub async fn build_result(&self, name: &str) -> Result<String, &'static str> {
        let message = self.operation_message(name, "build").await?;
        if message.len() == 71
            && message.starts_with("sha256:")
            && message[7..]
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
        {
            Ok(message)
        } else {
            Err("runtime_build_result_invalid")
        }
    }
    /// Schedule a pull-only registry lookup.
    /// # Errors
    /// Rejects invalid workload settings or conflicting Kubernetes resources.
    pub async fn start_discovery(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
    ) -> Result<(), &'static str> {
        runtime.validate()?;
        let mounts = runtime
            .pull_secret
            .as_ref()
            .map(|_| json!({"name":"registry","mountPath":"/registry","readOnly":true}))
            .into_iter()
            .collect::<Vec<_>>();
        let volumes = runtime.pull_secret.as_ref().map(|secret| json!({"name":"registry","secret":{"secretName":secret,"items":[{"key":".dockerconfigjson","path":"config.json"}]}})).into_iter().collect::<Vec<_>>();
        let job: Job = serde_json::from_value(json!({
            "apiVersion":"batch/v1","kind":"Job",
            "metadata":{"name":name,"labels":{"ayni.io/runtime-operation":name}},
            "spec":{"backoffLimit":0,"activeDeadlineSeconds":120,"template":{
                "metadata":{"labels":{"ayni.io/runtime-operation":name}},
                "spec":{"restartPolicy":"Never","serviceAccountName":runtime.service_account,
                    "affinity":workload::trusted_node_affinity(),
                    "automountServiceAccountToken":false,"securityContext":{"runAsNonRoot":true,"seccompProfile":{"type":"RuntimeDefault"}},
                    "imagePullSecrets":runtime.pull_secret.as_ref().map(|s|json!({"name":s})).into_iter().collect::<Vec<_>>(),
                    "containers":[{"name":"discover","image":self.configuration.worker_image,"args":["registry-discover"],
                        "securityContext":workload::container_security(),"resources":workload::container_resources(),
                        "env":[{"name":"DISCOVERY_IMAGE","value":runtime.image},{"name":"DISCOVERY_PLATFORM","value":runtime.platform}],
                        "volumeMounts":mounts}],"volumes":volumes
                }
            }}
        })).map_err(|_| "runtime_job_invalid")?;
        self.create(name, std::collections::BTreeMap::new(), "", &job)
            .await
    }

    /// Read the owned registry lookup outcome.
    /// # Errors
    /// Rejects unavailable Jobs, foreign Pods or malformed completion data.
    pub async fn discover(&self, name: &str) -> Result<ImageDiscovery, &'static str> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let job = jobs
            .get_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
            .ok_or("runtime_result_missing")?;
        if job
            .status
            .as_ref()
            .is_some_and(|s| s.failed.unwrap_or(0) > 0)
        {
            return Ok(ImageDiscovery::Failed {
                reason: "registry_discovery_failed".into(),
            });
        }
        if job.status.as_ref().and_then(|s| s.succeeded).unwrap_or(0) != 1 {
            return Ok(ImageDiscovery::Pending);
        }
        let pod = self.owned_operation_pod(name, &job).await?;
        let message = container_status(&pod, "discover")
            .and_then(|status| termination_message(status, true))
            .ok_or("runtime_result_invalid")?;
        serde_json::from_str(message).map_err(|_| "runtime_result_invalid")
    }
}
