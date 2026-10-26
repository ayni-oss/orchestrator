//! Deterministic M1 executor command claimant.
//!
//! The process retains exact command and signed claim bytes in `SQLite` before it
//! acknowledges a `JetStream` delivery. Execution starts only after the targeted
//! application acceptance is durable in the same retained ledger.

mod session_supervisor;
mod session_worker;

use async_nats::{ConnectOptions, HeaderMap, jetstream};
use ayni_orchestrator_conversation_supervisor::{
    Command, CommandDelivery, Decision, Ledger, LedgerLimits, Report, RetainedEvent,
};
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{ConversationTurnReport, ConversationTurnReportKind},
    workflow::{
        REPORT_SUBJECT, decode_command, decode_decision, encode_report, sign_report,
        signing_key_digest,
        transport::{ConsumerRole, rejected_delivery},
    },
};
use ayni_orchestrator_observability::{
    event_context,
    workflow::{Workflow, finish},
};
use ed25519_dalek::SigningKey;
use futures_util::StreamExt as _;
use sha2::{Digest as _, Sha256};
use std::{
    collections::VecDeque,
    env,
    error::Error,
    fs, io,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::time;
use tracing::Instrument as _;
use uuid::Uuid;

const COMMAND_STREAM: &str = "ORCHESTRATOR_WORKFLOW_COMMAND_V1";
// Retained broker cursor identity; independent of the supervisor process name.
const COMMAND_CONSUMER: &str = "scripted-executor";
const DECISION_STREAM: &str = "ORCHESTRATOR_WORKFLOW_DECISION_V1";
const RETRY_DELAY: Duration = Duration::from_millis(250);

type RuntimeResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[tokio::main]
async fn main() -> RuntimeResult {
    let telemetry = ayni_orchestrator_observability::init(
        if env::args().nth(1).as_deref() == Some("session-worker") {
            "conversation-worker"
        } else {
            "conversation-supervisor"
        },
    );
    let result = run().await;
    telemetry.shutdown();
    result
}

async fn run() -> RuntimeResult {
    if env::args().nth(1).as_deref() == Some("session-worker") {
        return session_worker::run().await.map_err(Into::into);
    }
    let configuration = Configuration::from_environment()?;
    let mut ledger = Ledger::open(
        &configuration.ledger_path,
        LedgerLimits::m1_default(),
        &configuration.instance_id,
        &configuration.seed,
    )?;
    // Ledger readiness is independent of broker availability. In a fresh local
    // install the broker streams are provisioned after workloads become ready.
    write_health(&configuration, &ledger)?;
    loop {
        let connected = connect(&configuration);
        let context = tokio::select! {
            result = connected => match result {
                Ok(context) => context,
                Err(error) => {
                    eprintln!("conversation supervisor broker connection pending: {error}");
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
        if let Err(error) = Box::pin(run_connected(&context, &configuration, &mut ledger)).await {
            eprintln!("conversation supervisor transport pending recovery: {error}");
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
    .request_timeout(Some(Duration::from_secs(3)))
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
            session_supervisor::mark_decided(configuration, &pending.report_id)?;
            ledger.reconcile_decision_ack_floor(
                &pending.report_source,
                &pending.report_id,
                &decision_consumer_name,
                acknowledged_floor,
            )?;
            let _ = session_supervisor::acknowledge(configuration, &pending.report_id).await;
        }
    }
    maintain_ledger(configuration, ledger)?;
    replay_quarantine(context, configuration, ledger).await?;
    publish_pending_reports(context, ledger.pending_reports()?).await?;
    execute_ready(context, configuration, ledger, None).await?;
    let mut commands = command_consumer.messages().await?;
    let mut decisions = decision_consumer.messages().await?;
    let mut maintenance = time::interval(Duration::from_secs(30));
    let mut commands_paused = false;
    let mut decisions_paused = false;
    loop {
        tokio::select! {
            _ = maintenance.tick() => {
                maintain_ledger(configuration, ledger)?;
                let _ = session_supervisor::cleanup(configuration).await;
                replay_quarantine(context, configuration, ledger).await?;
                publish_pending_reports(context, ledger.pending_reports()?).await?;
                execute_ready(context, configuration, ledger, None).await?;
                commands_paused=false;
                decisions_paused=false;
            }
            command = commands.next(), if !commands_paused => {
                let command = command
                    .ok_or_else(|| io::Error::other("command consumer stopped"))??;
                commands_paused=delivery_paused(handle_command(context, configuration, ledger, &command).await)?;
                write_health(configuration,ledger)?;
            }
            decision = decisions.next(), if !decisions_paused => {
                let decision = decision
                    .ok_or_else(|| io::Error::other("decision consumer stopped"))??;
                decisions_paused=delivery_paused(handle_decision(context, configuration, ledger, &decision).await)?;
                write_health(configuration,ledger)?;
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
    if quarantine_delivery(
        ConsumerRole::ExecutorCommand,
        configuration,
        ledger,
        message,
    )
    .await?
    {
        return Ok(());
    }
    let diagnostic = event_context(&message.payload);
    let span = Workflow::from_event(&message.payload).span_with_context(
        "supervisor.command.receive",
        diagnostic.as_ref(),
        message.info().is_ok_and(|info| info.delivered > 1),
    );
    let received = handle_command_bytes(context, configuration, ledger, &message.payload)
        .instrument(span.clone())
        .await;
    finish(&span, received.is_ok());
    received?;
    message.double_ack().await?;
    Ok(())
}

async fn handle_command_bytes(
    context: &jetstream::Context,
    configuration: &Configuration,
    ledger: &mut Ledger,
    bytes: &[u8],
) -> RuntimeResult {
    let received = Instant::now();
    let (identity, metadata, payload) = decode_command(bytes)?;
    let binding = payload
        .binding
        .as_ref()
        .ok_or_else(|| io::Error::other("command omitted binding"))?;
    let command = Command {
        source: &identity.source,
        id: &identity.id,
        exact_bytes: bytes,
        turn_id: &binding.turn_id,
        assignment_id: &binding.assignment_id,
        executor_generation: binding.executor_generation,
    };
    match ledger.classify_command_delivery(&command)? {
        CommandDelivery::New => {
            let claim = claim_bytes(configuration, &identity, &metadata, binding, bytes)?;
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
            record_handoff(0, received.elapsed());
            publish_exact(context, &claim_identity.source, &claim_identity.id, &claim).await?;
        }
        CommandDelivery::PendingClaim(claim) => {
            publish_retained_report(context, &claim).await?;
        }
        CommandDelivery::Resolved => {}
    }
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
        provider_call: None,
        plan_proposal: None,
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
    let received = Instant::now();
    if quarantine_delivery(
        ConsumerRole::ExecutorDecision,
        configuration,
        ledger,
        message,
    )
    .await?
    {
        return Ok(());
    }
    let (_, _, payload) = decode_decision(&message.payload)?;
    let consumer_sequence = message.info()?.consumer_sequence;
    let decision_result = ledger.record_decision_delivery(
        &Decision {
            exact_bytes: &message.payload,
        },
        consumer_sequence,
    );
    match decision_result {
        Ok(_) => {}
        Err(
            ayni_orchestrator_conversation_supervisor::LedgerError::Invalid
            | ayni_orchestrator_conversation_supervisor::LedgerError::NotFound
            | ayni_orchestrator_conversation_supervisor::LedgerError::EventConflict
            | ayni_orchestrator_conversation_supervisor::LedgerError::StaleGeneration,
        ) => {
            ledger.quarantine_inapplicable_decision(
                &message.payload,
                consumer_sequence,
                now_seconds()?,
            )?;
            message.double_ack().await?;
            return write_health(configuration, ledger);
        }
        Err(error) => return Err(error.into()),
    }
    message.double_ack().await?;
    session_supervisor::mark_decided(configuration, &payload.report_id)?;
    ledger.acknowledge_decision(&payload.report_source, &payload.report_id)?;
    if matches!(payload.turn_state.as_str(), "completed" | "failed") {
        let _ = session_supervisor::acknowledge(configuration, &payload.report_id).await;
    }
    execute_ready(context, configuration, ledger, Some(received)).await?;
    write_health(configuration, ledger)
}

async fn execute_ready(
    context: &jetstream::Context,
    configuration: &Configuration,
    ledger: &mut Ledger,
    decision_received: Option<Instant>,
) -> RuntimeResult {
    for command in ledger.executable_commands()? {
        let (identity, metadata, payload) = decode_command(&command.exact_bytes)?;
        let result = if payload.session.is_some() {
            let diagnostic = event_context(&command.exact_bytes);
            let span = Workflow::from_event(&command.exact_bytes).span_with_context(
                "supervisor.session.execute",
                diagnostic.as_ref(),
                false,
            );
            let executed = session_supervisor::execute(configuration, &command.exact_bytes)
                .instrument(span.clone())
                .await;
            finish(&span, executed.is_ok());
            executed?
        } else {
            session_worker::ExecutionResult {
                command_id: identity.id.clone(),
                command_digest: digest(&command.exact_bytes),
                response_content: session_worker::FIXTURE_TURN_RESPONSE.to_owned(),
                failure_reason: String::new(),
                provider_call: None,
                plan_proposal: None,
            }
        };
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
            kind: if result.failure_reason.is_empty() {
                ConversationTurnReportKind::Completed.into()
            } else {
                ConversationTurnReportKind::Failed.into()
            },
            report_sequence: 2,
            executor_instance_id: configuration.instance_id.clone(),
            signing_public_key: Vec::new(),
            signing_key_digest: signing_key_digest(&public_key),
            response_content: result.response_content,
            failure_reason: result.failure_reason,
            provider_call: result.provider_call,
            plan_proposal: result.plan_proposal,
            signature: Vec::new(),
        };
        sign_report(&report_identity, &mut report, &configuration.seed)?;
        let exact_bytes = encode_report(&report_identity, &metadata, &report)?;
        session_supervisor::bind_report(configuration, &identity.id, &report_identity.id)?;
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
        if let Some(received) = decision_received {
            record_handoff(1, received.elapsed());
        }
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

async fn quarantine_delivery(
    role: ConsumerRole,
    configuration: &Configuration,
    ledger: &mut Ledger,
    message: &jetstream::Message,
) -> RuntimeResult<bool> {
    if rejected_delivery(role, &message.payload).is_none() {
        return Ok(false);
    }
    let result = ledger.quarantine_transport(
        role,
        &message.payload,
        message.info()?.consumer_sequence,
        now_seconds()?,
    );
    write_health(configuration, ledger)?;
    result?;
    message.double_ack().await?;
    Ok(true)
}

async fn replay_quarantine(
    context: &jetstream::Context,
    configuration: &Configuration,
    ledger: &mut Ledger,
) -> RuntimeResult {
    for event in ledger.quarantine_replay_batch()? {
        if rejected_delivery(event.role, &event.exact_bytes).is_some() {
            continue;
        }
        match event.role {
            ConsumerRole::ExecutorCommand => {
                handle_command_bytes(context, configuration, ledger, &event.exact_bytes).await?;
            }
            ConsumerRole::ExecutorDecision => {
                let (_, _, payload) = decode_decision(&event.exact_bytes)?;
                ledger.record_decision_delivery(
                    &Decision {
                        exact_bytes: &event.exact_bytes,
                    },
                    event.consumer_sequence,
                )?;
                // This delivery was ACKed only after the original quarantine commit.
                session_supervisor::mark_decided(configuration, &payload.report_id)?;
                ledger.acknowledge_decision(&payload.report_source, &payload.report_id)?;
                if matches!(payload.turn_state.as_str(), "completed" | "failed") {
                    let _ =
                        session_supervisor::acknowledge(configuration, &payload.report_id).await;
                }
                execute_ready(context, configuration, ledger, None).await?;
            }
            ConsumerRole::ApplicationReport => {
                return Err(io::Error::other("invalid executor quarantine role").into());
            }
        }
        ledger.resolve_quarantine(&event)?;
    }
    Ok(())
}

fn now_seconds() -> RuntimeResult<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs())
}

fn maintain_ledger(configuration: &Configuration, ledger: &mut Ledger) -> RuntimeResult {
    let now = now_seconds()?;
    ledger.prune_resolved_metadata(now)?;
    ledger.prune_transport_quarantine(now)?;
    write_health(configuration, ledger)
}

fn write_health(configuration: &Configuration, ledger: &Ledger) -> RuntimeResult {
    let health = ledger.quarantine_health()?;
    let pressure = ledger.pressure()?;
    let timings = handoff_snapshot();
    let body = format!(
        "{{\"ready\":{},\"quarantine_records\":{},\"quarantine_bytes\":{},\"quarantine_unsupported\":{},\"quarantine_full\":{},\"ledger_records\":{},\"ledger_bytes\":{},\"ledger_full\":{},\"pending_reports\":{},\"unresolved_commands\":{},\"command_claim_samples\":{},\"command_claim_p95_seconds\":{},\"decision_terminal_samples\":{},\"decision_terminal_p95_seconds\":{}}}\n",
        health.unsupported == 0 && !health.full && !pressure.full,
        health.records,
        health.bytes,
        health.unsupported,
        health.full,
        pressure.records,
        pressure.bytes,
        pressure.full,
        pressure.pending_reports,
        pressure.unresolved_commands,
        timings[0].0,
        timings[0].1,
        timings[1].0,
        timings[1].1
    );
    let path = configuration.ledger_path.with_extension("health.json");
    let temporary = configuration.ledger_path.with_extension("health.tmp");
    fs::write(&temporary, body)?;
    fs::rename(temporary, path)?;
    Ok(())
}

#[derive(Default)]
struct HandoffTiming {
    seen: u64,
    samples: VecDeque<Duration>,
}
static HANDOFF_TIMINGS: OnceLock<Mutex<[HandoffTiming; 2]>> = OnceLock::new();

fn record_handoff(index: usize, elapsed: Duration) {
    let timings = HANDOFF_TIMINGS
        .get_or_init(|| Mutex::new([HandoffTiming::default(), HandoffTiming::default()]));
    if let Ok(mut timings) = timings.lock() {
        let Some(timing) = timings.get_mut(index) else {
            return;
        };
        timing.seen = timing.seen.saturating_add(1);
        // Discard ten warm-up operations; retain a bounded sliding distribution.
        if timing.seen > 10 {
            if timing.samples.len() == 1000 {
                timing.samples.pop_front();
            }
            timing.samples.push_back(elapsed);
        }
    }
}

fn handoff_snapshot() -> [(usize, f64); 2] {
    let Some(timings) = HANDOFF_TIMINGS.get() else {
        return [(0, 0.0); 2];
    };
    let Ok(timings) = timings.lock() else {
        return [(0, 0.0); 2];
    };
    std::array::from_fn(|index| {
        let mut samples = timings[index].samples.iter().copied().collect::<Vec<_>>();
        samples.sort_unstable();
        let position = (samples.len() * 95).div_ceil(100).saturating_sub(1);
        (
            samples.len(),
            samples.get(position).map_or(0.0, Duration::as_secs_f64),
        )
    })
}

fn delivery_paused(result: RuntimeResult) -> RuntimeResult<bool> {
    match result {
        Ok(()) => Ok(false),
        Err(error)
            if matches!(
                error.downcast_ref::<ayni_orchestrator_conversation_supervisor::LedgerError>(),
                Some(
                    ayni_orchestrator_conversation_supervisor::LedgerError::Capacity
                        | ayni_orchestrator_conversation_supervisor::LedgerError::QuarantineCapacity
                )
            ) =>
        {
            eprintln!("conversation supervisor consumer paused: retained store capacity exhausted");
            Ok(true)
        }
        Err(error) => Err(error),
    }
}
