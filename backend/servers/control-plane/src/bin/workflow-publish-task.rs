//! Trusted local publication review; frozen JSON is replayed exactly on submit.
use ayni_orchestrator_api::v1::{
    self, project_conversation_service_client::ProjectConversationServiceClient,
};
use ayni_orchestrator_control_plane::publication_review::{Frozen, compare_preview, review};
use std::{
    error::Error,
    fs::{File, OpenOptions},
    io::{Read as _, Write as _},
    net::SocketAddr,
    process::ExitCode,
    time::Duration,
};
use tonic::transport::Endpoint;
type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("task publication failed: {error}");
            ExitCode::FAILURE
        }
    }
}
async fn run() -> Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        return Err("usage: workflow-publish-task prepare LOOPBACK_ADDR SELECTION_JSON FROZEN_JSON | submit LOOPBACK_ADDR FROZEN_JSON | cancel LOOPBACK_ADDR CANCEL_JSON".into());
    }
    let address: SocketAddr = args[1].parse()?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("trusted loopback required".into());
    }
    let channel = Endpoint::from_shared(format!("http://{address}"))?
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .connect()
        .await?;
    let mut client = ProjectConversationServiceClient::new(channel);
    match args[0].as_str() {
        "prepare" if args.len() == 4 => prepare(&mut client, &args[2], &args[3]).await,
        "submit" if args.len() == 3 => submit(&mut client, &args[2]).await,
        "cancel" if args.len() == 3 => {
            let request: v1::CancelTaskPublicationRequest = read(&args[2])?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &client.cancel_task_publication(request).await?.into_inner()
                )?
            );
            Ok(())
        }
        _ => Err("invalid publication command".into()),
    }
}
fn read<T: serde::de::DeserializeOwned>(path: &str) -> Result<T> {
    let mut bytes = Vec::new();
    File::open(path)?.take(131_073).read_to_end(&mut bytes)?;
    if bytes.len() > 131_072 {
        return Err("publication file exceeds128KiB".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
async fn prepare(
    client: &mut ProjectConversationServiceClient<tonic::transport::Channel>,
    selection: &str,
    output: &str,
) -> Result {
    let selected = read(selection)?;
    let key = uuid::Uuid::now_v7().to_string();
    let frozen = review(client, selected, &key).await?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&serde_json::to_vec_pretty(&frozen)?)?;
    file.sync_all()?;
    println!(
        "Frozen publication request written. Review title, body, repository, source and evidence, then submit this exact file; preserve it for retries."
    );
    Ok(())
}

async fn submit(
    client: &mut ProjectConversationServiceClient<tonic::transport::Channel>,
    path: &str,
) -> Result {
    let frozen: Frozen = read(path)?;
    let request = &frozen.request;
    let selected = v1::GetTaskAttemptRequest {
        workspace_id: request.workspace_id.clone(),
        project_id: request.project_id.clone(),
        conversation_id: request.conversation_id.clone(),
        task_id: request.task_id.clone(),
        attempt_id: request.attempt_id.clone(),
    };
    let canonical = review(client, selected, &request.idempotency_key).await?;
    compare_preview(&frozen, &canonical)?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &client
                .start_task_publication(frozen.request)
                .await?
                .into_inner()
        )?
    );
    Ok(())
}
