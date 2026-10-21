//! Deterministic M1 executor command claimant.
//!
//! The process retains exact command and signed claim bytes in `SQLite` before it
//! acknowledges a `JetStream` delivery. Execution starts only after the targeted
//! application acceptance is durable in the same retained ledger.

use async_nats::{ConnectOptions, HeaderMap, jetstream};
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{ConversationTurnReport, ConversationTurnReportKind},
    workflow::{
        REPORT_SUBJECT, decode_command, decode_decision, encode_report, sign_report,
        signing_key_digest,
    },
};
use ayni_orchestrator_scripted_executor::{
    Command, CommandDelivery, Decision, Ledger, LedgerLimits, Report, RetainedEvent,
};
use ed25519_dalek::SigningKey;
use futures_util::StreamExt as _;
use sha2::{Digest as _, Sha256};
use std::{env, error::Error, fs, io, path::PathBuf, time::Duration};
use tokio::time;
use uuid::Uuid;

const COMMAND_STREAM: &str = "ORCHESTRATOR_WORKFLOW_COMMAND_V1";
const COMMAND_CONSUMER: &str = "scripted-executor";
const DECISION_STREAM: &str = "ORCHESTRATOR_WORKFLOW_DECISION_V1";
const RETRY_DELAY: Duration = Duration::from_millis(250);
const SCRIPTED_RESPONSE: &str = "The scripted executor completed this turn.";

type RuntimeResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[tokio::main]
async fn main() -> RuntimeResult {
    let configuration = Configuration::from_environment()?;
    let mut ledger = Ledger::open(
        &configuration.ledger_path,
        LedgerLimits::m1_default(),
        &configuration.instance_id,
        &configuration.seed,
    )?;
    loop {
        let connected = connect(&configuration);
        let context = tokio::select! {
            result = connected => match result {
                Ok(context) => context,
                Err(error) => {
                    eprintln!("scripted executor broker connection pending: {error}");
                    if wait_or_shutdown().await? {
                        return Ok(());
                    }
                    continue;
                }
            },
            result = shutdown_signal() => {
                result?;
                return Ok(());
            }
        };
        if let Err(error) = run_connected(&context, &configuration, &mut ledger).await {
            eprintln!("scripted executor transport pending recovery: {error}");
            if wait_or_shutdown().await? {
                return Ok(());
            }
        } else {
            return Ok(());
        }
    }
}

async fn connect(configuration: &Configuration) -> RuntimeResult<jetstream::Context> {
    let client = ConnectOptions::with_user_and_password(
        configuration.username.clone(),
        configuration.password.clone(),
    )
    .connection_timeout(Duration::from_secs(3))
    .max_reconnects(None)
    .connect(&configuration.broker_url)
    .await?;
    let mut context = jetstream::new(client);
    context.set_timeout(Duration::from_secs(3));
    Ok(context)
}

async fn run_connected(
    context: &jetstream::Context,
    configuration: &Configuration,
    ledger: &mut Ledger,
) -> RuntimeResult {
    let command_consumer: jetstream::consumer::PullConsumer = context
        .get_consumer_from_stream(COMMAND_CONSUMER, COMMAND_STREAM)
        .await?;
    let decision_consumer_name = format!("executor-{}", configuration.instance_id);
    let mut decision_consumer: jetstream::consumer::PullConsumer = context
        .get_consumer_from_stream(&decision_consumer_name, DECISION_STREAM)
        .await?;
    let acknowledged_floor = decision_consumer.info().await?.ack_floor.consumer_sequence;
    for pending in ledger.pending_decision_acknowledgements()? {
        if pending.consumer_sequence <= acknowledged_floor {
            ledger.reconcile_decision_ack_floor(
                &pending.report_source,
                &pending.report_id,
                &decision_consumer_name,
                acknowledged_floor,
            )?;
        }
    }
    publish_pending_reports(context, ledger.pending_reports()?).await?;
    execute_ready(context, configuration, ledger).await?;
    let mut commands = command_consumer.messages().await?;
    let mut decisions = decision_consumer.messages().await?;
    loop {
        tokio::select! {
            command = commands.next() => {
                let command = command
                    .ok_or_else(|| io::Error::other("command consumer stopped"))??;
                handle_command(context, configuration, ledger, &command).await?;
            }
            decision = decisions.next() => {
                let decision = decision
                    .ok_or_else(|| io::Error::other("decision consumer stopped"))??;
                handle_decision(context, configuration, ledger, &decision).await?;
            }
            result = shutdown_signal() => {
                result?;
                return Ok(());
            }
        }
    }
}

