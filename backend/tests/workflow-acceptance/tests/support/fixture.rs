use crate::Result;
use ayni_orchestrator_api::v1::{
    self, project_conversation_service_client::ProjectConversationServiceClient,
    project_service_client::ProjectServiceClient,
};
use ayni_orchestrator_control_plane::serve_grpc;
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{
        ConversationTurnBinding, ConversationTurnReport, ConversationTurnReportKind,
        ConversationTurnRequested,
    },
    workflow::{
        WorkflowEventMetadata, decode_command, encode_report, sign_report, signing_key_digest,
    },
};
use ayni_orchestrator_scripted_executor::{Command, Ledger, LedgerLimits, Report};
use ayni_orchestrator_workflow_postgres::{
    WorkflowStore,
    model::{OutboxRecord, ReportDecisionReceipt},
};
use ed25519_dalek::SigningKey;
use sha2::{Digest as _, Sha256};
use std::{env, fmt::Write as _, io, net::SocketAddr, time::Duration};
use tempfile::TempDir;
use tokio::sync::watch;
use tonic::transport::Channel;
use uuid::Uuid;

pub const WINNER_SEED: [u8; 32] = [7; 32];
pub const LOSER_SEED: [u8; 32] = [9; 32];

pub struct Runtime {
    database_url: String,
    pub store: WorkflowStore,
    pub api: ProjectConversationServiceClient<Channel>,
    pub projects: ProjectServiceClient<Channel>,
    shutdown: watch::Sender<bool>,
    server: tokio::task::JoinHandle<std::result::Result<(), io::Error>>,
}

impl Runtime {
    pub async fn start() -> Result<Self> {
        ayni_orchestrator_workflow_postgres::migrate_for_runtime(
            &env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
            &env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
        )
        .await?;
        let url = env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
        Self::open(url).await
    }

    async fn open(database_url: String) -> Result<Self> {
        let reservation = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = reservation.local_addr()?;
        drop(reservation);
        let (shutdown, receiver) = watch::channel(false);
        let store = WorkflowStore::new(database_url.clone());
        let server = tokio::spawn(serve_grpc(address, store.clone(), receiver));
        let channel = connect(address).await?;
        Ok(Self {
            database_url,
            store,
            api: ProjectConversationServiceClient::new(channel.clone()),
            projects: ProjectServiceClient::new(channel),
            shutdown,
            server,
        })
    }

    pub async fn restart(&mut self) -> Result {
        self.shutdown.send(true)?;
        // No observation RPC is left open at application restart boundaries.
        tokio::time::timeout(Duration::from_secs(3), &mut self.server).await???;
        let replacement = Self::open(self.database_url.clone()).await?;
        *self = replacement;
        Ok(())
    }

