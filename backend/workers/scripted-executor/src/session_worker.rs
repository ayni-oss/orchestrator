//! An isolated session Pod owns only its scoped retained results and bearer token.
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use ayni_orchestrator_events::workflow::decode_command;
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    env, fs, io,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq as _;

const SCHEMA: &str = "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS identity(singleton INTEGER PRIMARY KEY CHECK(singleton=1),session_id TEXT NOT NULL,generation INTEGER NOT NULL,conversation_id TEXT NOT NULL,workspace_id TEXT NOT NULL,project_id TEXT NOT NULL); CREATE TABLE IF NOT EXISTS policy(singleton INTEGER PRIMARY KEY CHECK(singleton=1),idle_timeout_seconds INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS results(command_id TEXT PRIMARY KEY,digest TEXT NOT NULL,request BLOB,result TEXT NOT NULL,retained_bytes INTEGER NOT NULL,acknowledged INTEGER NOT NULL DEFAULT 0,acknowledged_at INTEGER);";

const RESPONSE: &str = "The scripted executor completed this turn.";
const MAX_RETAINED_BYTES: i64 = 256 * 1024 * 1024;

#[derive(Clone, Deserialize, Serialize)]
pub struct ExecutionResult {
    pub command_id: String,
    pub command_digest: String,
    pub response_content: String,
}

struct WorkerState {
    session_id: String,
    generation: u64,
    conversation_id: String,
    workspace_id: String,
    project_id: String,
    token: String,
    connection: Mutex<Connection>,
    last_request: Mutex<Instant>,
    idle_timeout: Mutex<Duration>,
}

fn load_state() -> Result<Arc<WorkerState>, io::Error> {
    let required = |name: &str| {
        env::var(name).map_err(|_| io::Error::other("session configuration unavailable"))
    };
    let session_id = required("ORCHESTRATOR_SESSION_ID")?;
    let conversation_id = required("ORCHESTRATOR_SESSION_CONVERSATION_ID")?;
    let workspace_id = required("ORCHESTRATOR_SESSION_WORKSPACE_ID")?;
    let project_id = required("ORCHESTRATOR_SESSION_PROJECT_ID")?;
    uuid::Uuid::parse_str(&workspace_id).map_err(io::Error::other)?;
    uuid::Uuid::parse_str(&project_id).map_err(io::Error::other)?;
    uuid::Uuid::parse_str(&session_id).map_err(io::Error::other)?;
    uuid::Uuid::parse_str(&conversation_id).map_err(io::Error::other)?;
    let generation = required("ORCHESTRATOR_SESSION_GENERATION")?
        .parse::<u64>()
        .map_err(io::Error::other)?;
    let timeout = required("ORCHESTRATOR_SESSION_IDLE_SECONDS")?
        .parse::<u64>()
        .map_err(io::Error::other)?;
    if generation == 0 || !(30..=86400).contains(&timeout) {
        return Err(io::Error::other("session configuration invalid"));
    }
    let token = fs::read_to_string(required("ORCHESTRATOR_SESSION_TOKEN_PATH")?)
        .map_err(|_| io::Error::other("session token unavailable"))?;
    if token.len() < 32 || token.len() > 256 {
        return Err(io::Error::other("session token invalid"));
    }
    let path = PathBuf::from(required("ORCHESTRATOR_SESSION_LEDGER_PATH")?);
    if !path.is_absolute() {
        return Err(io::Error::other("session ledger must be absolute"));
    }
    let (connection, idle_timeout) = open_ledger(
        path,
        &session_id,
        generation,
        &conversation_id,
        &workspace_id,
        &project_id,
        timeout,
    )?;
    Ok(Arc::new(WorkerState {
        session_id,
        generation,
        conversation_id,
        workspace_id,
        project_id,
        token,
        connection: Mutex::new(connection),
        last_request: Mutex::new(Instant::now()),
        idle_timeout: Mutex::new(idle_timeout),
    }))
}

