//! Trusted file-proposal worker. Checkout source never supplies executable commands.
#[path = "../task_worker/execute.rs"]
mod execute;
#[cfg(target_os = "linux")]
#[path = "../task_worker/filesystem.rs"]
mod filesystem;
#[path = "../task_worker/ledger.rs"]
mod ledger;
#[cfg(target_os = "linux")]
#[path = "../task_worker/network.rs"]
mod network;
#[path = "../task_worker/quality.rs"]
mod quality;
use ayni_orchestrator_conversation_supervisor::task_files;
use ayni_orchestrator_events::task_execution::{TaskCommand, WorkerRequest, WorkerResponse};
use ayni_orchestrator_observability::workflow::{
    Workflow, finish, identity_context, link_recovery,
};
use std::{
    env, fs,
    io::{self, Write as _},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};
use tracing::Instrument as _;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub struct Worker {
    http: reqwest::Client,
    url: String,
    token: String,
    uid: String,
    root: PathBuf,
    source: PathBuf,
}
impl Worker {
    async fn call(&self, request: &WorkerRequest) -> Result<WorkerResponse> {
        let mut request_builder = self
            .http
            .post(&self.url)
            .bearer_auth(&self.token)
            .header("x-task-pod-uid", &self.uid)
            .json(request);
        if let Some(context) = ayni_orchestrator_observability::current_context() {
            request_builder = request_builder.header("traceparent", context.traceparent);
        }
        let mut response = request_builder.send().await?.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 40 * 1024 * 1024 {
                return Err(io::Error::other("task authority response limit").into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}
fn main() -> ExitCode {
    let result = match env::args().nth(1).as_deref() {
        Some("quality-child") => quality::child(&env::args().nth(2).unwrap_or_default()),
        Some("quality-prepare") => quality::launch("prepare"),
        Some("quality-run") => quality::launch("quality"),
        Some("quality-seal") => quality::seal(),
        _ => asynchronous(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if env::args().nth(1).as_deref() == Some("quality-child") {
                eprintln!("quality sandbox setup failed: {error}");
            } else {
                eprintln!("task worker stopped; retained source and ledger require inspection");
            }
            ExitCode::FAILURE
        }
    }
}
#[tokio::main]
async fn asynchronous() -> Result {
    let telemetry = ayni_orchestrator_observability::init("task-worker");
    let result = run().await;
    telemetry.shutdown();
    result
}

fn required(name: &str) -> Result<String> {
    env::var(name).map_err(|_| io::Error::other("task worker configuration missing").into())
}
async fn run() -> Result {
    let attempt = required("ORCHESTRATOR_TASK_ATTEMPT")?;
    uuid::Uuid::parse_str(&attempt)?;
    let generation = required("ORCHESTRATOR_TASK_GENERATION")?.parse::<u64>()?;
    let worker = Worker {
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?,
        url: format!(
            "{}/v1/task-worker/{attempt}/{generation}",
            required("ORCHESTRATOR_TASK_AUTHORITY_URL")?
        ),
        token: fs::read_to_string(required("ORCHESTRATOR_TASK_TOKEN_PATH")?)?
            .trim()
            .to_owned(),
        uid: required("ORCHESTRATOR_TASK_POD_UID")?,
        root: PathBuf::from(required("ORCHESTRATOR_TASK_LEDGER_ROOT")?),
        source: PathBuf::from(required("ORCHESTRATOR_TASK_SOURCE")?),
    };
    fs::create_dir_all(&worker.root)?;
    if env::args().nth(1).as_deref() == Some("initialize") {
        return initialize(&worker).await;
    }
    let command: TaskCommand =
        serde_json::from_slice(&fs::read(worker.root.join("command.json"))?)?;
    let c = &command;
    let parent = c
        .recovery
        .as_ref()
        .map_or("", |recovery| recovery.parent_attempt_id.as_str());
    let stage = match env::args().nth(1).as_deref() {
        Some("quality-freeze") => "task.quality.freeze",
        Some("quality-collect") => "task.quality.collect",
        _ => "task.coding.execute",
    };
    let span = Workflow {
        workspace_id: c.workspace_id.clone(),
        project_id: c.project_id.clone(),
        conversation_id: c.conversation_id.clone(),
        task_id: c.task_id.clone(),
        attempt_id: c.attempt_id.clone(),
        plan_id: c.plan_id.clone(),
        generation: c.generation,
        parent_attempt_id: parent.to_owned(),
        ..Workflow::default()
    }
    .span_with_context(stage, identity_context(&c.attempt_id).as_ref(), false);
    link_recovery(&span, parent);
    let executed = execute_command(&worker, &command)
        .instrument(span.clone())
        .await;
    finish(&span, executed.is_ok());
    executed
}
async fn execute_command(worker: &Worker, command: &TaskCommand) -> Result {
    let stage = env::args().nth(1);
    match stage.as_deref() {
        Some("quality-freeze") => return quality::freeze(worker, command),
        Some("quality-collect") => return quality::collect(worker).await,
        _ => {}
    }
    let mut ledger = ledger::Ledger::open(&worker.root, command)?;
    let outcome = if let Some(result) = ledger.outcome()? {
        ayni_orchestrator_observability::workflow::record_replay(
            &tracing::Span::current(),
            identity_context(&command.attempt_id).as_ref(),
        );
        result
    } else {
        let result = execute::execute(worker, command, &mut ledger).await;
        ledger.finish(&result)?;
        result
    };
    for _ in 0..30 {
        if worker
            .call(&WorkerRequest::Report {
                outcome: outcome.clone(),
            })
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err("task result awaits application acceptance".into())
}

async fn initialize(worker: &Worker) -> Result {
    let mut input = None;
    for _ in 0..60 {
        if let Ok(response) = worker.call(&WorkerRequest::Input).await {
            input = Some(response);
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let response = input.ok_or("task input unavailable")?;
    let command = response.command.ok_or("task command missing")?;
    fs::write(worker.root.join("source.pack"), &response.source_pack)?;
    let manifest = worker.root.join("command.json");
    if manifest.exists() {
        let old: TaskCommand = serde_json::from_slice(&fs::read(&manifest)?)?;
        if old != command {
            return Err("task input identity mismatch".into());
        }
    } else {
        let span = Workflow {
            workspace_id: command.workspace_id.clone(),
            project_id: command.project_id.clone(),
            conversation_id: command.conversation_id.clone(),
            task_id: command.task_id.clone(),
            attempt_id: command.attempt_id.clone(),
            plan_id: command.plan_id.clone(),
            generation: command.generation,
            ..Workflow::default()
        }
        .span_with_context(
            "task.source.prepare",
            identity_context(&command.attempt_id).as_ref(),
            false,
        );
        let prepared = span.in_scope(|| {
            task_files::checkout_for_coding(&worker.source, &response.source_pack, &command)
        });
        finish(&span, prepared.is_ok());
        if prepared.is_err() {
            fs::create_dir_all(&worker.source)?;
            let ledger = ledger::Ledger::open(&worker.root, &command)?;
            ledger.finish(&ayni_orchestrator_events::task_execution::TaskOutcome {
                state: "failed".into(),
                safe_reason: "source_invalid".into(),
                ..Default::default()
            })?;
        }
        let bytes = serde_json::to_vec(&command)?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(worker.root.join("command.pending"))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(worker.root.join("command.pending"), manifest)?;
        fs::File::open(&worker.root)?.sync_all()?;
    }
    let executable = env::current_exe()?;
    fs::copy(executable, "/opt/ayni-task/worker")?;
    Ok(())
}
