//! Reproducible production-profile transport boundaries with fixture credentials.
use async_nats::{
    ConnectOptions, Event, HeaderMap,
    jetstream::{self, stream},
};
use ayni_orchestrator_events::{
    ContractError, EventIdentity,
    v1::{ConversationContextRecord, ConversationTurnBinding, ConversationTurnRequested},
    workflow::{
        APPLICATION_SOURCE, MAX_COMMAND_BYTES, WorkflowEventMetadata, decode_command,
        encode_command,
    },
};
use std::{
    env,
    error::Error,
    fs,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::time::{sleep, timeout};
use uuid::Uuid;

type Result<T = ()> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const COMMAND: &str = "ORCHESTRATOR_WORKFLOW_COMMAND_V1";
const REPORT: &str = "ORCHESTRATOR_WORKFLOW_REPORT_V1";
const DECISION: &str = "ORCHESTRATOR_WORKFLOW_DECISION_V1";
const COMMAND_SUBJECT: &str = "orchestrator.workflow.command.conversation.v1";
const REPORT_SUBJECT: &str = "orchestrator.workflow.report.conversation.v1";

async fn context(role: &str) -> Result<jetstream::Context> {
    let client = ConnectOptions::with_user_and_password(
        env::var(format!("M1_{role}_NATS_USER"))?,
        env::var(format!("M1_{role}_NATS_PASSWORD"))?,
    )
    .connection_timeout(Duration::from_secs(2))
    .request_timeout(Some(Duration::from_secs(2)))
    .max_reconnects(1)
    .connect(env::var(format!("M1_{role}_NATS_URL"))?)
    .await?;
    let mut context = jetstream::new(client);
    context.set_timeout(Duration::from_secs(2));
    Ok(context)
}

#[tokio::test]
#[ignore = "requires scripts/ci/check-workflow-jetstream.sh"]
async fn disposable_profile_setup() -> Result {
    let root = std::path::PathBuf::from(env::var("M1_TRANSPORT_FIXTURE_DIRECTORY")?);
    let administrator = context("ADMIN").await?;
    for name in [COMMAND, REPORT, DECISION] {
        // This ignored setup is invoked only by the disposable fixture wrapper.
        // Stream deletion also resets ACKfloors between independent scenarios.
        let _ = administrator.delete_stream(name).await;
    }
    for file in ["command.json", "report.json", "decision.json"] {
        let config: stream::Config = serde_json::from_slice(&fs::read(root.join(file))?)?;
        administrator.create_stream(config).await?;
    }
    for (stream_name, file) in [
        (COMMAND, "command-consumer.json"),
        (REPORT, "report-consumer.json"),
        (DECISION, "executor-decision-consumer.json"),
        (DECISION, "loser-consumer.json"),
    ] {
        let config: jetstream::consumer::pull::Config =
            serde_json::from_slice(&fs::read(root.join(file))?)?;
        administrator
            .get_stream(stream_name)
            .await?
            .create_consumer(config)
            .await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires scripts/ci/check-workflow-jetstream.sh"]
async fn profile_authorization_capacity_and_payload() -> Result {
    let administrator = context("ADMIN").await?;
    // Restore long retention before the payload and capacity checks. The earlier
    // transport recovery test intentionally compressed replay age to two seconds.
    for name in [COMMAND, REPORT, DECISION] {
        let mut stream = administrator.get_stream(name).await?;
        let mut config = stream.info().await?.config.clone();
        config.max_age = Duration::from_secs(7 * 24 * 60 * 60);
        config.duplicate_window = Duration::from_secs(120);
        administrator.update_stream(config).await?;
    }
    authorization().await?;
    maximum_payload(&administrator).await?;
    for (stream_name, subject, role) in [
        (COMMAND, COMMAND_SUBJECT, "APPLICATION"),
        (REPORT, REPORT_SUBJECT, "WINNER"),
        (
            DECISION,
            "orchestrator.workflow.accepted.conversation.v1.018f0f7b-6a4d-7a11-8d77-010203040506",
            "APPLICATION",
        ),
    ] {
        stream_capacity(&administrator, stream_name, subject, role).await?;
    }
    println!(
        "transport-profile: anonymous-denied=pass cross-role-denied=pass stream-admin-denied=pass all-stream-discard-new=pass maximum-body-header=pass producer-excess-rejected=pass"
    );
    Ok(())
}

async fn authorization() -> Result {
    let endpoint = env::var("M1_ADMIN_NATS_URL")?;
    let anonymous = ConnectOptions::new()
        .connection_timeout(Duration::from_secs(1))
        .max_reconnects(0)
        .connect(endpoint)
        .await;
    assert!(
        anonymous.is_err(),
        "anonymous connection unexpectedly authorized"
    );
    for (role, forbidden_subject) in [
        ("APPLICATION", REPORT_SUBJECT),
        ("WINNER", COMMAND_SUBJECT),
        ("LOSER", COMMAND_SUBJECT),
    ] {
        let transport = context(role).await?;
        assert!(
            transport
                .publish(forbidden_subject.to_owned(), "forbidden".into())
                .await?
                .await
                .is_err(),
            "cross-role publication unexpectedly authorized"
        );
        assert!(
            transport
                .request::<_, _, serde_json::Value>(
                    format!("STREAM.PURGE.{COMMAND}"),
                    &serde_json::json!({})
                )
                .await
                .is_err(),
            "workload purged stream"
        );
        assert!(
            transport
                .create_stream(stream::Config {
                    name: format!("FORBIDDEN_{role}"),
                    subjects: vec![format!("forbidden.{role}")],
                    ..Default::default()
                })
                .await
                .is_err(),
            "workload created stream"
        );
        assert!(
            transport.delete_stream(COMMAND).await.is_err(),
            "workload deleted stream"
        );
        // A denied subscription is reported asynchronously by the NATS server.
        let errors = Arc::new(AtomicU64::new(0));
        let callback_errors = errors.clone();
        let client = ConnectOptions::with_user_and_password(
            env::var(format!("M1_{role}_NATS_USER"))?,
            env::var(format!("M1_{role}_NATS_PASSWORD"))?,
        )
        .event_callback(move |event| {
            let errors = callback_errors.clone();
            async move {
                if matches!(event, Event::ServerError(_)) {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
        .connect(env::var(format!("M1_{role}_NATS_URL"))?)
        .await?;
        let _subscription = client.subscribe(forbidden_subject.to_owned()).await?;
        client.flush().await?;
        timeout(Duration::from_secs(2), async {
            while errors.load(Ordering::Relaxed) == 0 {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
    }
    Ok(())
}

async fn stream_capacity(
    administrator: &jetstream::Context,
    name: &str,
    subject: &str,
    role: &str,
) -> Result {
    let publisher = context(role).await?;
    let initial = publisher
        .publish(subject.to_owned(), "retained-at-capacity".into())
        .await?
        .await?;
    let mut stream = administrator.get_stream(name).await?;
    let before = stream.info().await?.clone();
    let retained = stream.get_raw_message(initial.sequence).await?;
    let mut full = before.config.clone();
    full.max_messages = i64::try_from(before.state.messages)?;
    administrator.update_stream(full.clone()).await?;
    assert!(
        publisher
            .publish(subject.to_owned(), "rejected-at-capacity".into())
            .await?
            .await
            .is_err()
    );
    let after = stream.info().await?.clone();
    assert_eq!(after.state.messages, before.state.messages);
    assert_eq!(after.state.first_sequence, before.state.first_sequence);
    assert_eq!(
        stream.get_raw_message(initial.sequence).await?.payload,
        retained.payload
    );
    administrator.update_stream(before.config).await?;
    publisher
        .publish(subject.to_owned(), "recovered-after-capacity".into())
        .await?
        .await?;
    Ok(())
}

fn valid_command() -> (
    EventIdentity,
    WorkflowEventMetadata,
    ConversationTurnRequested,
) {
    let id = Uuid::now_v7().to_string();
    (
        EventIdentity {
            source: APPLICATION_SOURCE.into(),
            id: id.clone(),
        },
        WorkflowEventMetadata {
            produced_at: "2026-10-20T08:00:00+00:00".into(),
            traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into(),
        },
        ConversationTurnRequested {
            binding: Some(ConversationTurnBinding {
                workspace_id: format!("workspace:{}", Uuid::now_v7()),
                project_id: format!("project:{}", Uuid::now_v7()),
                conversation_id: format!("conversation:{}", Uuid::now_v7()),
                turn_id: format!("turn:{}", Uuid::now_v7()),
                assignment_id: Uuid::now_v7().to_string(),
                executor_generation: 1,
                execution_binding_revision: "scripted-v1".into(),
                context_revision: 1,
                command_event_id: id,
                command_digest: String::new(),
            }),
            input_message_id: format!("message:{}", Uuid::now_v7()),
            input_content: "maximum body fixture".into(),
            context: Vec::new(),
            ..Default::default()
        },
    )
}

async fn maximum_payload(administrator: &jetstream::Context) -> Result {
    let (identity, metadata, command) = valid_command();
    let mut bytes = encode_command(&identity, &metadata, &command)?;
    bytes.resize(MAX_COMMAND_BYTES, b' ');
    decode_command(&bytes)?;
    // NATS/1.0 CRLF + X-Padding: value CRLF + final CRLF = 25 bytes.
    let mut headers = HeaderMap::new();
    headers.insert("X-Padding", "x".repeat(4096 - 25));
    let publisher = context("APPLICATION").await?;
    let ack = publisher
        .publish_with_headers(COMMAND_SUBJECT.to_owned(), headers, bytes.clone().into())
        .await?
        .await?;
    let mut stream = administrator.get_stream(COMMAND).await?;
    let stored = stream.get_raw_message(ack.sequence).await?;
    assert_eq!(stored.payload.as_ref(), bytes);
    assert_eq!(
        stored
            .headers
            .get("X-Padding")
            .ok_or("padding header absent")?
            .as_str()
            .len(),
        4096 - 25
    );
    assert!(stream.info().await?.state.bytes >= u64::try_from(MAX_COMMAND_BYTES + 4096)?);
    bytes.push(b' ');
    assert!(
        decode_command(&bytes).is_err(),
        "oversized producer boundary accepted"
    );
    // The producer encoder also rejects excess before broker publication.
    let mut oversized = command;
    oversized.context = (1..=17)
        .map(|sequence| ConversationContextRecord {
            sequence,
            message_id: format!("message:{}", Uuid::now_v7()),
            role: "user".into(),
            content: "x".repeat(16 * 1024),
        })
        .collect();
    assert!(matches!(
        encode_command(&identity, &metadata, &oversized),
        Err(ContractError::Oversized)
    ));
    Ok(())
}