    pub async fn wait_database(&mut self) -> Result {
        for _ in 0..100 {
            if ayni_orchestrator_workflow_postgres::validate_for_runtime(
                &self.database_url,
                &env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
            )
            .await
            .is_ok()
            {
                self.store = WorkflowStore::new(self.database_url.clone());
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(io::Error::other("retained fixture database did not recover").into())
    }

    pub async fn command_record(&self, turn: &str) -> Result<OutboxRecord> {
        let (client, driver) =
            ayni_orchestrator_workflow_postgres::connect(&self.database_url).await?;
        let event: String = client
            .query_one(
                "SELECT command_event_id FROM workflow_dispatch_intents WHERE turn_id=$1",
                &[&turn],
            )
            .await?
            .get(0);
        driver.abort();
        self.outbox(&event).await
    }

    pub async fn decision_record(&self, receipt: &ReportDecisionReceipt) -> Result<OutboxRecord> {
        self.outbox(&receipt.decision_event_id).await
    }

    async fn outbox(&self, id: &str) -> Result<OutboxRecord> {
        let (client, driver) =
            ayni_orchestrator_workflow_postgres::connect(&self.database_url).await?;
        let row = client.query_one("SELECT id,conversation_id,turn_id,subject,event_bytes,attempt_count FROM workflow_outbox WHERE id=$1", &[&id]).await?;
        driver.abort();
        Ok(OutboxRecord {
            publication_attempts: u64::try_from(row.get::<_, i64>(5))?,
            id: row.get(0),
            conversation_id: row.get(1),
            turn_id: row.get(2),
            subject: row.get(3),
            event_bytes: row.get(4),
        })
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn connect(address: SocketAddr) -> Result<Channel> {
    let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}"))?;
    for _ in 0..50 {
        if let Ok(channel) = endpoint.connect().await {
            return Ok(channel);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(io::Error::other("canonical workflow listener unavailable").into())
}

pub struct Scope {
    pub workspace: String,
    pub project: String,
    pub conversation: String,
}
impl Scope {
    pub async fn create(runtime: &mut Runtime) -> Result<Self> {
        let workspace = runtime
            .projects
            .create_workspace(v1::CreateWorkspaceRequest {
                idempotency_key: Uuid::now_v7().to_string(),
                display_name: "Durability acceptance".into(),
            })
            .await?
            .into_inner()
            .workspace
            .ok_or("missing workspace")?;
        let project = runtime
            .projects
            .create_project(v1::CreateProjectRequest {
                idempotency_key: Uuid::now_v7().to_string(),
                workspace_id: workspace.id.clone(),
                display_name: "Durability acceptance".into(),
                goal: String::new(),
            })
            .await?
            .into_inner()
            .project
            .ok_or("missing project")?;
        let conversation = runtime
            .api
            .create_conversation(v1::CreateConversationRequest {
                idempotency_key: Uuid::now_v7().to_string(),
                workspace_id: workspace.id.clone(),
                project_id: project.id.clone(),
                title: "Durable handoff".into(),
            })
            .await?
            .into_inner()
            .conversation
            .ok_or("missing conversation")?;
        Ok(Self {
            workspace: workspace.id,
            project: project.id,
            conversation: conversation.id,
        })
    }

    pub async fn send(
        &self,
        runtime: &mut Runtime,
        key: &str,
        content: &str,
    ) -> Result<v1::CommandReceipt> {
        runtime
            .api
            .send_message(v1::SendMessageRequest {
                idempotency_key: key.into(),
                workspace_id: self.workspace.clone(),
                project_id: self.project.clone(),
                conversation_id: self.conversation.clone(),
                content: content.into(),
            })
            .await?
            .into_inner()
            .receipt
            .ok_or_else(|| "missing durable receipt".into())
    }

    pub async fn snapshot(&self, runtime: &mut Runtime) -> Result<v1::ConversationSnapshot> {
        runtime
            .api
            .get_conversation_snapshot(v1::GetConversationSnapshotRequest {
                workspace_id: self.workspace.clone(),
                project_id: self.project.clone(),
                conversation_id: self.conversation.clone(),
            })
            .await?
            .into_inner()
            .snapshot
            .ok_or_else(|| "missing authoritative snapshot".into())
    }
}

pub struct CommandFixture {
    pub identity: EventIdentity,
    pub binding: ConversationTurnBinding,
    pub payload: ConversationTurnRequested,
    pub metadata: WorkflowEventMetadata,
    pub record: OutboxRecord,
}
impl CommandFixture {
    pub fn decode(record: OutboxRecord) -> Result<Self> {
        let (identity, metadata, payload) = decode_command(&record.event_bytes)?;
        let binding = payload.binding.clone().ok_or("command binding absent")?;
        Ok(Self {
            identity,
            binding,
            payload,
            metadata,
            record,
        })
    }
    pub fn command(&self) -> Command<'_> {
        Command {
            source: &self.identity.source,
            id: &self.identity.id,
            exact_bytes: &self.record.event_bytes,
            turn_id: &self.binding.turn_id,
            assignment_id: &self.binding.assignment_id,
            executor_generation: self.binding.executor_generation,
        }
    }
}

pub struct SignedReport {
    pub identity: EventIdentity,
    pub bytes: Vec<u8>,
    pub sequence: u8,
}
impl SignedReport {
    pub fn new(
        command: &CommandFixture,
        executor: &str,
        seed: &[u8; 32],
        kind: ConversationTurnReportKind,
        response: &str,
    ) -> Result<Self> {
        let identity = EventIdentity {
            source: format!("ayni://orchestrator/supervisors/{executor}"),
            id: Uuid::now_v7().to_string(),
        };
        let key = SigningKey::from_bytes(seed).verifying_key().to_bytes();
        let mut binding = command.binding.clone();
        binding.command_digest = digest(&command.record.event_bytes);
        let sequence = if kind == ConversationTurnReportKind::Started {
            1
        } else {
            2
        };
        let mut report = ConversationTurnReport {
            binding: Some(binding),
            kind: kind.into(),
            report_sequence: u64::from(sequence),
            executor_instance_id: executor.into(),
            signing_public_key: if sequence == 1 { key.to_vec() } else { vec![] },
            signing_key_digest: signing_key_digest(&key),
            response_content: response.into(),
            failure_reason: if kind == ConversationTurnReportKind::Failed {
                "script_failed".into()
            } else {
                String::new()
            },
            ..Default::default()
        };
        sign_report(&identity, &mut report, seed)?;
        let bytes = encode_report(&identity, &command.metadata, &report)?;
        Ok(Self {
            identity,
            bytes,
            sequence,
        })
    }
    pub fn report(&self) -> Report<'_> {
        Report {
            source: &self.identity.source,
            id: &self.identity.id,
            exact_bytes: &self.bytes,
            sequence: self.sequence,
        }
    }
}

pub fn ledger(directory: &TempDir, instance: &str, seed: &[u8; 32]) -> Result<Ledger> {
    Ok(Ledger::open(
        directory.path().join("ledger.sqlite"),
        LedgerLimits::m1_default(),
        instance,
        seed,
    )?)
}

fn digest(bytes: &[u8]) -> String {
    let mut output = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
