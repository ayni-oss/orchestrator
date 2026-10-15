//! Resident planner processes with fixed, bounded local IPC and fenced turn receipts.
use ayni_orchestrator_runtime::{
    project_agent::{
        ProjectSessionReceipt, ProjectSessionSubmission, ProjectSessionTurn, research_key,
    },
    runtime_execution::read_bounded,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};

const SUBMISSION_BYTES: usize = 2 * 1024 * 1024;
const TURN_BYTES: usize = 600 * 1024;
const RECEIPT_BYTES: usize = 262_144;
const SOCKET: &str = "/scratch/project-session.sock";

#[derive(Clone)]
struct Identity {
    session_id: String,
    generation: u64,
    pod_uid: String,
}
impl Identity {
    fn environment() -> io::Result<Self> {
        Ok(Self {
            session_id: std::env::var("PROJECT_SESSION_ID").map_err(|_| invalid())?,
            generation: std::env::var("PROJECT_SESSION_GENERATION")
                .map_err(|_| invalid())?
                .parse()
                .map_err(|_| invalid())?,
            pod_uid: std::env::var("POD_UID").map_err(|_| invalid())?,
        })
    }
    fn validate(&self, turn: &ProjectSessionTurn) -> io::Result<()> {
        if turn.session_id != self.session_id
            || turn.generation != self.generation
            || turn.pod_uid != self.pod_uid
            || turn.generation == 0
            || turn.turn_sequence == 0
            || turn.request.run_id.is_empty()
            || turn.request.run_id.len() > 512
            || turn.request.project_id.is_empty()
            || turn.request.repositories.len() > 100
        {
            return Err(invalid());
        }
        Ok(())
    }
}
fn invalid() -> io::Error {
    io::Error::other("project_session_input_invalid")
}
#[cfg(test)]
fn now() -> io::Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid())?
        .as_secs())
}
fn remaining(turn: &ProjectSessionTurn) -> io::Result<Duration> {
    let current = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid())?;
    let duration = Duration::from_secs(turn.deadline_at_unix_seconds)
        .checked_sub(current)
        .ok_or_else(|| io::Error::other("project_agent_deadline_exceeded"))?;
    if duration.is_zero() || duration > Duration::from_secs(86_400) {
        return Err(io::Error::other("project_agent_deadline_exceeded"));
    }
    Ok(duration)
}

