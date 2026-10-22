//! Trusted local operator: verify immutable Git inputs, then submit retained bytes.
#[path = "task_preparation/git.rs"]
mod git;
use ayni_orchestrator_api::v1::{
    self, project_conversation_service_client::ProjectConversationServiceClient,
};
use std::{
    error::Error,
    fs::OpenOptions,
    io::{self, Read as _, Write as _},
    net::SocketAddr,
    path::Path,
    process::ExitCode,
    time::Duration,
};
use tonic::transport::Endpoint;
use uuid::Uuid;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("task preparation failed: {error}");
            ExitCode::FAILURE
        }
    }
}
async fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !matches!(args.first().map(String::as_str), Some("verify" | "submit"))
        || (args[0] == "verify" && args.len() != 5)
        || (args[0] == "submit" && args.len() != 3)
    {
        return Err(io::Error::other("usage: workflow-prepare-task verify LOOPBACK_ADDR CHECKOUT SELECTION_JSON FROZEN_REQUEST_JSON | submit LOOPBACK_ADDR FROZEN_REQUEST_JSON").into());
    }
    let address: SocketAddr = args[1].parse()?;
    if !address.ip().is_loopback() {
        return Err(io::Error::other("trusted loopback endpoint required").into());
    }
    let channel = Endpoint::from_shared(format!("http://{address}"))?
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .connect()
        .await?;
    let mut client = ProjectConversationServiceClient::new(channel);
    if args[0] == "submit" {
        let request = read_request(Path::new(&args[2]))?;
        let result = client.prepare_task_execution(request).await?.into_inner();
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    let mut request = read_request(Path::new(&args[3]))?;
    let task = client
        .get_task(v1::GetTaskRequest {
            workspace_id: request.workspace_id.clone(),
            project_id: request.project_id.clone(),
            conversation_id: request.conversation_id.clone(),
            task_id: request.task_id.clone(),
        })
        .await?
        .into_inner()
        .task
        .ok_or_else(|| io::Error::other("task missing"))?;
    let specification = task
        .specification
        .ok_or_else(|| io::Error::other("task specification missing"))?;
    let inputs = request.inputs.get_or_insert_default();
    if (!inputs.repository.is_empty() && inputs.repository != specification.repository)
        || (!inputs.source_ref.is_empty() && inputs.source_ref != specification.source_ref)
    {
        return Err(
            io::Error::other("selection must preserve the approved repository and source").into(),
        );
    }
    let (commit, lock) = git::verify(
        Path::new(&args[2]),
        &specification.repository,
        &specification.source_ref,
    )?;
    inputs.repository = specification.repository;
    inputs.source_ref = specification.source_ref;
    inputs.source_commit = commit;
    inputs.lock_digest = lock;
    if request.idempotency_key.is_empty() {
        request.idempotency_key = Uuid::now_v7().to_string();
    }
    let bytes = serde_json::to_vec_pretty(&request)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[4])?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    println!(
        "Frozen request written. Review runtime and budgets, then submit this same file; preserve it for ambiguous retries."
    );
    Ok(())
}
fn read_request(path: &Path) -> Result<v1::PrepareTaskExecutionRequest, Box<dyn Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(16385)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16384 {
        return Err(io::Error::other("request exceeds 16 KiB").into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
