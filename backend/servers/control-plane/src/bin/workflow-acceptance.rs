//! Canonical gRPC acceptance: correctness and warmed client-visible latency.
use ayni_orchestrator_api::v1::{
    CommandReceipt, ConversationSnapshot, ConversationUpdate, ConversationUpdateKind,
    CreateConversationRequest, CreateProjectRequest, CreateWorkspaceRequest,
    GetConversationSnapshotRequest, SendMessageRequest, WatchConversationRequest,
    WatchConversationResponse,
    project_conversation_service_client::ProjectConversationServiceClient,
    project_service_client::ProjectServiceClient, watch_conversation_response::Frame,
};
use futures_util::future::join_all;
use prost::Message as _;
use serde::{Deserialize, Serialize};
use std::{error::Error, io, net::SocketAddr, process::ExitCode, time::Duration};
use tokio::time::{Instant, timeout_at};
use tonic::{Code, Streaming, transport::Channel};
use uuid::Uuid;

type Result<T = ()> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
type Client = ProjectConversationServiceClient<Channel>;
const CONTENT: &str = "Exercise the deterministic durable workflow acceptance path.";
const SAMPLES: usize = 100;
const WARMUP: usize = 10;

#[derive(Serialize, Deserialize)]
struct Scope {
    workspace: String,
    project: String,
    conversation: String,
}

impl Scope {
    fn send(&self, key: &str) -> SendMessageRequest {
        SendMessageRequest {
            workspace_id: self.workspace.clone(),
            project_id: self.project.clone(),
            conversation_id: self.conversation.clone(),
            idempotency_key: key.to_owned(),
            content: CONTENT.to_owned(),
        }
    }
    fn watch(&self, cursor: Option<String>) -> WatchConversationRequest {
        WatchConversationRequest {
            workspace_id: self.workspace.clone(),
            project_id: self.project.clone(),
            conversation_id: self.conversation.clone(),
            after_cursor: cursor,
        }
    }
    fn snapshot(&self) -> GetConversationSnapshotRequest {
        GetConversationSnapshotRequest {
            workspace_id: self.workspace.clone(),
            project_id: self.project.clone(),
            conversation_id: self.conversation.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Sample {
    scope: Scope,
    idempotency_key: String,
    original_receipt: Vec<u8>,
    command_id: String,
    turn_id: String,
    final_revision: u64,
    cursors: Vec<String>,
    update_ids: Vec<String>,
    revisions: Vec<u64>,
    rpc_to_receipt_us: u64,
    rpc_to_activity_us: u64,
    rpc_to_terminal_us: u64,
}

#[derive(Serialize, Deserialize)]
struct Evidence {
    schema: String,
    completed_gates: Vec<String>,
    unmeasured_gates: Vec<String>,
    failed_gates: Vec<String>,
    warmup_samples: usize,
    measured_samples: usize,
    p95_receipt_us: u64,
    p95_activity_us: u64,
    p95_terminal_us: u64,
    samples: Vec<Sample>,
}

#[derive(Serialize, Deserialize)]
struct PendingEvidence {
    scope: Scope,
    requests: Vec<PendingRequest>,
}

#[derive(Serialize, Deserialize)]
struct PendingRequest {
    key: String,
    receipt: Vec<u8>,
    turn_id: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("workflow acceptance failed: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (endpoint, mode) = match args.as_slice() {
        [] => ("http://127.0.0.1:50051".to_owned(), "measure"),
        [flag, endpoint] if flag == "--endpoint" => (endpoint.clone(), "measure"),
        [flag, endpoint, mode] if flag == "--endpoint" => (endpoint.clone(), mode.as_str()),
        _ => {
            return Err(io::Error::other(
                "usage: workflow-acceptance [--endpoint LOOPBACK_URL [--verify-stdin]]",
            )
            .into());
        }
    };
    let address: SocketAddr = endpoint
        .strip_prefix("http://")
        .ok_or("loopback HTTP required")?
        .parse()?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("loopback endpoint required".into());
    }
    let channel = Channel::from_shared(endpoint)?.connect().await?;
    let mut client = Client::new(channel.clone());
    let mut projects = ProjectServiceClient::new(channel.clone());
    match mode {
        "--queue-capacity" => return queue_capacity(&mut projects, &mut client).await,
        "--prepare-pending" => return prepare_pending(&mut projects, &mut client).await,
        "--verify-pending-stdin" => return verify_pending(&mut client).await,
        "--expect-unavailable-stdin" => return expect_unavailable(&mut client).await,
        "--verify-stdin" => return verify_previous(&mut client).await,
        "measure" => {}
        _ => return Err("unknown acceptance mode".into()),
    }
    measure_all(&mut projects, &mut client).await
}

async fn prepare_pending(
    projects: &mut ProjectServiceClient<Channel>,
    client: &mut Client,
) -> Result {
    let scope = create_scope(projects, client).await?;
    let mut requests = Vec::new();
    for _ in 0..3 {
        let key = format!("pending-{}", Uuid::now_v7());
        let receipt = client
            .send_message(scope.send(&key))
            .await?
            .into_inner()
            .receipt
            .ok_or("pending receipt missing")?;
        requests.push(PendingRequest {
            key,
            receipt: receipt.encode_to_vec(),
            turn_id: receipt.turn_id,
        });
    }
    let state = snapshot(client, &scope).await?;
    if state.nonterminal_turns.len() != 3
        || state
            .nonterminal_turns
            .iter()
            .filter(|turn| turn.scheduling_state == 2)
            .count()
            != 1
    {
        return Err("broker outage did not preserve one eligible FIFO head".into());
    }
    println!(
        "{}",
        serde_json::to_string(&PendingEvidence { scope, requests })?
    );
    Ok(())
}

async fn verify_pending(client: &mut Client) -> Result {
    let pending: PendingEvidence = serde_json::from_reader(io::stdin().lock())?;
    let mut watch = client
        .watch_conversation(pending.scope.watch(None))
        .await?
        .into_inner();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = snapshot(client, &pending.scope).await?;
        if state.nonterminal_turns.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            return Err("pending turns failed to recover".into());
        }
        next_frame(&mut watch).await?;
    }
    for request in &pending.requests {
        let receipt = client
            .send_message(pending.scope.send(&request.key))
            .await?
            .into_inner()
            .receipt
            .ok_or("pending replay omitted receipt")?;
        if receipt.encode_to_vec() != request.receipt {
            return Err("pending restart changed original receipt".into());
        }
    }
    let state = snapshot(client, &pending.scope).await?;
    if state
        .history_counts
        .as_ref()
        .is_none_or(|count| count.messages != 6 || count.turns != 3)
        || state.recent_terminal_turns.len() != 3
    {
        return Err("pending recovery duplicated or lost accepted work".into());
    }
    println!("{{\"pending_recovery\":\"passed\",\"accepted_turns\":3}}");
    Ok(())
}

