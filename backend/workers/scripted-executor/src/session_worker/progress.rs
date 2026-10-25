//! Bounded provisional observation, retained separately from terminal execution results.
use super::*;
use ayni_orchestrator_events::progress::ExecutionProgress;

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS execution_progress(command_id TEXT PRIMARY KEY,digest TEXT NOT NULL,snapshot TEXT NOT NULL); CREATE TABLE IF NOT EXISTS execution_progress_steps(command_id TEXT NOT NULL,sequence INTEGER NOT NULL,snapshot TEXT NOT NULL,PRIMARY KEY(command_id,sequence));";

#[derive(Deserialize, Serialize)]
pub struct ProgressInput {
    pub command_id: String,
    pub command_digest: String,
    pub after_sequence: u64,
}

pub(super) fn record(
    state: &WorkerState,
    command_id: &str,
    command_digest: &str,
    mut observation: ExecutionProgress,
) -> Result<(), ()> {
    let mut connection = state.connection.lock().map_err(|_| ())?;
    let tx = connection.transaction().map_err(|_| ())?;
    let prior: Option<(String, String)> = tx
        .query_row(
            "SELECT digest,snapshot FROM execution_progress WHERE command_id=?1",
            [command_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| ())?;
    let previous = prior
        .map(|(digest, json)| {
            if digest != command_digest {
                return Err(());
            }
            serde_json::from_str::<ExecutionProgress>(&json).map_err(|_| ())
        })
        .transpose()?;
    observation.sequence = previous
        .as_ref()
        .map_or(Some(1), |p| p.sequence.checked_add(1))
        .ok_or(())?;
    if !observation.valid() {
        return Err(());
    }
    let changed_stage = previous.as_ref().is_none_or(|p| {
        p.round != observation.round
            || p.call_id != observation.call_id
            || p.stage != observation.stage
            || p.tool_name != observation.tool_name
            || p.tool_invocation_id != observation.tool_invocation_id
    });
    let count: i64 = tx
        .query_row("SELECT count(*) FROM execution_progress", [], |row| {
            row.get(0)
        })
        .map_err(|_| ())?;
    let retained: i64 = tx.query_row("SELECT (SELECT COALESCE(sum(length(CAST(snapshot AS BLOB))),0) FROM execution_progress)+(SELECT COALESCE(sum(length(CAST(snapshot AS BLOB))),0) FROM execution_progress_steps)", [], |row| row.get(0)).map_err(|_| ())?;
    let json = serde_json::to_string(&observation).map_err(|_| ())?;
    // Independent observation quota bounds retained previews even if execution is interrupted.
    if count >= 50000 || retained + i64::try_from(json.len() * 2).map_err(|_| ())? > 8 * 1024 * 1024
    {
        return Err(());
    }
    tx.execute("INSERT INTO execution_progress(command_id,digest,snapshot) VALUES(?1,?2,?3) ON CONFLICT(command_id) DO UPDATE SET snapshot=excluded.snapshot", params![command_id, command_digest, json]).map_err(|_| ())?;
    if changed_stage {
        let steps: i64 = tx
            .query_row(
                "SELECT count(*) FROM execution_progress_steps WHERE command_id=?1",
                [command_id],
                |row| row.get(0),
            )
            .map_err(|_| ())?;
        if steps >= 64 {
            return Err(());
        }
        observation.provisional_text.clear();
        let json = serde_json::to_string(&observation).map_err(|_| ())?;
        tx.execute(
            "INSERT INTO execution_progress_steps(command_id,sequence,snapshot) VALUES(?1,?2,?3)",
            params![
                command_id,
                i64::try_from(observation.sequence).map_err(|_| ())?,
                json
            ],
        )
        .map_err(|_| ())?;
    }
    tx.commit().map_err(|_| ())?;
    drop(connection);
    Ok(())
}

pub(super) async fn read(
    State(state): State<Arc<WorkerState>>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<ProgressInput>,
) -> Result<axum::Json<Vec<ExecutionProgress>>, StatusCode> {
    if !authorized(&state, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let connection = state
        .connection
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let retained: Option<(String, String)> = connection
        .query_row(
            "SELECT digest,snapshot FROM execution_progress WHERE command_id=?1",
            [&input.command_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let Some((digest, latest)) = retained else {
        return Ok(axum::Json(Vec::new()));
    };
    if digest != input.command_digest {
        return Err(StatusCode::CONFLICT);
    }
    let after = i64::try_from(input.after_sequence).map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut query = connection.prepare("SELECT snapshot FROM execution_progress_steps WHERE command_id=?1 AND sequence>?2 ORDER BY sequence LIMIT 64").map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let rows = query
        .query_map(params![input.command_id, after], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut observations = rows
        .map(|row| {
            row.map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
                .and_then(|json| {
                    serde_json::from_str::<ExecutionProgress>(&json)
                        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    drop(query);
    drop(connection);
    let latest: ExecutionProgress =
        serde_json::from_str(&latest).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if latest.sequence > input.after_sequence {
        if observations
            .last()
            .is_some_and(|p| p.sequence == latest.sequence)
        {
            observations.pop();
        }
        observations.push(latest);
    }
    Ok(axum::Json(observations))
}

pub(super) fn clear(connection: &Connection, command_id: &str) -> Result<(), StatusCode> {
    connection
        .execute(
            "DELETE FROM execution_progress WHERE command_id=?1",
            [command_id],
        )
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    connection
        .execute(
            "DELETE FROM execution_progress_steps WHERE command_id=?1",
            [command_id],
        )
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(())
}
