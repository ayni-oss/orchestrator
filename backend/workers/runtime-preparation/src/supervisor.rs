//! Reconcile exact generation-bound Kubernetes workloads; never advance before removal.
use crate::{Failure, client::WorkerClient};
use ayni_orchestrator_events::runtime_preparation::{
    Binding, StageClaim, SupervisorRequest, worker_token,
};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    Api, Client,
    api::{DeleteParams, PostParams, Preconditions},
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    env, fs,
    sync::Mutex,
    time::{Duration, Instant},
};

mod evidence;
mod pods;
mod resources;

/// Supervisor-only authority is never inserted into a stage workload.
pub struct Supervisor {
    client: Client,
    http: reqwest::Client,
    namespace: String,
    origin: String,
    authority: String,
    storage_class: Option<String>,
    pull_policy: String,
    last_failure: Mutex<BTreeMap<String, Instant>>,
}
impl Supervisor {
    /// # Errors
    /// Fail closed when workload namespace, configured authority or Kubernetes access is absent.
    pub async fn configured() -> Result<Self, Failure> {
        let namespace =
            env::var("ORCHESTRATOR_RUNTIME_NAMESPACE").map_err(|_| Failure::InvalidInput)?;
        if namespace.is_empty() || namespace.len() > 63 {
            return Err(Failure::InvalidInput);
        }
        let origin =
            env::var("ORCHESTRATOR_RUNTIME_AUTHORITY_URL").map_err(|_| Failure::InvalidInput)?;
        let parsed = reqwest::Url::parse(&origin).map_err(|_| Failure::InvalidInput)?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(Failure::InvalidInput);
        }
        let path = env::var("ORCHESTRATOR_RUNTIME_AUTHORITY_TOKEN_PATH")
            .map_err(|_| Failure::InvalidInput)?;
        let authority = fs::read_to_string(path)
            .map_err(|_| Failure::InvalidInput)?
            .trim()
            .to_owned();
        if !(32..=256).contains(&authority.len()) {
            return Err(Failure::InvalidInput);
        }
        Ok(Self {
            last_failure: Mutex::new(BTreeMap::new()),
            client: Client::try_default()
                .await
                .map_err(|_| Failure::ExecutionFailed)?,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| Failure::ExecutionFailed)?,
            namespace,
            origin: origin.trim_end_matches('/').into(),
            authority,
            storage_class: env::var("ORCHESTRATOR_RUNTIME_STORAGE_CLASS")
                .ok()
                .filter(|s| !s.is_empty()),
            pull_policy: env::var("ORCHESTRATOR_RUNTIME_PULL_POLICY")
                .unwrap_or_else(|_| "IfNotPresent".into()),
        })
    }
    // Only caller-created context is allowed here: never raw HTTP bodies or library errors.
    fn diagnostic(&self, message: String) {
        if let Ok(mut previous) = self.last_failure.lock()
            && previous
                .get(&message)
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
        {
            eprintln!("{message}; retrying retained operation");
            if previous.len() >= 32 {
                previous.clear();
            }
            previous.insert(message, Instant::now());
        }
    }
    fn kubernetes_failure(&self, action: &str, error: &kube::Error) -> Failure {
        let status = match error {
            kube::Error::Api(error) => error.code,
            _ => 0,
        };
        self.diagnostic(format!(
            "runtime Kubernetes request failed: action={action} status={status}"
        ));
        Failure::ExecutionFailed
    }
    async fn request<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &(impl serde::Serialize + Sync),
    ) -> Result<T, Failure> {
        let response = self
            .http
            .post(format!("{}{path}", self.origin))
            .bearer_auth(&self.authority)
            .json(body)
            .send()
            .await
            .map_err(|_| {
                self.diagnostic(format!(
                    "runtime authority request failed: endpoint={path} reason=transport"
                ));
                Failure::ExecutionFailed
            })?;
        if !response.status().is_success() {
            self.diagnostic(format!(
                "runtime authority request failed: endpoint={path} status={}",
                response.status().as_u16()
            ));
            return Err(Failure::ExecutionFailed);
        }
        let bytes = ayni_orchestrator_registry::bounded_body(
            response,
            ayni_orchestrator_events::runtime_preparation::MAX_SUPERVISOR_RESPONSE_BYTES,
        )
        .await
        .map_err(|_| {
            self.diagnostic(format!(
                "runtime authority request failed: endpoint={path} reason=response_body"
            ));
            Failure::ExecutionFailed
        })?;
        serde_json::from_slice(&bytes).map_err(|_| {
            self.diagnostic(format!(
                "runtime authority request failed: endpoint={path} reason=response_schema"
            ));
            Failure::ExecutionFailed
        })
    }
    async fn call(&self, request: &SupervisorRequest) -> Result<Option<StageClaim>, Failure> {
        self.request("/v1/runtime-supervisor", request).await
    }
    fn binding(claim: &StageClaim, uid: &str) -> Binding {
        Binding {
            operation_id: claim.operation.id.clone(),
            stage: claim.attempt.stage.clone(),
            generation: claim.attempt.generation,
            pod_uid: uid.into(),
            command_digest: claim.attempt.command_digest.clone(),
        }
    }
    fn capability(&self, claim: &StageClaim) -> String {
        worker_token(
            self.authority.as_bytes(),
            &claim.operation.id,
            &claim.attempt.stage,
            claim.attempt.generation,
            &claim.attempt.command_digest,
        )
    }
    fn worker(&self, claim: &StageClaim, uid: &str) -> Result<WorkerClient, Failure> {
        WorkerClient::new(
            &self.origin,
            self.capability(claim),
            Self::binding(claim, uid),
        )
    }
    /// Reconcile one retained operation, including restart, lost reports and cleanup.
    /// # Errors
    /// Reject foreign or replaced workloads and unavailable authority; retries preserve identity.
    pub async fn tick(&self) -> Result<(), Failure> {
        if resources::retire_replays(self).await.is_err() {
            self.diagnostic("telemetry recovery reconciliation pending".into());
        }
        let Some(claim) = self.call(&SupervisorRequest::Poll).await? else {
            return Ok(());
        };
        self.reconcile(&claim).await.inspect_err(|reason| {
            self.diagnostic(format!(
                "runtime reconciliation failed: operation={} stage={} generation={} reason={}",
                claim.operation.id,
                claim.attempt.stage,
                claim.attempt.generation,
                reason.reason()
            ));
        })
    }
    async fn reconcile(&self, claim: &StageClaim) -> Result<(), Failure> {
        let api: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let name = pods::name(claim)?;
        let pod = api
            .get_opt(&name)
            .await
            .map_err(|error| self.kubernetes_failure("get_pod", &error))?;
        if let Some(pod) = pod {
            pods::verify(&pod, claim)?;
            return self.existing(&api, claim, &pod).await;
        }
        if !claim.attempt.pod_uid.is_empty() {
            if resources::recover_capture(self, claim).await.is_err() {
                self.diagnostic(
                    "telemetry recovery pending; orphan reconciliation will retry".into(),
                );
            }
            if claim.operation.state == "running" {
                let mut diagnostic = ayni_orchestrator_observability::workload::record(
                    ayni_orchestrator_observability::workload::Kind::Runtime,
                    &claim.operation.id,
                    claim.attempt.generation,
                    &claim.attempt.stage,
                    "pod_lost",
                    None,
                );
                diagnostic.pod_uid.clone_from(&claim.attempt.pod_uid);
                self.retain_lifecycle(&diagnostic).await?;
            }
            self.call(&SupervisorRequest::Stopped {
                binding: Self::binding(claim, &claim.attempt.pod_uid),
            })
            .await?;
            return Ok(());
        }
        if matches!(claim.operation.state.as_str(), "stopping" | "cancelling") {
            self.call(&SupervisorRequest::Stopped {
                binding: Self::binding(claim, ""),
            })
            .await?;
            return Ok(());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Failure::ExecutionFailed)?
            .as_millis();
        if claim.attempt.stage == "cleanup" && u128::from(claim.attempt.deadline_unix_ms) <= now {
            self.call(&SupervisorRequest::Stopped {
                binding: Self::binding(claim, ""),
            })
            .await?;
            return Ok(());
        }
        if let Err(reason) = resources::prepare(self, claim).await {
            if reason == Failure::CertificationFailed {
                self.fail_before_start(&api, claim, "certification_failed")
                    .await?;
                tracing::warn!(operation_id = %claim.operation.id, generation = claim.attempt.generation,
                    cause = "signer_identity_rejected_before_execution", "Runtime certification failed");
                return Ok(());
            }
            return Err(reason);
        }
        let expected = pods::render(self, claim)?;
        match api.create(&PostParams::default(), &expected).await {
            Ok(pod) => {
                pods::verify(&pod, claim)?;
                self.bind(claim, &pod).await
            }
            Err(kube::Error::Api(error)) if error.code == 409 => Ok(()),
            Err(kube::Error::Api(error))
                if (400..500).contains(&error.code) && ![408, 429].contains(&error.code) =>
            {
                tracing::warn!(operation_id = %claim.operation.id, generation = claim.attempt.generation,
                    stage = %claim.attempt.stage, status = error.code, "Runtime workload creation rejected");
                self.fail_before_start(
                    &api,
                    claim,
                    if claim.attempt.stage == "build" {
                        "builder_unavailable"
                    } else {
                        "setup_failure"
                    },
                )
                .await
            }
            Err(_) => Err(Failure::ExecutionFailed),
        }
    }
    async fn retain_lifecycle(
        &self,
        diagnostic: &ayni_orchestrator_events::execution_diagnostics::Diagnostic,
    ) -> Result<(), Failure> {
        self.request("/v1/runtime-supervisor/diagnostics", diagnostic)
            .await
    }
    async fn fail_before_start(
        &self,
        api: &Api<Pod>,
        claim: &StageClaim,
        safe_reason: &str,
    ) -> Result<(), Failure> {
        if !claim.attempt.pod_uid.is_empty() {
            return Err(Failure::ExecutionFailed);
        }
        if api
            .get_opt(&pods::name(claim)?)
            .await
            .map_err(|_| Failure::ExecutionFailed)?
            .is_some()
        {
            return Err(Failure::ExecutionFailed);
        }
        self.call(&SupervisorRequest::FailedBeforeStart {
            binding: Self::binding(claim, ""),
            safe_reason: safe_reason.into(),
        })
        .await?;
        Ok(())
    }
    async fn bind(&self, claim: &StageClaim, pod: &Pod) -> Result<(), Failure> {
        let uid = pod.metadata.uid.as_deref().ok_or(Failure::InvalidInput)?;
        self.call(&SupervisorRequest::Bind {
            binding: Self::binding(claim, uid),
        })
        .await?;
        Ok(())
    }
    async fn existing(&self, api: &Api<Pod>, claim: &StageClaim, pod: &Pod) -> Result<(), Failure> {
        if !claim.attempt.pod_uid.is_empty() {
            let _ = resources::arm_recovery(self, claim).await;
        }
        if claim.attempt.pod_uid.is_empty() {
            return self.bind(claim, pod).await;
        }
        if matches!(claim.operation.state.as_str(), "stopping" | "cancelling") {
            if claim.operation.state == "stopping"
                && std::env::var("ORCHESTRATOR_DEBUG").is_ok_and(|v| v == "true")
            {
                // Give the API collector a bounded final pass over terminated containers.
                // Cancellation never waits for diagnostics; retained spools recover separately.
                let finished = pod
                    .status
                    .iter()
                    .flat_map(|s| s.container_statuses.iter().flatten())
                    .filter_map(|s| s.state.as_ref()?.terminated.as_ref()?.finished_at.as_ref())
                    .map(|t| t.0.as_millisecond())
                    .max();
                if finished.is_some_and(|t| {
                    ayni_orchestrator_execution::now_ms().saturating_sub(t) < 60_000
                }) {
                    return Ok(());
                }
            }
            if pod.metadata.deletion_timestamp.is_none() {
                let diagnostic = ayni_orchestrator_observability::workload::record(
                    ayni_orchestrator_observability::workload::Kind::Runtime,
                    &claim.operation.id,
                    claim.attempt.generation,
                    &claim.attempt.stage,
                    "retiring",
                    Some(pod),
                );
                let retained = self.retain_lifecycle(&diagnostic).await;
                if claim.operation.state != "cancelling" {
                    retained?;
                }
            }
            return delete_pod(api, pod).await;
        }
        if claim.attempt.stage == "cleanup" {
            return resources::cleanup(self, claim, pod).await;
        }
        let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
        if matches!(phase, Some("Succeeded" | "Failed")) {
            evidence::finished(self, api, claim, pod).await?;
        }
        Ok(())
    }
}
async fn delete_pod(api: &Api<Pod>, pod: &Pod) -> Result<(), Failure> {
    let name = pod.metadata.name.as_deref().ok_or(Failure::InvalidInput)?;
    let params = DeleteParams {
        preconditions: Some(Preconditions {
            uid: pod.metadata.uid.clone(),
            resource_version: pod.metadata.resource_version.clone(),
        }),
        ..DeleteParams::default()
    };
    match api.delete(name, &params).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
        Err(_) => Err(Failure::CleanupPending),
    }
}

#[derive(Deserialize)]
struct PullCredential {
    kind: String,
    username: String,
    password: String,
}