async fn expect_unavailable(client: &mut Client) -> Result {
    let report: Evidence = serde_json::from_reader(io::stdin().lock())?;
    let sample = report.samples.first().ok_or("missing probe sample")?;
    let admission = client
        .send_message(sample.scope.send("database-unavailable-probe"))
        .await
        .err()
        .ok_or("admission succeeded while database was unavailable")?;
    if admission.code() != Code::Unavailable {
        return Err("unavailable database admission had wrong status".into());
    }
    let observation = client
        .get_conversation_snapshot(sample.scope.snapshot())
        .await
        .err()
        .ok_or("observation succeeded while database was unavailable")?;
    if observation.code() != Code::Unavailable {
        return Err("unavailable database observation had wrong status".into());
    }
    println!("{{\"database_unavailable\":\"passed\",\"false_receipts\":0}}");
    Ok(())
}

async fn verify_previous(client: &mut Client) -> Result {
    let report: Evidence = serde_json::from_reader(io::stdin().lock())?;
    for sample in report.samples {
        verify_sample(client, &sample).await?;
    }
    println!(
        "{{\"restart_replay\":\"passed\",\"samples\":{}}}",
        report.measured_samples
    );
    Ok(())
}

async fn measure_all(projects: &mut ProjectServiceClient<Channel>, client: &mut Client) -> Result {
    let mut samples = Vec::with_capacity(SAMPLES);
    for index in 0..SAMPLES + WARMUP {
        let scope = create_scope(projects, client).await?;
        let sample = measure(client, scope).await?;
        verify_sample(client, &sample).await?;
        if index >= WARMUP {
            samples.push(sample);
        }
    }
    let receipt = p95(samples.iter().map(|s| s.rpc_to_receipt_us).collect())?;
    let activity = p95(samples.iter().map(|s| s.rpc_to_activity_us).collect())?;
    let terminal = p95(samples.iter().map(|s| s.rpc_to_terminal_us).collect())?;
    let mut completed_gates = vec![
        "canonical_grpc_snapshot_watch_boundary".into(),
        "watch_replay_from_every_cursor".into(),
        "exact_receipt_replay_after_terminal".into(),
        "concurrent_idempotent_replay".into(),
        "changed_payload_conflict_without_mutation".into(),
    ];
    let mut failed_gates = Vec::new();
    for (gate, measured, maximum) in [
        ("client_receipt_p95_250ms", receipt, 250_000),
        ("client_activity_p95_500ms", activity, 500_000),
        ("client_terminal_p95_2s", terminal, 2_000_000),
    ] {
        if measured > maximum {
            failed_gates.push(gate.to_owned());
        } else {
            completed_gates.push(gate.to_owned());
        }
    }
    let report = Evidence {
        schema: "build.ayni.workflow.client-acceptance.v1".into(),
        completed_gates,
        failed_gates,
        unmeasured_gates: vec![
            "owned_boundary_timings_require_server_metrics".into(),
            "fault_and_capacity_matrix_requires_cumulative_gate_results".into(),
        ],
        warmup_samples: WARMUP,
        measured_samples: samples.len(),
        p95_receipt_us: receipt,
        p95_activity_us: activity,
        p95_terminal_us: terminal,
        samples,
    };
    // Always emit measured evidence, including when a latency target fails.
    println!("{}", serde_json::to_string(&report)?);
    if receipt > 250_000 || activity > 500_000 || terminal > 2_000_000 {
        return Err(format!("client p95 thresholds exceeded: receipt={receipt}us activity={activity}us terminal={terminal}us").into());
    }
    Ok(())
}

