//! Operator diagnostics ingress and bounded cursor replay on the private authority listener.
use axum::{
    Json,
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
};
use ayni_orchestrator_events::execution_diagnostics::{Diagnostic, Entry, Kind};
use ayni_orchestrator_workflow_postgres::{StoreError, WorkflowStore};
use futures_util::{Stream, stream};
use serde::Deserialize;
use std::{collections::VecDeque, convert::Infallible, time::Duration};
use tokio::{
    sync::{Semaphore, SemaphorePermit},
    time::{Instant, sleep, timeout},
};

static WATCHES: Semaphore = Semaphore::const_new(8);

#[derive(Deserialize)]
pub struct Replay {
    pub kind: Kind,
    pub id: String,
    #[serde(default)]
    pub after: i64,
}

pub async fn retain(
    store: &WorkflowStore,
    diagnostic: Diagnostic,
    runtime: bool,
) -> Result<Json<()>, StatusCode> {
    if (diagnostic.kind == Kind::Runtime) != runtime {
        return Err(StatusCode::FORBIDDEN);
    }
    store
        .record_execution_lifecycle(&diagnostic)
        .await
        .map_err(|error| status(&error))?;
    Ok(Json(()))
}

pub async fn replay(
    store: &WorkflowStore,
    query: Replay,
    runtime: bool,
) -> Result<Json<Vec<Entry>>, StatusCode> {
    if (query.kind == Kind::Runtime) != runtime {
        return Err(StatusCode::FORBIDDEN);
    }
    store
        .list_execution_diagnostics(query.kind, &query.id, query.after)
        .await
        .map(Json)
        .map_err(|error| status(&error))
}

struct Tail {
    store: WorkflowStore,
    query: Replay,
    entries: VecDeque<Entry>,
    deadline: Instant,
    _permit: SemaphorePermit<'static>,
}

pub fn tail(
    store: WorkflowStore,
    mut query: Replay,
    runtime: bool,
    headers: &HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>> + use<>>, StatusCode> {
    if (query.kind == Kind::Runtime) != runtime {
        return Err(StatusCode::FORBIDDEN);
    }
    if let Some(cursor) = headers.get("last-event-id") {
        query.after = cursor
            .to_str()
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or(StatusCode::BAD_REQUEST)?;
    }
    if uuid::Uuid::parse_str(&query.id).is_err() || query.after < 0 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let permit = WATCHES
        .try_acquire()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    let state = Tail {
        store,
        query,
        entries: VecDeque::new(),
        deadline: Instant::now() + Duration::from_secs(60),
        _permit: permit,
    };
    let events = stream::unfold(state, |mut state| async move {
        while Instant::now() < state.deadline {
            if let Some(entry) = state.entries.pop_front() {
                state.query.after = entry.cursor;
                let event = Event::default()
                    .event("diagnostic")
                    .id(entry.cursor.to_string())
                    .json_data(entry)
                    .ok()?;
                return Some((Ok(event), state));
            }
            match timeout(
                Duration::from_secs(3),
                state.store.list_execution_diagnostics(
                    state.query.kind,
                    &state.query.id,
                    state.query.after,
                ),
            )
            .await
            {
                Ok(Ok(batch)) if batch.is_empty() => sleep(Duration::from_secs(1)).await,
                Ok(Ok(batch)) => state.entries = batch.into(),
                _ => {
                    state.deadline = Instant::now();
                    return Some((
                        Ok(Event::default()
                            .event("unavailable")
                            .data("diagnostic replay unavailable")),
                        state,
                    ));
                }
            }
        }
        None
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

const fn status(error: &StoreError) -> StatusCode {
    match error {
        StoreError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        StoreError::NotFound => StatusCode::NOT_FOUND,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}