fn open_ledger(
    path: PathBuf,
    session_id: &str,
    generation: u64,
    conversation_id: &str,
    workspace_id: &str,
    project_id: &str,
    timeout: u64,
) -> Result<(Connection, Duration), io::Error> {
    let connection = Connection::open(path).map_err(io::Error::other)?;
    connection.execute_batch(SCHEMA).map_err(io::Error::other)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO policy VALUES(1,?1)",
            [i64::try_from(timeout).map_err(io::Error::other)?],
        )
        .map_err(io::Error::other)?;
    let retained_timeout: i64 = connection
        .query_row(
            "SELECT idle_timeout_seconds FROM policy WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(io::Error::other)?;
    let generation_i64 = i64::try_from(generation).map_err(io::Error::other)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO identity VALUES(1,?1,?2,?3,?4,?5)",
            params![
                session_id,
                generation_i64,
                conversation_id,
                workspace_id,
                project_id
            ],
        )
        .map_err(io::Error::other)?;
    let matches: bool = connection.query_row("SELECT session_id=?1 AND generation=?2 AND conversation_id=?3 AND workspace_id=?4 AND project_id=?5 FROM identity WHERE singleton=1",params![session_id,generation_i64,conversation_id,workspace_id,project_id],|row|row.get(0)).map_err(io::Error::other)?;
    if !matches {
        return Err(io::Error::other("session ledger identity conflict"));
    }
    let timeout = Duration::from_secs(u64::try_from(retained_timeout).map_err(io::Error::other)?);
    Ok((connection, timeout))
}

pub async fn run() -> Result<(), io::Error> {
    let state = load_state()?;
    let expiry = state.clone();
    let app = Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .route("/v1/execute", post(execute))
        .route("/v1/acknowledge", post(acknowledge))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8081").await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if idle_and_resolved(&expiry) { break; }
                    },
                    _ = tokio::signal::ctrl_c() => break,
                }
            }
        })
        .await
}

fn idle_and_resolved(state: &WorkerState) -> bool {
    let Ok(connection) = state.connection.lock() else {
        return false;
    };
    let idle = state.last_request.lock().is_ok_and(|last| {
        state
            .idle_timeout
            .lock()
            .is_ok_and(|timeout| last.elapsed() >= *timeout)
    });
    idle && connection
        .query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM results WHERE acknowledged=0)",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false)
}

fn authorized(state: &WorkerState, headers: &HeaderMap) -> bool {
    let Some(value) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return false;
    };
    let supplied = Sha256::digest(value.as_bytes());
    let expected = Sha256::digest(state.token.as_bytes());
    bool::from(supplied.as_slice().ct_eq(expected.as_slice()))
}

