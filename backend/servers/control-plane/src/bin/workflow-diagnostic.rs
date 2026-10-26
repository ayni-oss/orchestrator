use ayni_orchestrator_api::v1::{
    ConversationSnapshot, CreateConversationRequest, CreateProjectRequest, CreateWorkspaceRequest,
    GetConversationSnapshotRequest, ListConversationHistoryRequest, PresentationState,
    SendMessageRequest, WatchConversationRequest,
    project_conversation_service_client::ProjectConversationServiceClient,
    project_service_client::ProjectServiceClient, watch_conversation_response::Frame,
};
use std::{env, error::Error, io, net::SocketAddr, process::ExitCode, time::Duration};
use tokio::time::timeout;
use tonic::transport::Endpoint;
use uuid::Uuid;

const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:50051";
const SCRIPTED_RESPONSE: &str = "The conversation supervisor completed this turn.";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("workflow diagnostic failed: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<String, Box<dyn Error>> {
    let endpoint = parse_endpoint(env::args().skip(1))?;
    let channel = Endpoint::from_shared(endpoint)?.connect().await?;
    let mut projects = ProjectServiceClient::new(channel.clone());
    let mut conversations = ProjectConversationServiceClient::new(channel);
    let nonce = Uuid::now_v7().to_string();

    let workspace_response = projects
        .create_workspace(CreateWorkspaceRequest {
            idempotency_key: format!("diagnostic-workspace-{nonce}"),
            display_name: format!("Diagnostic {nonce}"),
        })
        .await?
        .into_inner();
    let workspace = workspace_response
        .workspace
        .ok_or_else(|| invalid_response("CreateWorkspace omitted workspace"))?;
    require_receipt(workspace_response.receipt, "CreateWorkspace")?;

    let project_response = projects
        .create_project(CreateProjectRequest {
            idempotency_key: format!("diagnostic-project-{nonce}"),
            display_name: "Diagnostic project".to_owned(),
            workspace_id: workspace.id.clone(),
            goal: "Verify durable execution, watches and resumable reads".to_owned(),
        })
        .await?
        .into_inner();
    let project = project_response
        .project
        .ok_or_else(|| invalid_response("CreateProject omitted project"))?;
    require_receipt(project_response.receipt, "CreateProject")?;

    let conversation_response = conversations
        .create_conversation(CreateConversationRequest {
            idempotency_key: format!("diagnostic-conversation-{nonce}"),
            workspace_id: workspace.id.clone(),
            project_id: project.id.clone(),
            title: "Diagnostic conversation".to_owned(),
        })
        .await?
        .into_inner();
    let conversation = conversation_response
        .conversation
        .ok_or_else(|| invalid_response("CreateConversation omitted conversation"))?;
    require_receipt(conversation_response.receipt, "CreateConversation")?;

    let initial = initial_snapshot(
        &mut conversations,
        &workspace.id,
        &project.id,
        &conversation.id,
    )
    .await?;

    let message_response = conversations
        .send_message(SendMessageRequest {
            idempotency_key: format!("diagnostic-message-{nonce}"),
            workspace_id: workspace.id.clone(),
            project_id: project.id.clone(),
            conversation_id: conversation.id.clone(),
            content: "Verify PostgreSQL-authoritative workflow admission.".to_owned(),
        })
        .await?
        .into_inner();
    require_receipt(message_response.receipt, "SendMessage")?;

    let snapshot = wait_for_scripted_response(
        &mut conversations,
        &workspace.id,
        &project.id,
        &conversation.id,
        initial.resume_cursor,
        initial.conversation_revision,
    )
    .await?;

    let history = conversations
        .list_conversation_history(ListConversationHistoryRequest {
            workspace_id: workspace.id.clone(),
            project_id: project.id.clone(),
            conversation_id: conversation.id.clone(),
            snapshot_id: snapshot.history_snapshot_id,
            before_sequence: 0,
            limit: 10,
        })
        .await?
        .into_inner();
    if history.records.len() < 2
        || !history
            .records
            .iter()
            .any(|record| record.content_or_outcome == SCRIPTED_RESPONSE)
    {
        return Err(invalid_response("history did not contain the scripted response").into());
    }

    Ok(format!(
        "workflow Slice 3 diagnostic passed workspace_id={} project_id={} conversation_id={} revision={}",
        workspace.id, project.id, conversation.id, snapshot.conversation_revision
    ))
}