async fn handle_command(
    context: &jetstream::Context,
    configuration: &Configuration,
    ledger: &mut Ledger,
    message: &jetstream::Message,
) -> RuntimeResult {
    let (identity, metadata, payload) = decode_command(&message.payload)?;
    let binding = payload
        .binding
        .as_ref()
        .ok_or_else(|| io::Error::other("command omitted binding"))?;
    let command = Command {
        source: &identity.source,
        id: &identity.id,
        exact_bytes: &message.payload,
        turn_id: &binding.turn_id,
        assignment_id: &binding.assignment_id,
        executor_generation: binding.executor_generation,
    };
    match ledger.classify_command_delivery(&command)? {
        CommandDelivery::New => {
            let claim = claim_bytes(
                configuration,
                &identity,
                &metadata,
                binding,
                &message.payload,
            )?;
            let (claim_identity, _, _) = ayni_orchestrator_events::workflow::decode_report(&claim)?;
            ledger.retain_command_and_claim(
                &command,
                &Report {
                    source: &claim_identity.source,
                    id: &claim_identity.id,
                    exact_bytes: &claim,
                    sequence: 1,
                },
            )?;
            publish_exact(context, &claim_identity.source, &claim_identity.id, &claim).await?;
        }
        CommandDelivery::PendingClaim(claim) => {
            publish_retained_report(context, &claim).await?;
        }
        CommandDelivery::Resolved => {}
    }
    message.double_ack().await?;
    Ok(())
}

fn claim_bytes(
    configuration: &Configuration,
    command_identity: &EventIdentity,
    metadata: &ayni_orchestrator_events::workflow::WorkflowEventMetadata,
    binding: &ayni_orchestrator_events::v1::ConversationTurnBinding,
    exact_command_bytes: &[u8],
) -> RuntimeResult<Vec<u8>> {
    let identity = EventIdentity {
        source: configuration.report_source(),
        id: Uuid::now_v7().to_string(),
    };
    let public_key = SigningKey::from_bytes(&configuration.seed)
        .verifying_key()
        .to_bytes();
    let mut report = ConversationTurnReport {
        binding: Some(ayni_orchestrator_events::v1::ConversationTurnBinding {
            command_event_id: command_identity.id.clone(),
            command_digest: digest(exact_command_bytes),
            ..binding.clone()
        }),
        kind: ConversationTurnReportKind::Started.into(),
        report_sequence: 1,
        executor_instance_id: configuration.instance_id.clone(),
        signing_public_key: public_key.to_vec(),
        signing_key_digest: signing_key_digest(&public_key),
        response_content: String::new(),
        failure_reason: String::new(),
        signature: Vec::new(),
    };
    sign_report(&identity, &mut report, &configuration.seed)?;
    Ok(encode_report(&identity, metadata, &report)?)
}

async fn handle_decision(
    context: &jetstream::Context,
    configuration: &Configuration,
    ledger: &mut Ledger,
    message: &jetstream::Message,
) -> RuntimeResult {
    let (_, _, payload) = decode_decision(&message.payload)?;
    let consumer_sequence = message.info()?.consumer_sequence;
    ledger.record_decision_delivery(
        &Decision {
            exact_bytes: &message.payload,
        },
        consumer_sequence,
    )?;
    message.double_ack().await?;
    ledger.acknowledge_decision(&payload.report_source, &payload.report_id)?;
    execute_ready(context, configuration, ledger).await
}

async fn execute_ready(
    context: &jetstream::Context,
    configuration: &Configuration,
    ledger: &mut Ledger,
) -> RuntimeResult {
    for command in ledger.executable_commands()? {
        let (identity, metadata, payload) = decode_command(&command.exact_bytes)?;
        let binding = payload
            .binding
            .ok_or_else(|| io::Error::other("retained command omitted binding"))?;
        let report_identity = EventIdentity {
            source: configuration.report_source(),
            id: Uuid::now_v7().to_string(),
        };
        let public_key = SigningKey::from_bytes(&configuration.seed)
            .verifying_key()
            .to_bytes();
        let mut report = ConversationTurnReport {
            binding: Some(ayni_orchestrator_events::v1::ConversationTurnBinding {
                command_event_id: identity.id.clone(),
                command_digest: digest(&command.exact_bytes),
                ..binding
            }),
            kind: ConversationTurnReportKind::Completed.into(),
            report_sequence: 2,
            executor_instance_id: configuration.instance_id.clone(),
            signing_public_key: Vec::new(),
            signing_key_digest: signing_key_digest(&public_key),
            response_content: SCRIPTED_RESPONSE.to_owned(),
            failure_reason: String::new(),
            signature: Vec::new(),
        };
        sign_report(&report_identity, &mut report, &configuration.seed)?;
        let exact_bytes = encode_report(&report_identity, &metadata, &report)?;
        ledger.retain_terminal_report(
            &command.source,
            &command.id,
            &Report {
                source: &report_identity.source,
                id: &report_identity.id,
                exact_bytes: &exact_bytes,
                sequence: 2,
            },
        )?;
        publish_exact(
            context,
            &report_identity.source,
            &report_identity.id,
            &exact_bytes,
        )
        .await?;
    }
    Ok(())
}