async fn queue_capacity(
    projects: &mut ProjectServiceClient<Channel>,
    client: &mut Client,
) -> Result {
    let scope = create_scope(projects, client).await?;
    // Seed below the bound, then race the remaining slots. Opening105database
    // sessions at once would measure the connection cap instead of queue authority.
    let mut accepted = Vec::new();
    for _ in 0..95 {
        let key = format!("capacity-{}", Uuid::now_v7());
        let receipt = client
            .send_message(scope.send(&key))
            .await?
            .into_inner()
            .receipt
            .ok_or("capacity seed receipt missing")?;
        accepted.push(PendingRequest {
            key,
            receipt: receipt.encode_to_vec(),
            turn_id: receipt.turn_id,
        });
    }
    let requests = (0..10).map(|_| {
        let mut client = client.clone();
        let key = format!("capacity-{}", Uuid::now_v7());
        let request = scope.send(&key);
        async move { (key, client.send_message(request).await) }
    });
    let mut rejected = 0;
    for (key, response) in join_all(requests).await {
        match response {
            Ok(response) => {
                let receipt = response
                    .into_inner()
                    .receipt
                    .ok_or("capacity receipt missing")?;
                accepted.push(PendingRequest {
                    key,
                    receipt: receipt.encode_to_vec(),
                    turn_id: receipt.turn_id,
                });
            }
            Err(status) if status.code() == Code::ResourceExhausted => rejected += 1,
            Err(status) => return Err(status.into()),
        }
    }
    let state = snapshot(client, &scope).await?;
    let sequences: std::collections::BTreeSet<_> = state
        .nonterminal_turns
        .iter()
        .map(|turn| turn.sequence)
        .collect();
    if accepted.len() != 100
        || rejected != 5
        || state.nonterminal_turns.len() != 100
        || sequences != (1..=100).collect()
        || state
            .nonterminal_turns
            .iter()
            .filter(|turn| turn.scheduling_state == 2)
            .count()
            != 1
        || state
            .history_counts
            .as_ref()
            .is_none_or(|count| count.messages != 100 || count.turns != 100)
    {
        return Err("concurrent queue capacity/FIFO accounting failed".into());
    }
    println!(
        "{}",
        serde_json::to_string(&PendingEvidence {
            scope,
            requests: accepted
        })?
    );
    Ok(())
}

