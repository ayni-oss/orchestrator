//! Bounded concurrent desired-state supervisor; `PostgreSQL` owns dispatch and fencing.
#[path = "../task_supervisor/pods.rs"]
mod pods;
use ayni_orchestrator_events::task_execution::{SupervisorRequest, TaskRun};
use ayni_orchestrator_observability::workflow::{
    Workflow, finish, identity_context, link_recovery,
};
use std::{env, fs, io, time::Duration};
use tracing::Instrument as _;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub struct Supervisor {
    http: reqwest::Client,
    url: String,
    token: String,
    namespace: String,
    image: String,
    fixture: bool,
    openai: bool,
    anthropic: bool,
    anthropic_endpoint: String,
    endpoint: String,
}
impl Supervisor {
    async fn call(&self, request: &SupervisorRequest) -> Result<Option<TaskRun>> {
        let mut request_builder = self
            .http
            .post(format!("{}/v1/task-supervisor", self.url))
            .bearer_auth(&self.token)
            .json(request);
        if let Some(context) = ayni_orchestrator_observability::current_context() {
            request_builder = request_builder.header("traceparent", context.traceparent);
        }
        let mut response = request_builder.send().await?.error_for_status()?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len()
                > ayni_orchestrator_events::task_execution::MAX_TASK_SUPERVISOR_RESPONSE_BYTES
            {
                return Err(io::Error::other("task response exceeds bound").into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(serde_json::from_slice(&body)?)
    }
}
fn required(name: &str) -> Result<String> {
    env::var(name).map_err(|_| io::Error::other("task supervisor configuration missing").into())
}
#[tokio::main]
async fn main() -> Result {
    let telemetry = ayni_orchestrator_observability::init("task-supervisor");
    let result = Box::pin(run()).await;
    telemetry.shutdown();
    result
}
async fn run() -> Result {
    let s = Supervisor {
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()?,
        url: required("ORCHESTRATOR_TASK_AUTHORITY_URL")?,
        token: fs::read_to_string(required("ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH")?)?
            .trim()
            .to_owned(),
        namespace: required("ORCHESTRATOR_SESSION_NAMESPACE")?,
        image: required("ORCHESTRATOR_TASK_BOOTSTRAP_IMAGE")?,
        fixture: env::var("ORCHESTRATOR_TASK_FIXTURE_ENABLED").as_deref() == Ok("true"),
        openai: env::var("ORCHESTRATOR_OPENAI_ENABLED").as_deref() == Ok("true"),
        anthropic: env::var("ORCHESTRATOR_ANTHROPIC_ENABLED").as_deref() == Ok("true"),
        anthropic_endpoint: env::var("ANTHROPIC_BASE_URL")
            .unwrap_or_else(|_| "https://api.anthropic.com/v1".into()),
        endpoint: env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
    };
    let client = kube::Client::try_default().await?;
    loop {
        let operation = async {
            if let Some(run) = s.call(&SupervisorRequest::Poll).await? {
                let c = &run.command;
                let parent = c
                    .recovery
                    .as_ref()
                    .map_or("", |recovery| recovery.parent_attempt_id.as_str());
                let span = Workflow {
                    workspace_id: c.workspace_id.clone(),
                    project_id: c.project_id.clone(),
                    conversation_id: c.conversation_id.clone(),
                    task_id: c.task_id.clone(),
                    attempt_id: c.attempt_id.clone(),
                    plan_id: c.plan_id.clone(),
                    generation: run.generation,
                    parent_attempt_id: parent.to_owned(),
                    ..Workflow::default()
                }
                .span_with_context(
                    "task.supervisor.reconcile",
                    identity_context(&c.attempt_id).as_ref(),
                    false,
                );
                span.record("execution", run.state.as_str());
                link_recovery(&span, parent);
                let reconciled = pods::reconcile(&s, client.clone(), &run)
                    .instrument(span.clone())
                    .await;
                finish(&span, reconciled.is_ok());
                reconciled?;
            }
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        };
        tokio::select! {result=operation=>{if result.is_err(){eprintln!("task supervision pending recovery");}},_=tokio::signal::ctrl_c()=>return Ok(())}
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