async fn initial_snapshot(
    conversations: &mut ProjectConversationServiceClient<tonic::transport::Channel>,
    workspace_id: &str,
    project_id: &str,
    conversation_id: &str,
) -> Result<ConversationSnapshot, Box<dyn Error>> {
    let mut updates = conversations
        .watch_conversation(WatchConversationRequest {
            workspace_id: workspace_id.to_owned(),
            project_id: project_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            after_cursor: None,
        })
        .await?
        .into_inner();
    let initial = timeout(Duration::from_secs(5), updates.message())
        .await??
        .and_then(|frame| match frame.frame {
            Some(Frame::Snapshot(snapshot)) => Some(snapshot),
            _ => None,
        })
        .ok_or_else(|| invalid_response("watch omitted its atomic initial snapshot"))?;
    if initial.resume_cursor.is_empty() {
        return Err(invalid_response("snapshot omitted resume cursor").into());
    }
    drop(updates);

    Ok(initial)
}

async fn wait_for_scripted_response(
    conversations: &mut ProjectConversationServiceClient<tonic::transport::Channel>,
    workspace_id: &str,
    project_id: &str,
    conversation_id: &str,
    mut cursor: String,
    mut revision: u64,
) -> Result<ConversationSnapshot, Box<dyn Error>> {
    // Reconnect after each accepted frame: notification delivery may disappear,
    // but the persisted cursor must replay every later revision exactly once.
    for _ in 0..16 {
        let mut updates = conversations
            .watch_conversation(WatchConversationRequest {
                workspace_id: workspace_id.to_owned(),
                project_id: project_id.to_owned(),
                conversation_id: conversation_id.to_owned(),
                after_cursor: Some(cursor.clone()),
            })
            .await?
            .into_inner();
        let completed = loop {
            let frame = timeout(Duration::from_secs(15), updates.message())
                .await??
                .ok_or_else(|| invalid_response("watch ended before terminal execution"))?;
            match frame.frame {
                Some(Frame::Update(update)) => {
                    if update.conversation_revision != revision + 1 || update.cursor == cursor {
                        return Err(invalid_response(
                            "watch replay skipped or duplicated a revision",
                        )
                        .into());
                    }
                    cursor = update.cursor;
                    revision = update.conversation_revision;
                    break update.presentation_state == i32::from(PresentationState::Completed);
                }
                Some(Frame::Heartbeat(_)) => {}
                _ => {
                    return Err(invalid_response(
                        "cursor resume emitted a snapshot or missing frame",
                    )
                    .into());
                }
            }
        };
        drop(updates);
        if !completed {
            continue;
        }
        let snapshot = conversations
            .get_conversation_snapshot(GetConversationSnapshotRequest {
                workspace_id: workspace_id.to_owned(),
                project_id: project_id.to_owned(),
                conversation_id: conversation_id.to_owned(),
            })
            .await?
            .into_inner()
            .snapshot
            .ok_or_else(|| invalid_response("GetConversationSnapshot omitted snapshot"))?;
        if snapshot.conversation_revision != revision
            || !snapshot.nonterminal_turns.is_empty()
            || snapshot.recent_terminal_turns.len() != 1
            || !snapshot
                .recent_messages
                .iter()
                .any(|message| message.content == SCRIPTED_RESPONSE)
        {
            return Err(
                invalid_response("terminal watch and authoritative snapshot disagree").into(),
            );
        }
        return Ok(snapshot);
    }
    Err(invalid_response("too many updates without a terminal response").into())
}

fn parse_endpoint(mut arguments: impl Iterator<Item = String>) -> Result<String, io::Error> {
    let endpoint = match (arguments.next().as_deref(), arguments.next()) {
        (None, None) => DEFAULT_ENDPOINT.to_owned(),
        (Some("--endpoint"), Some(endpoint)) if arguments.next().is_none() => endpoint,
        _ => {
            return Err(io::Error::other(
                "usage: workflow-diagnostic [--endpoint URL]",
            ));
        }
    };
    let address = endpoint
        .strip_prefix("http://")
        .ok_or_else(|| io::Error::other("diagnostic endpoint must use loopback HTTP"))?
        .parse::<SocketAddr>()
        .map_err(|_| io::Error::other("diagnostic endpoint must contain a loopback socket"))?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(io::Error::other(
            "diagnostic endpoint must contain a loopback socket",
        ));
    }
    Ok(endpoint)
}

fn require_receipt<T>(receipt: Option<T>, operation: &str) -> Result<(), io::Error> {
    receipt.map_or_else(
        || Err(invalid_response(&format!("{operation} omitted receipt"))),
        |_| Ok(()),
    )
}

fn invalid_response(message: &str) -> io::Error {
    io::Error::other(message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_is_loopback_only() {
        let accepted =
            parse_endpoint(["--endpoint".to_owned(), "http://[::1]:50051".to_owned()].into_iter());
        assert!(matches!(accepted.as_deref(), Ok("http://[::1]:50051")));
        assert!(
            parse_endpoint(
                ["--endpoint".to_owned(), "http://192.0.2.1:50051".to_owned()].into_iter()
            )
            .is_err()
        );
    }
}