async fn publish_pending_reports(
    context: &jetstream::Context,
    reports: Vec<RetainedEvent>,
) -> RuntimeResult {
    for report in reports {
        publish_retained_report(context, &report).await?;
    }
    Ok(())
}

async fn publish_retained_report(
    context: &jetstream::Context,
    report: &RetainedEvent,
) -> RuntimeResult {
    publish_exact(context, &report.source, &report.id, &report.exact_bytes).await
}

async fn publish_exact(
    context: &jetstream::Context,
    source: &str,
    id: &str,
    bytes: &[u8],
) -> RuntimeResult {
    let mut headers = HeaderMap::new();
    headers.insert("Nats-Msg-Id", format!("{source}:{id}"));
    context
        .publish_with_headers(REPORT_SUBJECT.to_owned(), headers, bytes.to_vec().into())
        .await?
        .await?;
    Ok(())
}

async fn wait_or_shutdown() -> RuntimeResult<bool> {
    tokio::select! {
        () = time::sleep(RETRY_DELAY) => Ok(false),
        result = shutdown_signal() => {
            result?;
            Ok(true)
        }
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> io::Result<()> {
    let interrupt = tokio::signal::ctrl_c();
    let terminate = async {
        let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        signal.recv().await;
        Ok(())
    };
    tokio::select! {
        result = interrupt => result,
        result = terminate => result,
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> io::Result<()> {
    tokio::signal::ctrl_c().await
}

fn digest(bytes: &[u8]) -> String {
    let value = Sha256::digest(bytes);
    format!("sha256:{value:x}")
}

struct Configuration {
    broker_url: String,
    username: String,
    password: String,
    instance_id: String,
    seed: [u8; 32],
    ledger_path: PathBuf,
}

impl Configuration {
    fn from_environment() -> Result<Self, io::Error> {
        let required = |name: &str| {
            env::var(name).map_err(|_| io::Error::other(format!("{name} is required")))
        };
        let broker_url = required("ORCHESTRATOR_EXECUTOR_BROKER_URL")?;
        let broker_authority = broker_url
            .strip_prefix("nats://")
            .ok_or_else(|| io::Error::other("executor broker URL is invalid"))?;
        if broker_authority.is_empty()
            || broker_url.len() > 512
            || broker_authority
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b'@' | b'/' | b'?' | b'#'))
        {
            return Err(io::Error::other("executor broker URL is invalid"));
        }
        let username = required("ORCHESTRATOR_EXECUTOR_BROKER_USERNAME")?;
        validate_token(&username, "executor broker username")?;
        let instance_id = read_secret_path(
            &required("ORCHESTRATOR_EXECUTOR_INSTANCE_ID_PATH")?,
            "executor instance ID",
        )?;
        validate_token(&instance_id, "executor instance ID")?;
        Uuid::parse_str(&instance_id)
            .map_err(|_| io::Error::other("executor instance ID is invalid"))?;
        let password = read_secret_path(
            &required("ORCHESTRATOR_EXECUTOR_BROKER_PASSWORD_PATH")?,
            "executor broker password",
        )?;
        let seed_text = read_secret_path(
            &required("ORCHESTRATOR_EXECUTOR_SIGNING_SEED_PATH")?,
            "executor signing seed",
        )?;
        let seed = decode_seed(&seed_text)?;
        let ledger_path = PathBuf::from(required("ORCHESTRATOR_EXECUTOR_LEDGER_PATH")?);
        if !ledger_path.is_absolute() {
            return Err(io::Error::other("executor ledger path must be absolute"));
        }
        Ok(Self {
            broker_url,
            username,
            password,
            instance_id,
            seed,
            ledger_path,
        })
    }

    fn report_source(&self) -> String {
        format!("ayni://orchestrator/supervisors/{}", self.instance_id)
    }
}

fn read_secret_path(path: &str, name: &str) -> Result<String, io::Error> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(io::Error::other(format!("{name} path must be absolute")));
    }
    let value =
        fs::read_to_string(path).map_err(|_| io::Error::other(format!("{name} is unavailable")))?;
    let value = value.trim_end_matches(['\n', '\r']).to_owned();
    if value.is_empty() {
        return Err(io::Error::other(format!("{name} is unavailable")));
    }
    Ok(value)
}

fn validate_token(value: &str, name: &str) -> Result<(), io::Error> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        Err(io::Error::other(format!("{name} is invalid")))
    } else {
        Ok(())
    }
}

fn decode_seed(value: &str) -> Result<[u8; 32], io::Error> {
    if value.len() != 64 {
        return Err(io::Error::other("executor signing seed is invalid"));
    }
    let mut seed = [0_u8; 32];
    for (index, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| io::Error::other("executor signing seed is invalid"))?;
    }
    Ok(seed)
}
