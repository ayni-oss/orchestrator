//! Exact stage capability transport. No token, credential or provider response is logged.
use crate::Failure;
use ayni_orchestrator_events::execution_diagnostics::BuildOutput;
use ayni_orchestrator_events::runtime_preparation::{
    Binding, BuildMilestone, MAX_SUPERVISOR_RESPONSE_BYTES, StageClaim, StageReport, WorkerRequest,
    WorkerResponse,
};
use serde::{Deserialize, de::DeserializeOwned};
use std::time::Duration;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Credential {
    GithubInstallation { token: String },
    Registry { username: String, password: String },
}
pub struct WorkerClient {
    http: reqwest::Client,
    url: String,
    capability: String,
    pub binding: Binding,
}
impl WorkerClient {
    /// # Errors
    /// Reject absent identity, malformed authority origin or unavailable transport.
    pub fn new(origin: &str, capability: String, binding: Binding) -> Result<Self, Failure> {
        let origin = reqwest::Url::parse(origin).map_err(|_| Failure::InvalidInput)?;
        if !matches!(origin.scheme(), "http" | "https")
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
            || uuid::Uuid::parse_str(&binding.operation_id).is_err()
            || uuid::Uuid::parse_str(&binding.pod_uid).is_err()
            || !ayni_orchestrator_events::runtime_preparation::STAGES
                .contains(&binding.stage.as_str())
            || binding.generation == 0
            || capability.len() != 64
        {
            return Err(Failure::InvalidInput);
        }
        let url = format!(
            "{}/v1/runtime-worker/{}/{}/{}",
            origin.as_str().trim_end_matches('/'),
            binding.operation_id,
            binding.stage,
            binding.generation
        );
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| Failure::ExecutionFailed)?,
            url,
            capability,
            binding,
        })
    }
    async fn call<T: DeserializeOwned>(&self, request: &WorkerRequest) -> Result<T, Failure> {
        self.request(&self.url, Some(request)).await
    }
    async fn request<T: DeserializeOwned>(
        &self,
        url: &str,
        body: Option<&WorkerRequest>,
    ) -> Result<T, Failure> {
        let mut request = self
            .http
            .post(url)
            .bearer_auth(&self.capability)
            .header("x-runtime-pod-uid", &self.binding.pod_uid)
            .header("x-runtime-command-digest", &self.binding.command_digest);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(|_| Failure::ExecutionFailed)?;
        if !response.status().is_success() {
            return Err(Failure::ExecutionFailed);
        }
        let body =
            ayni_orchestrator_registry::bounded_body(response, MAX_SUPERVISOR_RESPONSE_BYTES)
                .await
                .map_err(|_| Failure::ExecutionFailed)?;
        serde_json::from_slice(&body).map_err(|_| Failure::ExecutionFailed)
    }
    /// Best-effort preview. A failed update never changes the build outcome.
    pub async fn build_progress(&self, milestone: BuildMilestone) {
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            self.call::<WorkerResponse>(&WorkerRequest::BuildProgress { milestone }),
        )
        .await;
    }

    pub(crate) async fn build_output(&self, output: BuildOutput) -> bool {
        let request = WorkerRequest::BuildOutput { output };
        for _ in 0..2 {
            if let Ok(Ok(response)) = tokio::time::timeout(
                Duration::from_millis(750),
                self.call::<WorkerResponse>(&request),
            )
            .await
                && response.accepted
            {
                return true;
            }
        }
        false
    }

    /// # Errors
    /// Inputs are available only after exact observed Pod UID binding.
    pub async fn input(&self) -> Result<StageClaim, Failure> {
        self.call::<WorkerResponse>(&WorkerRequest::Input)
            .await?
            .claim
            .ok_or(Failure::ExecutionFailed)
    }
    /// # Errors
    /// The authority selects exactly one credential allowed to this frozen stage.
    pub async fn credential(&self) -> Result<Credential, Failure> {
        self.request(&format!("{}/credential", self.url), None)
            .await
    }
    /// # Errors
    /// No registry side effect may precede this accepted immutable intent.
    pub async fn prepare_publication(&self, digest: &str) -> Result<(), Failure> {
        let response: WorkerResponse = self
            .call(&WorkerRequest::PreparePublication {
                expected_manifest_digest: digest.into(),
            })
            .await?;
        if !response.accepted {
            return Err(Failure::ExecutionFailed);
        }
        Ok(())
    }
    /// # Errors
    /// Conflicts and lost responses preserve the operation; a retry replays exact bytes.
    pub async fn report(&self, report: StageReport) -> Result<(), Failure> {
        if report.binding != self.binding {
            return Err(Failure::InvalidInput);
        }
        let response: WorkerResponse = self
            .call(&WorkerRequest::Report {
                report: Box::new(report),
            })
            .await?;
        if !response.accepted {
            return Err(Failure::ExecutionFailed);
        }
        Ok(())
    }
}