async fn create_scope(
    projects: &mut ProjectServiceClient<Channel>,
    client: &mut Client,
) -> Result<Scope> {
    let nonce = Uuid::now_v7().to_string();
    let workspace = projects
        .create_workspace(CreateWorkspaceRequest {
            idempotency_key: format!("acceptance-workspace-{nonce}"),
            display_name: "Acceptance workspace".into(),
        })
        .await?
        .into_inner()
        .workspace
        .ok_or("workspace missing")?;
    let project = projects
        .create_project(CreateProjectRequest {
            idempotency_key: format!("acceptance-project-{nonce}"),
            display_name: "Acceptance project".into(),
            workspace_id: workspace.id.clone(),
            goal: "Exercise canonical durable execution".into(),
        })
        .await?
        .into_inner()
        .project
        .ok_or("project missing")?;
    let conversation = client
        .create_conversation(CreateConversationRequest {
            idempotency_key: format!("acceptance-conversation-{nonce}"),
            workspace_id: workspace.id.clone(),
            project_id: project.id.clone(),
            title: "Acceptance conversation".into(),
        })
        .await?
        .into_inner()
        .conversation
        .ok_or("conversation missing")?;
    Ok(Scope {
        workspace: workspace.id,
        project: project.id,
        conversation: conversation.id,
    })
}

async fn measure(client: &mut Client, scope: Scope) -> Result<Sample> {
    let mut watch = client
        .watch_conversation(scope.watch(None))
        .await?
        .into_inner();
    let Frame::Snapshot(snapshot) = next_frame(&mut watch).await? else {
        return Err("cursorless watch must start with snapshot".into());
    };
    let mut revision = snapshot.conversation_revision;
    let key = format!("acceptance-message-{}", Uuid::now_v7());
    let started = Instant::now();
    let receipt = client
        .send_message(scope.send(&key))
        .await?
        .into_inner()
        .receipt
        .ok_or("receipt missing")?;
    let receipt_us = micros(started.elapsed());
    let mut cursors = vec![snapshot.resume_cursor];
    let mut update_ids = Vec::new();
    let mut revisions = Vec::new();
    let mut activity_us = None;
    let terminal_us;
    loop {
        let frame = next_frame(&mut watch).await?;
        let Frame::Update(update) = frame else {
            return Err("resume unexpectedly emitted snapshot".into());
        };
        validate_update(&update, revision, &cursors, &update_ids)?;
        revision = update.conversation_revision;
        cursors.push(update.cursor.clone());
        update_ids.push(update.update_id);
        revisions.push(revision);
        activity_us.get_or_insert_with(|| micros(started.elapsed()));
        let terminal = update.kind == i32::from(ConversationUpdateKind::TerminalOutcomeAccepted);
        if terminal {
            terminal_us = micros(started.elapsed());
            break;
        }
    }
    drop(watch);
    Ok(Sample {
        scope,
        idempotency_key: key,
        original_receipt: receipt.encode_to_vec(),
        command_id: receipt.command_id,
        turn_id: receipt.turn_id,
        final_revision: revision,
        cursors,
        update_ids,
        revisions,
        rpc_to_receipt_us: receipt_us,
        rpc_to_activity_us: activity_us.ok_or("no activity")?,
        rpc_to_terminal_us: terminal_us,
    })
}

fn validate_update(
    update: &ConversationUpdate,
    revision: u64,
    cursors: &[String],
    ids: &[String],
) -> Result {
    if update.conversation_revision != revision + 1
        || update.cursor.is_empty()
        || cursors.contains(&update.cursor)
        || update.update_id.is_empty()
        || ids.contains(&update.update_id)
    {
        return Err("watch gap, duplicate, or missing identity".into());
    }
    Ok(())
}