fn encode<T: Serialize>(value: &T, maximum: usize) -> io::Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid())?;
    if bytes.len() > maximum {
        return Err(io::Error::other("project_session_input_oversized"));
    }
    Ok(bytes)
}
fn atomic_json<T: Serialize>(path: &Path, value: &T, maximum: usize) -> io::Result<()> {
    use std::io::Write as _;
    let bytes = encode(value, maximum)?;
    let temporary = path.with_extension("pending");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    fs::File::open(path.parent().ok_or_else(invalid)?)?.sync_all()
}
fn optional_json<T: DeserializeOwned>(path: &Path, maximum: usize) -> io::Result<Option<T>> {
    match read_bounded(path, maximum) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| invalid()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R, maximum: usize) -> io::Result<Vec<u8>> {
    let size = usize::try_from(reader.read_u32().await?).map_err(|_| invalid())?;
    if size == 0 || size > maximum {
        return Err(invalid());
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).await?;
    Ok(bytes)
}
async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    writer
        .write_u32(u32::try_from(bytes.len()).map_err(|_| invalid())?)
        .await?;
    writer.write_all(bytes).await?;
    writer.flush().await
}
fn read_stdin(maximum: usize) -> io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut input = std::io::stdin().lock();
    let mut header = [0; 4];
    input.read_exact(&mut header)?;
    let size = usize::try_from(u32::from_be_bytes(header)).map_err(|_| invalid())?;
    if size == 0 || size > maximum {
        return Err(invalid());
    }
    let mut bytes = vec![0; size];
    input.read_exact(&mut bytes)?;
    drop(input);
    Ok(bytes)
}
fn publish_stdout<T: Serialize>(value: &T) -> io::Result<()> {
    use std::io::Write as _;
    std::io::stdout()
        .lock()
        .write_all(&encode(value, RECEIPT_BYTES)?)
}
/// Execute only one of the fixed resident-planner commands.
/// # Errors
/// Returns safe protocol failures without printing source credentials or model inputs.
pub async fn command(operation: &str) -> io::Result<()> {
    match operation {
        "project-session-submit" => submit_client().await,
        "project-session-result" => result_client(),
        "project-session-checkout" => checkout_server().await,
        "project-session-research" => research_server().await,
        _ => Err(invalid()),
    }
}
async fn submit_client() -> io::Result<()> {
    let bytes = read_stdin(SUBMISSION_BYTES)?;
    let mut socket = UnixStream::connect(SOCKET).await?;
    write_frame(&mut socket, &bytes).await?;
    let result = read_frame(&mut socket, 1024).await?;
    if result != b"accepted" {
        return Err(io::Error::other("project_session_preparation_failed"));
    }
    publish_stdout(&serde_json::json!({"accepted":true}))
}
fn result_client() -> io::Result<()> {
    let turn: ProjectSessionTurn =
        serde_json::from_slice(&read_stdin(TURN_BYTES)?).map_err(|_| invalid())?;
    Identity::environment()?.validate(&turn)?;
    let receipt: Option<ProjectSessionReceipt> =
        optional_json(Path::new("/results/receipt.json"), RECEIPT_BYTES)?;
    let matching = receipt.filter(|receipt| turn.matches_receipt(receipt));
    publish_stdout(&matching)
}
fn previous_turn_guard(root: &Path, results: &Path, turn: &ProjectSessionTurn) -> io::Result<bool> {
    let Some(previous): Option<ProjectSessionTurn> =
        optional_json(&root.join("ready.json"), TURN_BYTES)?
    else {
        return Ok(false);
    };
    if previous.request.run_id == turn.request.run_id {
        if previous.session_id == turn.session_id
            && previous.generation == turn.generation
            && previous.turn_sequence == turn.turn_sequence
            && previous.pod_uid == turn.pod_uid
            && previous.request_digest().map_err(|_| invalid())?
                == turn.request_digest().map_err(|_| invalid())?
            && previous.deadline_at_unix_seconds == turn.deadline_at_unix_seconds
        {
            return Ok(true);
        }
        return Err(io::Error::other("project_session_turn_conflict"));
    }
    if turn.turn_sequence <= previous.turn_sequence
        || turn.request.project_id != previous.request.project_id
    {
        return Err(io::Error::other("project_session_turn_stale"));
    }
    let receipt: ProjectSessionReceipt =
        optional_json(&results.join("receipt.json"), RECEIPT_BYTES)?
            .ok_or_else(|| io::Error::other("project_session_research_busy"))?;
    if !previous.matches_receipt(&receipt) || receipt.result.is_some() != receipt.failure.is_empty()
    {
        return Err(io::Error::other("project_session_research_busy"));
    }
    Ok(false)
}
fn turn_root(root: &Path, turn: &ProjectSessionTurn) -> PathBuf {
    root.join(research_key(&turn.request.run_id))
}
async fn prepare(
    root: &Path,
    results: &Path,
    identity: &Identity,
    submission: ProjectSessionSubmission,
) -> io::Result<()> {
    let turn = submission.turn;
    identity.validate(&turn)?;
    encode(&turn, TURN_BYTES)?;
    if previous_turn_guard(root, results, &turn)? {
        return Ok(());
    }
    let duration = remaining(&turn)?;
    if submission.source_tokens.len() != turn.request.repositories.len()
        || submission
            .source_tokens
            .values()
            .any(|value| value.is_empty() || value.len() > 16_384)
    {
        return Err(invalid());
    }
    // Only completed research permits replacement or removal of its source snapshot.
    if let Some(previous) =
        optional_json::<ProjectSessionTurn>(&root.join("ready.json"), TURN_BYTES)?
    {
        fs::remove_dir_all(turn_root(root, &previous))?;
    }
    let destination = turn_root(root, &turn);
    fs::create_dir(&destination)?;
    tokio::time::timeout(
        duration,
        crate::project_agent::checkout_at(
            &turn.request,
            &destination,
            Some(&submission.source_tokens),
        ),
    )
    .await
    .map_err(|_| io::Error::other("project_agent_deadline_exceeded"))??;
    remaining(&turn)?;
    atomic_json(&root.join("ready.json"), &turn, TURN_BYTES)
}
fn same_turn(first: &ProjectSessionTurn, second: &ProjectSessionTurn) -> bool {
    first.session_id == second.session_id
        && first.generation == second.generation
        && first.turn_sequence == second.turn_sequence
        && first.pod_uid == second.pod_uid
        && first.deadline_at_unix_seconds == second.deadline_at_unix_seconds
        && first.request.run_id == second.request.run_id
        && first
            .request_digest()
            .is_ok_and(|digest| second.request_digest().is_ok_and(|other| digest == other))
}
struct Preparation {
    turn: ProjectSessionTurn,
    task: tokio::task::JoinHandle<io::Result<()>>,
}
async fn accept_submission(
    socket: &mut UnixStream,
    identity: &Identity,
    active: &mut Option<Preparation>,
) -> io::Result<()> {
    let bytes = read_frame(socket, SUBMISSION_BYTES).await?;
    let submission: ProjectSessionSubmission =
        serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    identity.validate(&submission.turn)?;
    if let Some(active) = active {
        if !same_turn(&active.turn, &submission.turn) {
            return Err(io::Error::other("project_session_research_busy"));
        }
    } else if !previous_turn_guard(
        Path::new("/workspace"),
        Path::new("/results"),
        &submission.turn,
    )? {
        remaining(&submission.turn)?;
        atomic_json(
            Path::new("/workspace/preparing.json"),
            &submission.turn,
            TURN_BYTES,
        )?;
        let identity = identity.clone();
        let turn = submission.turn.clone();
        let task = tokio::spawn(async move {
            prepare(
                Path::new("/workspace"),
                Path::new("/results"),
                &identity,
                submission,
            )
            .await
        });
        *active = Some(Preparation { turn, task });
    }
    // The resident process owns preparation before this acknowledgement. The exec
    // client and controller can disconnect without cancelling or repeating it.
    write_frame(socket, b"accepted").await
}
async fn checkout_server() -> io::Result<()> {
    let identity = Identity::environment()?;
    let listener = UnixListener::bind(SOCKET)?;
    let mut active: Option<Preparation> = None;
    loop {
        if active
            .as_ref()
            .is_some_and(|active| active.task.is_finished())
        {
            let finished = active.take().ok_or_else(invalid)?;
            finished.task.await.map_err(|_| invalid())??;
        }
        tokio::select! {
            accepted=listener.accept()=>{
                let(mut socket,_)=accepted?;
                if accept_submission(&mut socket,&identity,&mut active).await.is_err() {
                    let _=write_frame(&mut socket,b"failed").await;
                }
            }
            ()=tokio::time::sleep(Duration::from_millis(50))=>{}
        }
    }
}

