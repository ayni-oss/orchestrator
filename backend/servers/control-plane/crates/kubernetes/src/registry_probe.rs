//! Isolated registry test Jobs. Only Secret references cross this boundary.
use ayni_orchestrator_runtime::runtime_contract::RuntimeImage;
use k8s_openapi::api::{batch::v1::Job, core::v1::Pod};
use kube::{
    Api, Client,
    api::{DeleteParams, ListParams, PostParams},
};
use serde_json::json;

/// Frozen probe identity; a retry can observe/adopt the same Job.
#[derive(Clone)]
pub struct RegistryProbeRequest {
    pub namespace: String,
    pub operation: String,
    pub artifact_operation: String,
    pub image: String,
    pub image_namespace: String,
    pub push_secret: String,
    pub pull_secret: Option<String>,
    pub cleanup_digest: Option<String>,
}
fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|v| v.is_ascii_lowercase() || v.is_ascii_digit() || v == b'-')
}
impl RegistryProbeRequest {
    #[must_use]
    pub fn job_name(&self) -> String {
        format!("registry-test-{}", self.operation)
    }
    /// Renders a probe with no API token, host storage, or worker/model secrets.
    ///
    /// # Errors
    /// Returns a bounded code for invalid names, namespace, or unpinned image.
    pub fn render(&self) -> Result<Job, &'static str> {
        if !name(&self.namespace)
            || !name(&self.operation)
            || !name(&self.artifact_operation)
            || !name(&self.job_name())
            || !name(&self.push_secret)
            || self.pull_secret.as_deref().is_some_and(|s| !name(s))
            || RuntimeImage::parse(&self.image).is_err()
            || self.image_namespace.is_empty()
            || self.image_namespace.len() > 100
            || !self.image_namespace.bytes().all(|v| {
                v.is_ascii_lowercase()
                    || v.is_ascii_digit()
                    || matches!(v, b'-' | b'_' | b'.' | b'/')
            })
            || self.pull_secret.as_deref() == Some(self.push_secret.as_str())
        {
            return Err("registry_test_configuration_invalid");
        }
        let mut volumes = vec![
            json!({"name":"push","secret":{"secretName":self.push_secret,"defaultMode":292,"items":[{"key":".dockerconfigjson","path":"config.json"}]}}),
        ];
        let mut mounts =
            vec![json!({"name":"push","mountPath":"/credentials/push","readOnly":true})];
        if let Some(pull) = &self.pull_secret {
            volumes.push(json!({"name":"pull","secret":{"secretName":pull,"defaultMode":292,"items":[{"key":".dockerconfigjson","path":"config.json"}]}}));
            mounts.push(json!({"name":"pull","mountPath":"/credentials/pull","readOnly":true}));
        }
        let mut env = vec![
            json!({"name":"REGISTRY_NAMESPACE","value":self.image_namespace}),
            json!({"name":"REGISTRY_OPERATION","value":self.artifact_operation}),
        ];
        if let Some(digest) = &self.cleanup_digest {
            env.push(json!({"name":"REGISTRY_CLEANUP_DIGEST","value":digest}));
        }
        serde_json::from_value(json!({
            "apiVersion":"batch/v1","kind":"Job",
            "metadata":{"name":self.job_name(),"namespace":self.namespace,"labels":{"app.kubernetes.io/component":"connection-test","ayni.io/connection-operation":self.operation}},
            "spec":{"backoffLimit":0,"activeDeadlineSeconds":180,
                "template":{"metadata":{"labels":{"ayni.io/connection-operation":self.operation}},"spec":{
                    "automountServiceAccountToken":false,"restartPolicy":"Never",
                    "securityContext":{"runAsNonRoot":true,"runAsUser":10001,"runAsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},
                    "containers":[{"name":"probe","image":self.image,"imagePullPolicy":"IfNotPresent",
                        "command":["/usr/local/bin/registry-connection-test"],"env":env,"volumeMounts":mounts,
                        "securityContext":{"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}},
                        "resources":{"requests":{"cpu":"50m","memory":"32Mi"},"limits":{"cpu":"250m","memory":"128Mi"}},
                        "terminationMessagePolicy":"File","terminationMessagePath":"/dev/termination-log"}],"volumes":volumes}}}
        })).map_err(|_| "registry_test_configuration_invalid")
    }
}

/// Namespaced transport; never reads credential Secrets or logs.
pub struct RegistryProbeKubernetes {
    client: Client,
    namespace: String,
}
impl RegistryProbeKubernetes {
    /// # Errors
    /// Returns a bounded code when no in-cluster identity is available.
    pub fn new(namespace: &str) -> Result<Self, &'static str> {
        if !name(namespace) {
            return Err("registry_test_configuration_invalid");
        }
        let config = kube::Config::incluster().map_err(|_| "kubernetes_incluster_required")?;
        let client = Client::try_from(config).map_err(|_| "kubernetes_unavailable")?;
        Ok(Self {
            client,
            namespace: namespace.into(),
        })
    }
    /// # Errors
    /// Returns a bounded code for API failure or mismatched operation identity.
    pub async fn start(&self, request: &RegistryProbeRequest) -> Result<(), &'static str> {
        if request.namespace != self.namespace {
            return Err("registry_test_configuration_invalid");
        }
        let job = request.render()?;
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        match jobs.create(&PostParams::default(), &job).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(error)) if error.code == 409 => {
                let old = jobs
                    .get(&request.job_name())
                    .await
                    .map_err(|_| "registry_test_unavailable")?;
                let old = old.spec.ok_or("registry_test_identity_conflict")?;
                let desired = job.spec.ok_or("registry_test_configuration_invalid")?;
                // Kubernetes defaults fields; compare the immutable authority-bearing inputs.
                let same = old
                    .template
                    .spec
                    .as_ref()
                    .zip(desired.template.spec.as_ref())
                    .is_some_and(|(a, b)| {
                        a.containers == b.containers
                            && a.volumes == b.volumes
                            && a.automount_service_account_token
                                == b.automount_service_account_token
                            && a.security_context == b.security_context
                    });
                if same {
                    Ok(())
                } else {
                    Err("registry_test_identity_conflict")
                }
            }
            Err(_) => Err("registry_test_unavailable"),
        }
    }
    /// Observes the safe termination report. Caller persists before deleting the Job.
    ///
    /// # Errors
    /// Returns a bounded code if the Job failed without reporting or API is unavailable.
    pub async fn report(&self, operation: &str) -> Result<Option<String>, &'static str> {
        if !name(operation) {
            return Err("registry_test_identity_invalid");
        }
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let pods = pods
            .list(
                &ListParams::default().labels(&format!("ayni.io/connection-operation={operation}")),
            )
            .await
            .map_err(|_| "registry_test_unavailable")?;
        for pod in pods {
            if let Some(report) = pod_report(pod)? {
                return Ok(Some(report));
            }
        }
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        let job = jobs
            .get(&format!("registry-test-{operation}"))
            .await
            .map_err(|_| "registry_test_unavailable")?;
        check_job_completion(job)?;
        Ok(None)
    }
    /// Deletes only the completed test Job after its report is durably persisted.
    ///
    /// # Errors
    /// Returns a bounded code if the API refuses deletion.
    pub async fn cleanup_job(&self, operation: &str) -> Result<(), &'static str> {
        if !name(operation) {
            return Err("registry_test_identity_invalid");
        }
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        match jobs
            .delete(
                &format!("registry-test-{operation}"),
                &DeleteParams {
                    propagation_policy: Some(kube::api::PropagationPolicy::Background),
                    ..DeleteParams::default()
                },
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(error)) if error.code == 404 => Ok(()),
            Err(_) => Err("registry_test_cleanup_unavailable"),
        }
    }
}
fn pod_report(pod: Pod) -> Result<Option<String>, &'static str> {
    let Some(status) = pod.status else {
        return Ok(None);
    };
    let terminated = status
        .container_statuses
        .unwrap_or_default()
        .into_iter()
        .filter(|container| container.name == "probe")
        .find_map(|container| container.state.and_then(|state| state.terminated));
    if let Some(terminated) = terminated {
        let message = terminated.message.unwrap_or_default();
        if terminated.exit_code != 0 && message.is_empty() {
            return Err("registry_test_failed");
        }
        if message.is_empty() || message.len() > 4096 {
            return Err("registry_test_result_invalid");
        }
        return Ok(Some(message));
    }
    if status.phase.as_deref() == Some("Failed") {
        return Err("registry_test_failed");
    }
    Ok(None)
}
fn check_job_completion(job: Job) -> Result<(), &'static str> {
    let Some(status) = job.status else {
        return Ok(());
    };
    let failed = status.failed.unwrap_or_default() > 0
        || status.conditions.as_ref().is_some_and(|conditions| {
            conditions
                .iter()
                .any(|condition| condition.type_ == "Failed" && condition.status == "True")
        });
    if failed {
        return Err("registry_test_failed");
    }
    if status.succeeded.unwrap_or_default() > 0 {
        return Err("registry_test_result_missing");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> RegistryProbeRequest {
        RegistryProbeRequest {
            namespace: "test".into(),
            operation: "probe-1".into(),
            artifact_operation: "probe-1".into(),
            image: format!("ghcr.io/acme/control-plane@sha256:{}", "a".repeat(64)),
            image_namespace: "acme".into(),
            push_secret: "push-revision".into(),
            pull_secret: Some("pull-revision".into()),
            cleanup_digest: None,
        }
    }
    #[test]
    fn registry_probe_has_only_separate_mounted_registry_authority() -> Result<(), &'static str> {
        let job = serde_json::to_value(request().render()?).map_err(|_| "serialize_failed")?;
        let pod = &job["spec"]["template"]["spec"];
        assert_eq!(pod["automountServiceAccountToken"], false);
        assert_eq!(
            pod["containers"][0]["securityContext"]["capabilities"]["drop"][0],
            "ALL"
        );
        assert_eq!(pod["volumes"].as_array().map(Vec::len), Some(2));
        assert!(pod.get("imagePullSecrets").is_none());
        assert!(job["spec"].get("ttlSecondsAfterFinished").is_none());
        Ok(())
    }
    #[test]
    fn probe_requires_pinned_image_and_separate_pull_authority() {
        let mut request = request();
        request.image = "ghcr.io/acme/control-plane:latest".into();
        assert!(request.render().is_err());
        request = self::request();
        request.pull_secret = Some(request.push_secret.clone());
        assert!(request.render().is_err());
    }
    #[test]
    fn interrupted_probe_keeps_its_safe_checkpoint_report() -> Result<(), &'static str> {
        let pod:Pod=serde_json::from_value(json!({"metadata":{"name":"probe"},"status":{"phase":"Failed","containerStatuses":[{"name":"probe","image":"probe","imageID":"fixture","ready":false,"restartCount":0,"state":{"terminated":{"exitCode":137,"message":"{\"artifact_digest\":\"sha256:fixture\"}"}}}]}})).map_err(|_|"fixture_invalid")?;
        assert!(pod_report(pod)?.is_some());
        Ok(())
    }
}
