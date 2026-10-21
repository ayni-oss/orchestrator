use crate::Result;
use async_nats::{
    HeaderMap,
    jetstream::{self, consumer::PullConsumer},
};
use std::{env, io, time::Duration};
use tokio_stream::StreamExt as _;

pub const COMMAND_STREAM: &str = "ORCHESTRATOR_WORKFLOW_COMMAND_V1";
pub const REPORT_STREAM: &str = "ORCHESTRATOR_WORKFLOW_REPORT_V1";
pub const DECISION_STREAM: &str = "ORCHESTRATOR_WORKFLOW_DECISION_V1";
pub const REPORT_SUBJECT: &str = "orchestrator.workflow.report.conversation.v1";
pub const WINNER: &str = "018f0f7b-6a4d-7a11-8d77-010203040506";
pub const LOSER: &str = "018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6";
pub const WINNER_CONSUMER: &str = "executor-018f0f7b-6a4d-7a11-8d77-010203040506";
pub const LOSER_CONSUMER: &str = "executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6";

pub struct Transport {
    pub application: jetstream::Context,
    pub administrator: jetstream::Context,
    pub winner: jetstream::Context,
    pub loser: jetstream::Context,
    pub commands: PullConsumer,
    pub reports: PullConsumer,
    pub winner_decisions: PullConsumer,
    pub loser_decisions: PullConsumer,
}

impl Transport {
    pub async fn connect() -> Result<Self> {
        let application = context("APPLICATION").await?;
        let administrator = context("ADMIN").await?;
        let winner = context("WINNER").await?;
        let loser = context("LOSER").await?;
        Ok(Self {
            commands: winner
                .get_consumer_from_stream("scripted-executor", COMMAND_STREAM)
                .await?,
            reports: application
                .get_consumer_from_stream("workflow-application", REPORT_STREAM)
                .await?,
            winner_decisions: winner
                .get_consumer_from_stream(WINNER_CONSUMER, DECISION_STREAM)
                .await?,
            loser_decisions: loser
                .get_consumer_from_stream(LOSER_CONSUMER, DECISION_STREAM)
                .await?,
            application,
            administrator,
            winner,
            loser,
        })
    }
}

async fn context(role: &str) -> Result<jetstream::Context> {
    for _ in 0..50 {
        let client = async_nats::ConnectOptions::with_user_and_password(
            env::var(format!("M1_{role}_NATS_USER"))?,
            env::var(format!("M1_{role}_NATS_PASSWORD"))?,
        )
        .connection_timeout(Duration::from_millis(500))
        .max_reconnects(1)
        .connect(env::var(format!("M1_{role}_NATS_URL"))?)
        .await;
        if let Ok(client) = client {
            let mut context = jetstream::new(client);
            context.set_timeout(Duration::from_secs(3));
            return Ok(context);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(io::Error::other("disposable broker did not reconnect").into())
}

pub async fn next(consumer: &PullConsumer) -> Result<jetstream::Message> {
    let mut messages = consumer
        .fetch()
        .max_messages(1)
        .expires(Duration::from_secs(3))
        .messages()
        .await?;
    messages
        .next()
        .await
        .ok_or_else(|| io::Error::other("expected retained handoff delivery"))?
}

pub async fn publish(
    context: &jetstream::Context,
    subject: &str,
    id: &str,
    bytes: &[u8],
) -> Result<jetstream::publish::PublishAck> {
    let mut headers = HeaderMap::new();
    headers.insert("Nats-Msg-Id", id);
    Ok(context
        .publish_with_headers(subject.to_owned(), headers, bytes.to_vec().into())
        .await?
        .await?)
}

pub async fn restart_fixture(variable: &str) -> Result {
    let container = env::var(variable)?;
    if !(container.starts_with("ayni-m1-")
        && container
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
    {
        return Err(
            io::Error::other("restart target is not a disposable acceptance fixture").into(),
        );
    }
    let output = tokio::process::Command::new("docker")
        .args(["restart", &container])
        .output()
        .await?;
    if !output.status.success() {
        return Err(io::Error::other("disposable dependency restart failed").into());
    }
    Ok(())
}