async fn next_frame(watch: &mut Streaming<WatchConversationResponse>) -> Result<Frame> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let response = timeout_at(deadline, watch.message())
            .await??
            .ok_or("watch ended")?;
        match response.frame.ok_or("watch omitted frame")? {
            Frame::Heartbeat(_) => {}
            frame => return Ok(frame),
        }
    }
}

async fn replay_every_cursor(client: &mut Client, sample: &Sample) -> Result {
    for (boundary, cursor) in sample
        .cursors
        .iter()
        .enumerate()
        .take(sample.update_ids.len())
    {
        let mut replay = client
            .watch_conversation(sample.scope.watch(Some(cursor.clone())))
            .await?
            .into_inner();
        for index in boundary..sample.update_ids.len() {
            let Frame::Update(update) = next_frame(&mut replay).await? else {
                return Err("cursor replay emitted snapshot".into());
            };
            if update.update_id != sample.update_ids[index]
                || update.cursor != sample.cursors[index + 1]
                || update.conversation_revision != sample.revisions[index]
            {
                return Err("cursor replay lost or duplicated an accepted update".into());
            }
        }
    }
    Ok(())
}

async fn verify_sample(client: &mut Client, sample: &Sample) -> Result {
    replay_every_cursor(client, sample).await?;
    let replay: CommandReceipt = client
        .send_message(sample.scope.send(&sample.idempotency_key))
        .await?
        .into_inner()
        .receipt
        .ok_or("replay receipt missing")?;
    if replay.encode_to_vec() != sample.original_receipt {
        return Err("replay changed original receipt".into());
    }
    let replays = join_all((0..8).map(|_| {
        let mut concurrent = client.clone();
        let request = sample.scope.send(&sample.idempotency_key);
        async move { concurrent.send_message(request).await }
    }))
    .await;
    for response in replays {
        if response?
            .into_inner()
            .receipt
            .ok_or("concurrent receipt missing")?
            .encode_to_vec()
            != sample.original_receipt
        {
            return Err("concurrent idempotent replay diverged".into());
        }
    }
    let before = snapshot(client, &sample.scope).await?;
    let mut changed = sample.scope.send(&sample.idempotency_key);
    changed.content = "A conflicting admission payload".into();
    let conflict = client
        .send_message(changed)
        .await
        .err()
        .ok_or("changed request accepted")?;
    if conflict.code() != Code::AlreadyExists {
        return Err("changed request returned wrong conflict status".into());
    }
    let after = snapshot(client, &sample.scope).await?;
    if after.conversation_revision != sample.final_revision
        || before.conversation_revision != sample.final_revision
        || !after.nonterminal_turns.is_empty()
        || after.recent_terminal_turns.len() != 1
        || after
            .history_counts
            .as_ref()
            .is_none_or(|counts| counts.messages != 2 || counts.turns != 1)
    {
        return Err("terminal snapshot/replay changed authoritative state".into());
    }
    Ok(())
}

async fn snapshot(client: &mut Client, scope: &Scope) -> Result<ConversationSnapshot> {
    Ok(client
        .get_conversation_snapshot(scope.snapshot())
        .await?
        .into_inner()
        .snapshot
        .ok_or("snapshot missing")?)
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn p95(mut samples: Vec<u64>) -> Result<u64> {
    if samples.len() < SAMPLES {
        return Err("at least 100 warmed samples required".into());
    }
    samples.sort_unstable();
    Ok(samples[(samples.len() * 95).div_ceil(100) - 1])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percentile_uses_nearest_rank_and_requires_enough_samples() -> Result {
        assert!(p95(vec![1; 99]).is_err());
        assert_eq!(p95((1..=100).collect())?, 95);
        assert_eq!(p95((1..=101).collect())?, 96);
        Ok(())
    }
    #[test]
    fn replay_rejects_gap_and_duplicate_cursor() {
        let update = ConversationUpdate {
            cursor: "cursor-b".into(),
            update_id: "id-b".into(),
            conversation_revision: 3,
            ..Default::default()
        };
        assert!(validate_update(&update, 1, &[], &[]).is_err());
        assert!(validate_update(&update, 2, &["cursor-b".into()], &[]).is_err());
        assert!(validate_update(&update, 2, &[], &[]).is_ok());
    }
}
