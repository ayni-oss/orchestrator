//! Kubernetes execution transport. Secret contents are never read back.
use crate::runtime_configuration::{ProfileRuntime, RuntimeConfiguration};
use ayni_orchestrator_runtime::runtime_execution::{ExecutionRequest, VerificationReport};
use k8s_openapi::api::{
    batch::v1::Job,
    core::v1::{ConfigMap, Pod, Secret},
};
use kube::{
    Api, Client,
    api::{DeleteParams, ListParams, PostParams},
};
use serde_json::json;
use sha2::{Digest, Sha256};

pub struct RuntimeKubernetes {
    pub(crate) client: Client,
    pub(crate) configuration: RuntimeConfiguration,
}

pub enum Observation {
    Missing,
    Running,
    Verified {
        digest: String,
        report: Box<VerificationReport>,
    },
    Failed(&'static str),
}

impl RuntimeKubernetes {
    pub(crate) fn new(configuration: RuntimeConfiguration) -> Result<Self, &'static str> {
        // Runtime execution is a deployed controller feature. Never fall back
        // to a developer's current kubeconfig context when run on a host.
        let config = kube::Config::incluster().map_err(|_| "kubernetes_incluster_required")?;
        let client = Client::try_from(config).map_err(|_| "kubernetes_unavailable")?;
        Ok(Self {
            client,
            configuration,
        })
    }

    pub(crate) async fn start(
        &self,
        name: &str,
        runtime: &ProfileRuntime,
        request: &ExecutionRequest,
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
                if existing.data != map.data {
                    return Err("runtime_request_conflict");
                }
            }
            Err(_) => return Err("kubernetes_unavailable"),
        }
        let secret: Secret = serde_json::from_value(json!({"apiVersion":"v1", "kind":"Secret", "metadata":{"name":name,"labels":{"ayni.io/runtime-operation":name}},"immutable":true,"stringData":{"token":token}})).map_err(|_| "runtime_request_invalid")?;
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

    pub(crate) async fn observe(&self, name: &str) -> Result<Observation, &'static str> {
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
            return Ok(Observation::Failed("runtime_verification_failed"));
        }
        if job.status.as_ref().and_then(|s| s.succeeded).unwrap_or(0) != 1 {
            return Ok(Observation::Running);
        }
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let list = pods
            .list(
                &ListParams::default()
                    .labels(&format!("ayni.io/runtime-operation={name}"))
                    .limit(2),
            )
            .await
            .map_err(|_| "kubernetes_unavailable")?;
        if list.items.len() != 1 {
            return Ok(Observation::Failed("runtime_pod_identity_invalid"));
        }
        let pod = &list.items[0];
        let owner_matches = pod
            .metadata
            .owner_references
            .as_ref()
            .is_some_and(|owners| {
                owners.iter().any(|owner| {
                    Some(&owner.uid) == job.metadata.uid.as_ref() && owner.controller == Some(true)
                })
            });
        if !owner_matches {
            return Ok(Observation::Failed("runtime_pod_identity_invalid"));
        }
        let status = pod
            .status
            .as_ref()
            .and_then(|s| s.container_statuses.as_ref())
            .and_then(|s| s.iter().find(|s| s.name == "verify"))
            .ok_or("runtime_result_missing")?;
        let finished = status
            .state
            .as_ref()
            .and_then(|s| s.terminated.as_ref())
            .filter(|s| s.exit_code == 0)
            .ok_or("runtime_verification_failed")?;
        let message = finished
            .message
            .as_deref()
            .filter(|s| s.len() <= 4096)
            .ok_or("runtime_result_invalid")?;
        let report = serde_json::from_str(message).map_err(|_| "runtime_result_invalid")?;
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
        Ok(Observation::Verified { digest, report })
    }

    pub(crate) async fn cleanup(&self, name: &str) -> Result<(), &'static str> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let maps: Api<ConfigMap> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let secrets: Api<Secret> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        for result in [
            jobs.delete(name, &DeleteParams::background())
                .await
                .map(|_| ()),
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
        Ok(())
    }
}

pub fn operation_name(profile: &str, sequence: u64) -> String {
    format!(
        "runtime-{:x}",
        Sha256::digest(format!("{profile}:{sequence}"))
    )[..48]
        .to_owned()
}