async fn execute(
    State(state): State<Arc<WorkerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::Json<ExecutionResult>, StatusCode> {
    if !authorized(&state, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let (identity, _, command) = decode_command(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let scope = command.session.ok_or(StatusCode::BAD_REQUEST)?;
    let binding = command.binding.ok_or(StatusCode::BAD_REQUEST)?;
    if scope.session_id != state.session_id
        || scope.generation != state.generation
        || binding.conversation_id != state.conversation_id
        || binding.workspace_id != state.workspace_id
        || binding.project_id != state.project_id
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let digest = digest(&body);
    let mut connection = state
        .connection
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let tx = connection
        .transaction()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let existing: Option<(String, String)> = tx
        .query_row(
            "SELECT digest,result FROM results WHERE command_id=?1",
            [&identity.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let result = if let Some((retained_digest, json)) = existing {
        if retained_digest != digest {
            return Err(StatusCode::CONFLICT);
        }
        serde_json::from_str(&json).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    } else {
        tx.execute(
            "DELETE FROM results WHERE acknowledged=1 AND acknowledged_at < unixepoch()-691200",
            [],
        )
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let retained: i64 = tx
            .query_row(
                "SELECT COALESCE(sum(retained_bytes),0) FROM results",
                [],
                |row| row.get(0),
            )
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM results", [], |row| row.get(0))
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let bytes = i64::try_from(body.len() + 1024).map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
        if retained + bytes > MAX_RETAINED_BYTES || count >= 50000 {
            return Err(StatusCode::INSUFFICIENT_STORAGE);
        }
        let result = ExecutionResult {
            command_id: identity.id.clone(),
            command_digest: digest.clone(),
            response_content: RESPONSE.to_owned(),
        };
        let json = serde_json::to_string(&result).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        tx.execute("INSERT INTO results(command_id,digest,request,result,retained_bytes) VALUES(?1,?2,?3,?4,?5)",params![identity.id,digest,body.as_ref(),json,bytes]).map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
        result
    };
    tx.execute("INSERT INTO policy(singleton,idle_timeout_seconds) VALUES(1,?1) ON CONFLICT(singleton) DO UPDATE SET idle_timeout_seconds=excluded.idle_timeout_seconds",[i64::try_from(scope.idle_timeout_seconds).map_err(|_|StatusCode::BAD_REQUEST)?]).map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    tx.commit().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    *state
        .idle_timeout
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)? =
        Duration::from_secs(scope.idle_timeout_seconds);
    drop(connection);
    *state
        .last_request
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)? = Instant::now();
    Ok(axum::Json(result))
}

async fn acknowledge(
    State(state): State<Arc<WorkerState>>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, StatusCode> {
    if !authorized(&state, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    uuid::Uuid::parse_str(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let connection = state
        .connection
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    // Keep a bounded digest/result tombstone for exact retries; release transcript accounting only.
    connection.execute("UPDATE results SET acknowledged=1,request=NULL,retained_bytes=0,acknowledged_at=COALESCE(acknowledged_at,unixepoch()) WHERE command_id=?1",[&body]).map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    drop(connection);
    *state
        .last_request
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)? = Instant::now();
    Ok(StatusCode::NO_CONTENT)
}

pub fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "fixture construction and assertions fail immediately on unexpected state"
)]
mod tests {
    use super::*;
    use ayni_orchestrator_events::{
        EventIdentity,
        v1::{ConversationTurnBinding, ConversationTurnRequested, SessionExecutionScope},
        workflow::{APPLICATION_SOURCE, WorkflowEventMetadata, encode_command},
    };

    fn state(connection: Connection) -> Arc<WorkerState> {
        connection.execute_batch(SCHEMA).unwrap();
        Arc::new(WorkerState {
            session_id: uuid::Uuid::now_v7().to_string(),
            generation: 1,
            conversation_id: uuid::Uuid::now_v7().to_string(),
            workspace_id: uuid::Uuid::now_v7().to_string(),
            project_id: uuid::Uuid::now_v7().to_string(),
            token: "scoped-test-bearer-012345678901234567890123456789".into(),
            connection: Mutex::new(connection),
            last_request: Mutex::new(Instant::now()),
            idle_timeout: Mutex::new(Duration::from_secs(30)),
        })
    }

    fn command(
        state: &WorkerState,
    ) -> (
        EventIdentity,
        ConversationTurnRequested,
        WorkflowEventMetadata,
    ) {
        let identity = EventIdentity {
            source: APPLICATION_SOURCE.into(),
            id: uuid::Uuid::now_v7().to_string(),
        };
        let payload = ConversationTurnRequested {
            binding: Some(ConversationTurnBinding {
                workspace_id: state.workspace_id.clone(),
                project_id: state.project_id.clone(),
                conversation_id: state.conversation_id.clone(),
                turn_id: uuid::Uuid::now_v7().to_string(),
                assignment_id: uuid::Uuid::now_v7().to_string(),
                executor_generation: 1,
                execution_binding_revision: "scripted-v1".into(),
                context_revision: 1,
                command_event_id: identity.id.clone(),
                command_digest: String::new(),
            }),
            input_message_id: uuid::Uuid::now_v7().to_string(),
            input_content: "isolated input".into(),
            session: Some(SessionExecutionScope {
                session_id: state.session_id.clone(),
                generation: 1,
                idle_timeout_seconds: 30,
            }),
            ..Default::default()
        };
        let metadata = WorkflowEventMetadata {
            produced_at: "2026-10-21T12:00:00Z".into(),
            traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into(),
        };
        (identity, payload, metadata)
    }

    fn headers(state: &WorkerState) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", state.token).parse().unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn idle_expiry_waits_for_decision_and_next_turn_updates_policy() {
        let state = state(Connection::open_in_memory().unwrap());
        *state.last_request.lock().unwrap() = Instant::now()
            .checked_sub(Duration::from_secs(120))
            .unwrap();
        assert!(idle_and_resolved(&state));
        let (identity, mut payload, metadata) = command(&state);
        payload.session.as_mut().unwrap().idle_timeout_seconds = 60;
        let bytes = encode_command(&identity, &metadata, &payload).unwrap();
        let _ = execute(State(state.clone()), headers(&state), Bytes::from(bytes))
            .await
            .unwrap();
        assert_eq!(*state.idle_timeout.lock().unwrap(), Duration::from_secs(60));
        *state.last_request.lock().unwrap() = Instant::now()
            .checked_sub(Duration::from_secs(120))
            .unwrap();
        assert!(!idle_and_resolved(&state));
        acknowledge(State(state.clone()), headers(&state), identity.id)
            .await
            .unwrap();
        *state.last_request.lock().unwrap() = Instant::now()
            .checked_sub(Duration::from_secs(120))
            .unwrap();
        assert!(idle_and_resolved(&state));
        let retained: i64 = state
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT idle_timeout_seconds FROM policy WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, 60);
    }

    #[tokio::test]
    async fn scoped_request_replay_retains_one_result_until_application_decision() {
        let state = state(Connection::open_in_memory().unwrap());
        let (identity, mut payload, metadata) = command(&state);
        let bytes = encode_command(&identity, &metadata, &payload).unwrap();
        let first = execute(
            State(state.clone()),
            headers(&state),
            Bytes::from(bytes.clone()),
        )
        .await
        .unwrap()
        .0;
        let replay = execute(
            State(state.clone()),
            headers(&state),
            Bytes::from(bytes.clone()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(first.command_digest, replay.command_digest);
        let pending: i64 = state
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM results WHERE request IS NOT NULL AND acknowledged=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 1);
        payload.input_content = "changed bytes".into();
        let changed = encode_command(&identity, &metadata, &payload).unwrap();
        assert!(matches!(
            execute(State(state.clone()), headers(&state), Bytes::from(changed)).await,
            Err(StatusCode::CONFLICT)
        ));
        assert_eq!(
            acknowledge(State(state.clone()), headers(&state), identity.id)
                .await
                .unwrap(),
            StatusCode::NO_CONTENT
        );
        let pending: i64 = state
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM results WHERE request IS NOT NULL OR acknowledged=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0);
        let replay = execute(State(state.clone()), headers(&state), Bytes::from(bytes))
            .await
            .unwrap()
            .0;
        assert_eq!(first.command_digest, replay.command_digest);
    }

    #[tokio::test]
    async fn session_generation_and_all_parent_scopes_are_enforced() {
        let state = state(Connection::open_in_memory().unwrap());
        let (identity, payload, metadata) = command(&state);
        let bytes = encode_command(&identity, &metadata, &payload).unwrap();
        assert!(matches!(
            execute(State(state.clone()), HeaderMap::new(), Bytes::from(bytes)).await,
            Err(StatusCode::UNAUTHORIZED)
        ));
        for field in [
            "workspace",
            "project",
            "conversation",
            "session",
            "generation",
        ] {
            let mut changed = payload.clone();
            let binding = changed.binding.as_mut().unwrap();
            match field {
                "workspace" => binding.workspace_id = uuid::Uuid::now_v7().to_string(),
                "project" => binding.project_id = uuid::Uuid::now_v7().to_string(),
                "conversation" => binding.conversation_id = uuid::Uuid::now_v7().to_string(),
                "session" => {
                    changed.session.as_mut().unwrap().session_id = uuid::Uuid::now_v7().to_string();
                }
                _ => {
                    binding.executor_generation = 2;
                    changed.session.as_mut().unwrap().generation = 2;
                }
            }
            let bytes = encode_command(&identity, &metadata, &changed).unwrap();
            assert!(
                matches!(
                    execute(State(state.clone()), headers(&state), Bytes::from(bytes)).await,
                    Err(StatusCode::FORBIDDEN)
                ),
                "{field}"
            );
        }
    }

    #[tokio::test]
    async fn retained_result_survives_session_process_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.sqlite");
        let original = state(Connection::open(&path).unwrap());
        let (identity, payload, metadata) = command(&original);
        let bytes = encode_command(&identity, &metadata, &payload).unwrap();
        let first = execute(
            State(original.clone()),
            headers(&original),
            Bytes::from(bytes.clone()),
        )
        .await
        .unwrap()
        .0;
        let restarted = Arc::new(WorkerState {
            session_id: original.session_id.clone(),
            generation: original.generation,
            conversation_id: original.conversation_id.clone(),
            workspace_id: original.workspace_id.clone(),
            project_id: original.project_id.clone(),
            token: original.token.clone(),
            connection: Mutex::new(Connection::open(path).unwrap()),
            last_request: Mutex::new(Instant::now()),
            idle_timeout: Mutex::new(Duration::from_secs(30)),
        });
        drop(original);
        let replay = execute(
            State(restarted.clone()),
            headers(&restarted),
            Bytes::from(bytes),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(first.command_digest, replay.command_digest);
        assert_eq!(first.response_content, replay.response_content);
    }
}
