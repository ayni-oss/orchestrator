//! Isolated public trust-policy preparation with no signing or registry-push authority.
use crate::{execution::RuntimeKubernetes, workload};
use ayni_orchestrator_application::execution::WorkloadRuntime;
use ayni_orchestrator_runtime::{
    runtime_contract::RuntimeImage, runtime_execution::ExecutionRequest,
};
use k8s_openapi::api::{batch::v1::Job, core::v1::Pod};
use kube::{
    Api,
    api::{ListParams, LogParams},
};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;

impl RuntimeKubernetes {
    /// Creates an isolated job that can only prepare public policy and lock files.
    /// # Errors
    /// Returns a safe reason for missing build configuration or conflicting Kubernetes resources.
    pub async fn start_trust_lock<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<P>,
        token: &str,
        key_id: &str,
        public_key: &str,
    ) -> Result<(), &'static str> {
        let job = trust_lock_job(name, &self.configuration, runtime, key_id, public_key)?;
        self.create(
            name,
            BTreeMap::from([(
                "execution.json".into(),
                serde_json::to_string(request).map_err(|_| "trust_lock_request_invalid")?,
            )]),
            token,
            &job,
        )
        .await
    }

    /// Reads bounded public output from the completed owned worker pod.
    /// # Errors
    /// Rejects incomplete jobs, unrelated pods, unavailable logs, or missing output.
    pub async fn trust_lock_result(&self, name: &str) -> Result<String, &'static str> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let job = jobs.get(name).await.map_err(|_| "trust_lock_job_missing")?;
        if job
            .status
            .as_ref()
            .is_none_or(|status| status.succeeded.unwrap_or(0) == 0)
        {
            return Err("trust_lock_job_incomplete");
        }
        let uid = job
            .metadata
            .uid
            .as_deref()
            .ok_or("trust_lock_job_invalid")?;
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let list = pods
            .list(&ListParams::default().labels(&format!("ayni.io/runtime-operation={name}")))
            .await
            .map_err(|_| "trust_lock_result_unavailable")?;
        let pod = list
            .items
            .into_iter()
            .find(|pod| {
                pod.metadata
                    .owner_references
                    .as_ref()
                    .is_some_and(|refs| refs.iter().any(|owner| owner.uid == uid))
            })
            .ok_or("trust_lock_result_invalid")?;
        let logs = pods
            .logs(
                pod.metadata
                    .name
                    .as_deref()
                    .ok_or("trust_lock_result_invalid")?,
                &LogParams {
                    container: Some("trust-lock".into()),
                    limit_bytes: Some(2_100_000),
                    ..LogParams::default()
                },
            )
            .await
            .map_err(|_| "trust_lock_result_unavailable")?;
        logs.lines()
            .find_map(|line| line.strip_prefix("AYNI_TRUST_RESULT="))
            .map(str::to_owned)
            .ok_or("trust_lock_result_invalid")
    }
}

