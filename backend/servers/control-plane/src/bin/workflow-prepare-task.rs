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
    if !matches!(
        args.first().map(String::as_str),
        Some("verify" | "submit" | "start" | "submit-start")
    ) || (matches!(args[0].as_str(), "verify" | "start") && args.len() != 5)
        || (matches!(args[0].as_str(), "submit" | "submit-start") && args.len() != 3)
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
    match args[0].as_str() {
        "submit-start" => submit_start(&mut client, &args).await,
        "start" => stage_start(&mut client, &args).await,
        "submit" => submit_preparation(&mut client, &args).await,
        _ => verify_preparation(&mut client, &args).await,
    }
}

type Client = ProjectConversationServiceClient<tonic::transport::Channel>;
async fn submit_start(client: &mut Client, args: &[String]) -> Result<(), Box<dyn Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(&args[2])?
        .take(12 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 12 * 1024 * 1024 {
        return Err(io::Error::other("start request exceeds 12 MiB").into());
    }
    let request: v1::StartTaskExecutionRequest = serde_json::from_slice(&bytes)?;
    let response = client.start_task_execution(request).await?.into_inner();
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
async fn stage_start(client: &mut Client, args: &[String]) -> Result<(), Box<dyn Error>> {
    let original = read_request(Path::new(&args[3]))?;
    let preparation = client
        .get_task_execution(v1::GetTaskExecutionRequest {
            workspace_id: original.workspace_id.clone(),
            project_id: original.project_id.clone(),
            conversation_id: original.conversation_id.clone(),
            task_id: original.task_id.clone(),
        })
        .await?
        .into_inner()
        .preparation
        .ok_or("task preparation missing")?;
    if preparation.attempt_id.is_empty() || preparation.inputs != original.inputs {
        return Err(io::Error::other("frozen task preparation differs").into());
    }
    let inputs = preparation.inputs.ok_or("prepared inputs missing")?;
    let (commit, lock) = git::verify(
        Path::new(&args[2]),
        &inputs.repository,
        &inputs.source_commit,
    )?;
    if commit != inputs.source_commit || lock != inputs.lock_digest {
        return Err(io::Error::other("frozen source differs").into());
    }
    git::verify_upstreams(
        Path::new(&args[2]),
        &inputs.source_commit,
        &inputs
            .upstream_selections
            .iter()
            .map(|s| s.commit_sha.clone())
            .collect::<Vec<_>>(),
    )?;
    let pack = git::snapshot_pack(Path::new(&args[2]), &inputs.source_commit)?;
    let request = v1::StartTaskExecutionRequest {
        workspace_id: original.workspace_id,
        project_id: original.project_id,
        conversation_id: original.conversation_id,
        task_id: original.task_id,
        idempotency_key: format!("start-{}", preparation.attempt_id),
        attempt_id: preparation.attempt_id,
        source_pack: pack,
    };
    let bytes = serde_json::to_vec(&request)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[4])?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    println!(
        "Frozen start request written. Submit this same file with submit-start; retain it for exact retries."
    );
    Ok(())
}
async fn submit_preparation(client: &mut Client, args: &[String]) -> Result<(), Box<dyn Error>> {
    let request = read_request(Path::new(&args[2]))?;
    let result = client.prepare_task_execution(request).await?.into_inner();
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
async fn verify_preparation(client: &mut Client, args: &[String]) -> Result<(), Box<dyn Error>> {
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
        if inputs.source_commit.is_empty() {
            &specification.source_ref
        } else {
            &inputs.source_commit
        },
    )?;
    git::verify_upstreams(
        Path::new(&args[2]),
        &commit,
        &inputs
            .upstream_selections
            .iter()
            .map(|s| s.commit_sha.clone())
            .collect::<Vec<_>>(),
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