fn receipt(turn: &ProjectSessionTurn) -> io::Result<ProjectSessionReceipt> {
    Ok(ProjectSessionReceipt {
        session_id: turn.session_id.clone(),
        generation: turn.generation,
        turn_sequence: turn.turn_sequence,
        pod_uid: turn.pod_uid.clone(),
        run_id: turn.request.run_id.clone(),
        request_digest: turn.request_digest().map_err(|_| invalid())?,
        result: None,
        failure: String::new(),
    })
}
async fn research_turn(
    root: &Path,
    turn: &ProjectSessionTurn,
) -> io::Result<ProjectSessionReceipt> {
    let mut receipt = receipt(turn)?;
    let duration = remaining(turn)?;
    if duration.as_secs() == 0 {
        return Err(io::Error::other("project_agent_deadline_exceeded"));
    }
    let mut request = turn.request.clone();
    request.deadline_seconds = request.deadline_seconds.min(duration.as_secs());
    let result = tokio::time::timeout(
        duration,
        crate::project_agent::run_at(request, &turn_root(root, turn)),
    )
    .await;
    match result {
        Ok(Ok(result)) => {
            if remaining(turn).is_ok() {
                receipt.result = Some(result);
            } else {
                receipt.failure = "project_agent_deadline_exceeded".into();
            }
        }
        Ok(Err(error)) => {
            receipt.failure = if error.to_string() == "project_agent_deadline_exceeded" {
                "project_agent_deadline_exceeded"
            } else {
                "project_agent_model_failed"
            }
            .into();
        }
        Err(_) => {
            receipt.failure = "project_agent_deadline_exceeded".into();
        }
    }
    if encode(&receipt, RECEIPT_BYTES).is_err() {
        receipt.result = None;
        receipt.failure = "project_result_oversized".into();
    }
    Ok(receipt)
}
async fn research_server() -> io::Result<()> {
    let identity = Identity::environment()?;
    let root = Path::new("/workspace");
    let result_path = Path::new("/results/receipt.json");
    loop {
        if let Some(turn) =
            optional_json::<ProjectSessionTurn>(&root.join("ready.json"), TURN_BYTES)?
        {
            identity.validate(&turn)?;
            let old: Option<ProjectSessionReceipt> = optional_json(result_path, RECEIPT_BYTES)?;
            if !old.as_ref().is_some_and(|old| turn.matches_receipt(old)) {
                if old
                    .as_ref()
                    .is_some_and(|old| old.turn_sequence >= turn.turn_sequence)
                {
                    return Err(invalid());
                }
                let result = if let Ok(result) = research_turn(root, &turn).await {
                    result
                } else {
                    let mut result = receipt(&turn)?;
                    result.failure = "project_agent_deadline_exceeded".into();
                    result
                };
                atomic_json(result_path, &result, RECEIPT_BYTES)?;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
#[path = "project_session_tests.rs"]
mod tests;