fn trust_lock_job(
    name: &str,
    configuration: &crate::configuration::ExecutionConfiguration,
    runtime: &WorkloadRuntime,
    key_id: &str,
    public_key: &str,
) -> Result<Job, &'static str> {
    if RuntimeImage::parse(&configuration.worker_image).is_err() {
        return Err("trust_lock_image_unpinned");
    }
    let mut environment = workload::runtime_environment();
    environment.push(json!({"name":"CERTIFICATE_KEY_ID","value":key_id}));
    environment.push(json!({"name":"CERTIFICATE_PUBLIC_KEY","value":public_key}));
    let container =
        workload::worker_container("trust-lock", &configuration.worker_image, &environment);
    let job: Job = serde_json::from_value(workload::source_job(
        name,
        configuration,
        runtime,
        &runtime.service_account,
        &container,
    ))
    .map_err(|_| "trust_lock_job_invalid")?;
    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{BuildRuntime, ExecutionConfiguration};
    use serde_json::Value;

    fn configuration() -> ExecutionConfiguration {
        ExecutionConfiguration {
            namespace: "test".into(),
            worker_image: format!("worker@sha256:{}", "a".repeat(64)),
            output_claim: "outputs-private".into(),
            model_secret: "model-private".into(),
            allow_emulation: false,
        }
    }
    fn runtime() -> WorkloadRuntime {
        WorkloadRuntime {
            image: "runtime:lock".into(),
            platform: "linux/amd64".into(),
            service_account: "runtime-coding".into(),
            pull_secret: Some("pull-only".into()),
            build: Some(BuildRuntime {
                image: format!("builder@sha256:{}", "b".repeat(64)),
                service_account: "runtime-builder".into(),
                registry_secret: "push-private".into(),
                signing_secret: "seed-private".into(),
                engine_secret: "engine-private".into(),
                engine_host: "tcp://engine:2376".into(),
                isolated_engine: true,
            }),
        }
    }
    fn manifest() -> Result<Value, &'static str> {
        serde_json::to_value(trust_lock_job(
            "trust-one",
            &configuration(),
            &runtime(),
            "release",
            &"a".repeat(64),
        )?)
        .map_err(|_| "invalid_json")
    }

    #[test]
    fn public_trust_job_has_only_checkout_authority() -> Result<(), &'static str> {
        let manifest = manifest()?;
        let rendered = manifest.to_string();
        for forbidden in [
            "push-private",
            "seed-private",
            "engine-private",
            "model-private",
            "outputs-private",
            "runtime-builder",
            "DOCKER_HOST",
            "AYNI_ENV_CERTIFICATE_SIGNING_KEY",
        ] {
            assert!(!rendered.contains(forbidden), "{forbidden}");
        }
        let pod = &manifest["spec"]["template"]["spec"];
        assert_eq!(pod["serviceAccountName"], "runtime-coding");
        assert_eq!(pod["automountServiceAccountToken"], false);
        let containers = pod["containers"].as_array().ok_or("missing_containers")?;
        assert_eq!(containers.len(), 1);
        assert_eq!(containers[0]["image"], configuration().worker_image);
        assert_eq!(pod["imagePullSecrets"], json!([{"name":"pull-only"}]));
        for container in containers
            .iter()
            .chain(pod["initContainers"].as_array().ok_or("missing_init")?)
        {
            let environments = container["env"].as_array();
            let token_refs = environments
                .into_iter()
                .flatten()
                .filter(|env| !env["valueFrom"]["secretKeyRef"].is_null())
                .collect::<Vec<_>>();
            if container["name"] == "checkout" {
                assert_eq!(token_refs.len(), 1);
                assert_eq!(token_refs[0]["name"], "SOURCE_TOKEN");
                assert_eq!(
                    token_refs[0]["valueFrom"]["secretKeyRef"],
                    json!({"name":"trust-one","key":"token"})
                );
            } else {
                assert!(token_refs.is_empty());
            }
            assert!(container["envFrom"].is_null());
        }
        for volume in pod["volumes"].as_array().ok_or("missing_volumes")? {
            assert!(volume["secret"].is_null());
            assert!(volume["hostPath"].is_null());
            assert!(volume["persistentVolumeClaim"].is_null());
        }
        Ok(())
    }

    #[test]
    fn public_trust_launcher_request_and_root_are_read_only() -> Result<(), &'static str> {
        let manifest = manifest()?;
        let pod = &manifest["spec"]["template"]["spec"];
        let container = &pod["containers"][0];
        assert_eq!(
            container["args"],
            json!(["/launcher/runtime-worker", "trust-lock"])
        );
        for volume in ["launcher", "request"] {
            let mount = container["volumeMounts"]
                .as_array()
                .ok_or("missing_mounts")?
                .iter()
                .find(|mount| mount["name"] == volume)
                .ok_or("missing_readonly_mount")?;
            assert_eq!(mount["readOnly"], true);
        }
        assert_eq!(container["securityContext"]["readOnlyRootFilesystem"], true);
        assert_eq!(
            container["securityContext"]["allowPrivilegeEscalation"],
            false
        );
        assert_eq!(container["securityContext"]["runAsNonRoot"], true);
        assert_eq!(
            container["securityContext"]["capabilities"]["drop"],
            json!(["ALL"])
        );
        assert_eq!(
            pod["securityContext"]["seccompProfile"]["type"],
            "RuntimeDefault"
        );
        Ok(())
    }

    #[test]
    fn trust_job_requires_only_a_pinned_worker_and_accepts_no_builder() {
        let mut runtime = runtime();
        runtime.build = None;
        assert!(
            trust_lock_job(
                "trust-one",
                &configuration(),
                &runtime,
                "release",
                &"a".repeat(64)
            )
            .is_ok()
        );
        let mut config = configuration();
        config.worker_image = "worker:latest".into();
        assert!(matches!(
            trust_lock_job("trust-one", &config, &runtime, "release", &"a".repeat(64)),
            Err("trust_lock_image_unpinned")
        ));
    }
}
