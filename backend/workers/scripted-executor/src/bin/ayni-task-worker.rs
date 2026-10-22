//! Trusted file-proposal worker. Checkout source never supplies executable commands.
#[path = "../task_worker/execute.rs"]
mod execute;
#[path = "../task_worker/ledger.rs"]
mod ledger;
use ayni_orchestrator_events::task_execution::{TaskCommand, WorkerRequest, WorkerResponse};
use ayni_orchestrator_scripted_executor::task_files;
use std::{
    env, fs,
    io::{self, Write as _},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};
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
        let mut response = self
            .http
            .post(&self.url)
            .bearer_auth(&self.token)
            .header("x-task-pod-uid", &self.uid)
            .json(request)
            .send()
            .await?
            .error_for_status()?;
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
#[tokio::main]
async fn main() -> ExitCode {
    if run().await.is_ok() {
        ExitCode::SUCCESS
    } else {
        eprintln!("task worker stopped; retained source and ledger require inspection");
        ExitCode::FAILURE
    }
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
    let mut ledger = ledger::Ledger::open(&worker.root, &command)?;
    let outcome = if let Some(result) = ledger.outcome()? {
        result
    } else {
        let result = execute::execute(&worker, &command, &mut ledger).await;
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
    let manifest = worker.root.join("command.json");
    if manifest.exists() {
        let old: TaskCommand = serde_json::from_slice(&fs::read(&manifest)?)?;
        if old != command {
            return Err("task input identity mismatch".into());
        }
    } else {
        if task_files::checkout(&worker.source, &response.source_pack, &command).is_err() {
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