pub fn verification_job(
    name: &str,
    configuration: &RuntimeConfiguration,
    runtime: &ProfileRuntime,
) -> Result<Job, &'static str> {
    let security = json!({"runAsNonRoot":true,"runAsUser":10001,"runAsGroup":10001,"readOnlyRootFilesystem":true,"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]}});
    let resources = json!({"requests":{"cpu":"500m","memory":"1Gi"}, "limits":{"cpu":"2","memory":"4Gi","ephemeral-storage":"8Gi"}});
    let workspace = json!({"name":"workspace","mountPath":"/workspace"});
    let scratch = json!({"name":"scratch","mountPath":"/scratch"});
    let request = json!({"name":"request","mountPath":"/request","readOnly":true});
    let pulls: Vec<_> = runtime
        .pull_secret
        .iter()
        .map(|name| json!({"name":name}))
        .collect();
    serde_json::from_value(json!({"apiVersion":"batch/v1","kind":"Job","metadata":{"name":name,"namespace":configuration.namespace,"labels":{"ayni.io/runtime-operation":name}},
        "spec":{"backoffLimit":0,"activeDeadlineSeconds":1800,"template":{"metadata":{"labels":{"ayni.io/runtime-operation":name}},"spec":{
            "restartPolicy":"Never","serviceAccountName":runtime.service_account,"automountServiceAccountToken":false,"imagePullSecrets":pulls,
            "nodeSelector":{"kubernetes.io/os":"linux","kubernetes.io/arch":runtime.platform.split('/').nth(1)},
            "securityContext":{"fsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},
            "initContainers":[
                {"name":"launcher","image":configuration.worker_image,"args":["install"],"securityContext":security,"resources":resources,"volumeMounts":[{"name":"launcher","mountPath":"/launcher"}]},
                {"name":"checkout","image":configuration.worker_image,"args":["checkout"],"workingDir":"/workspace","securityContext":security,"resources":resources,
                 "env":[{"name":"SOURCE_TOKEN","valueFrom":{"secretKeyRef":{"name":name,"key":"token"}}}],"volumeMounts":[workspace,scratch,request]}
            ],
            "containers":[{"name":"verify","image":runtime.image,"imagePullPolicy":"Always","args":["/launcher/runtime-worker","verify"],"workingDir":"/workspace","securityContext":security,"resources":resources,"env":runtime_environment(),
                "terminationMessagePolicy":"File","volumeMounts":[workspace,scratch,request,{"name":"launcher","mountPath":"/launcher","readOnly":true}]}],
            "volumes":[{"name":"workspace","emptyDir":{"sizeLimit":"4Gi"}},{"name":"scratch","emptyDir":{"sizeLimit":"4Gi"}},{"name":"launcher","emptyDir":{"sizeLimit":"128Mi"}},{"name":"request","configMap":{"name":name}}]
        }}}})).map_err(|_| "runtime_job_invalid")
}

pub fn runtime_environment() -> serde_json::Value {
    json!([
        {"name":"HOME","value":"/scratch"}, {"name":"TMPDIR","value":"/scratch"},
        {"name":"CARGO_HOME","value":"/scratch/cargo"},
        {"name":"MISE_CACHE_DIR","value":"/scratch/mise-cache"}, {"name":"MISE_STATE_DIR","value":"/scratch/mise-state"}
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verification_preserves_entrypoint_and_limits_credentials_to_checkout()
    -> Result<(), &'static str> {
        let configuration = RuntimeConfiguration {
            namespace: "test".into(),
            worker_image: "worker".into(),
            output_claim: "output".into(),
            model_secret: "model".into(),
            interval_seconds: 60,
            request_timeout_seconds: 30,
            concurrency: 4,
            profiles: std::collections::BTreeMap::default(),
        };
        let runtime = ProfileRuntime {
            assessment: None,
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
        assert_eq!(pod.automount_service_account_token, Some(false));
        assert!(pod.containers[0].command.is_none());
        assert!(
            pod.containers[0]
                .env
                .as_ref()
                .is_some_and(|env| env.iter().all(|e| e.value_from.is_none()))
        );
        assert_eq!(pod.service_account_name.as_deref(), Some("runtime"));
        assert!(
            pod.volumes
                .ok_or("volumes_missing")?
                .iter()
                .all(|v| v.host_path.is_none())
        );
        Ok(())
    }
}
