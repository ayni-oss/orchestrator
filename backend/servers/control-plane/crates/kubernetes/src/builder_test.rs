//! Trusted fixture Jobs share deployment topology without repository checkout authority.
use crate::{execution::RuntimeKubernetes, workload};
use ayni_orchestrator_application::execution::WorkloadRuntime;
use ayni_orchestrator_runtime::runtime_execution::ExecutionRequest;
use k8s_openapi::api::{batch::v1::Job, core::v1::Pod};
use kube::{
    Api,
    api::{ListParams, LogParams},
};
use serde::Serialize;
use serde_json::{Value, json};
impl RuntimeKubernetes {
    /// Pull and verify a reusable builder without signing or push authority.
    /// # Errors
    /// Rejects invalid immutable image identities.
    pub async fn start_builder_probe(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
    ) -> Result<(), &'static str> {
        let mut container =
            workload::worker_container("verify", &runtime.image, &workload::runtime_environment());
        container["args"] = json!(["/launcher/runtime-worker", "builder-probe"]);
        let mut job = workload::source_job(
            name,
            &self.configuration,
            runtime,
            "runtime-coding",
            &container,
        );
        workload::without_checkout(&mut job)?;
        job["spec"]["template"]["spec"]["nodeSelector"] =
            workload::node_selector(&runtime.platform, false);
        self.create(
            name,
            std::collections::BTreeMap::from([("execution.json".into(), "{}".into())]),
            "",
            &serde_json::from_value(job).map_err(|_| "image_manifest_invalid")?,
        )
        .await
    }
    /// Collect bounded public fixture inputs from the completed build.
    /// # Errors
    /// Rejects missing, oversized, or malformed results.
    pub async fn builder_test_result(&self, name: &str) -> Result<Value, &'static str> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let list = pods
            .list(
                &ListParams::default()
                    .labels(&format!("ayni.io/runtime-operation={name}"))
                    .limit(2),
            )
            .await
            .map_err(|_| "kubernetes_unavailable")?;
        let [pod] = list.items.as_slice() else {
            return Err("runtime_pod_identity_invalid");
        };
        let log = pods
            .logs(
                pod.metadata
                    .name
                    .as_deref()
                    .ok_or("runtime_pod_identity_invalid")?,
                &LogParams {
                    container: Some("build".into()),
                    limit_bytes: Some(1_048_576),
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| "runtime_result_missing")?;
        let result = log
            .lines()
            .find_map(|line| line.strip_prefix("AYNI_BUILDER_TEST_RESULT="))
            .ok_or("runtime_result_missing")?;
        serde_json::from_str(result).map_err(|_| "runtime_result_invalid")
    }
    /// Verify a published fixture with pull-only credentials.
    /// # Errors
    /// Rejects invalid requests or Kubernetes resource conflicts.
    pub async fn start_builder_verification<P: Serialize + Sync>(
        &self,
        name: &str,
        runtime: &WorkloadRuntime,
        request: &ExecutionRequest<P>,
        lock: &str,
        key: &str,
        public: &str,
    ) -> Result<(), &'static str> {
        let mut env = workload::runtime_environment();
        env.extend([
            json!({"name":"CERTIFICATE_KEY_ID","value":key}),
            json!({"name":"CERTIFICATE_PUBLIC_KEY","value":public}),
        ]);
        let mut container = workload::worker_container("verify", &runtime.image, &env);
        container["args"] = json!(["/launcher/runtime-worker", "builder-test-verify"]);
        container["volumeMounts"].as_array_mut().ok_or("engine_manifest_invalid")?.push(json!({"name":"request","mountPath":"/request/fixture.lock","subPath":"fixture.lock","readOnly":true}));
        let mut job = workload::source_job(
            name,
            &self.configuration,
            runtime,
            "runtime-coding",
            &container,
        );
        job["spec"]["template"]["spec"]["initContainers"]
            .as_array_mut()
            .ok_or("engine_manifest_invalid")?
            .retain(|c| c["name"] == "launcher");
        let job: Job = serde_json::from_value(job).map_err(|_| "engine_manifest_invalid")?;
        self.create(
            name,
            std::collections::BTreeMap::from([
                (
                    "execution.json".into(),
                    serde_json::to_string(request).map_err(|_| "runtime_request_invalid")?,
                ),
                ("fixture.lock".into(), lock.into()),
            ]),
            "",
            &job,
        )
        .await
    }
}
