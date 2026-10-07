use super::*;

pub(super) fn repository_watch(
    state: AppState,
    mut shutdown: shutdown_watch::Receiver<bool>,
    request: v1::WatchRepositoriesRequest,
    request_id: String,
) -> impl Stream<Item = Result<v1::WatchRepositoriesResponse, Status>> + Send {
    async_stream::stream! {
        let mut cursor = request.after_cursor.unwrap_or(0);
        if request.after_cursor.is_none() {
            let query = request.clone();
            match with_store(&state, move |store| store.initial_events(
                REPOSITORY_SERVICE, query.repository_connection_id.as_deref(), None)).await {
                Ok((events, high_water)) => {
                    for event in events {
                        match repository_update(&event) {
                            Ok(response) => yield Ok(response),
                            Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                        }
                    }
                    cursor = high_water;
                }
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
        }
        loop {
            let notifications = state.database.notifications();
            let notified = notifications.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let query = request.clone();
            let events = with_store(&state, move |store| store.events_after(
                REPOSITORY_SERVICE, cursor, query.repository_connection_id.as_deref(), None)).await;
            match events {
                Ok(events) => for event in events {
                    cursor = event.cursor;
                    match repository_update(&event) {
                        Ok(response) => yield Ok(response),
                        Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                    }
                },
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
            tokio::select! {
                () = notified => {},
                () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return; }
                }
            }
        }
    }
}

pub(super) fn profile_watch(
    state: AppState,
    mut shutdown: shutdown_watch::Receiver<bool>,
    request: v1::WatchRuntimeProfilesRequest,
    request_id: String,
) -> impl Stream<Item = Result<v1::WatchRuntimeProfilesResponse, Status>> + Send {
    async_stream::stream! {
        let mut cursor = request.after_cursor.unwrap_or(0);
        if request.after_cursor.is_none() {
            let query = request.clone();
            match with_store(&state, move |store| store.initial_events(
                PROFILE_SERVICE, query.repository_connection_id.as_deref(),
                query.runtime_profile_id.as_deref())).await {
                Ok((events, high_water)) => {
                    for event in events {
                        match profile_update(&event) {
                            Ok(response) => yield Ok(response),
                            Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                        }
                    }
                    cursor = high_water;
                }
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
        }
        loop {
            let notifications = state.database.notifications();
            let notified = notifications.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let query = request.clone();
            let events = with_store(&state, move |store| store.events_after(
                PROFILE_SERVICE, cursor, query.repository_connection_id.as_deref(),
                query.runtime_profile_id.as_deref())).await;
            match events {
                Ok(events) => for event in events {
                    cursor = event.cursor;
                    match profile_update(&event) {
                        Ok(response) => yield Ok(response),
                        Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                    }
                },
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
            tokio::select! {
                () = notified => {},
                () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { return; }
            }
        }
    }
}

pub(super) fn development_watch(
    state: AppState,
    mut shutdown: shutdown_watch::Receiver<bool>,
    request: v1::WatchDevelopmentTaskRequest,
    request_id: String,
) -> impl Stream<Item = Result<v1::WatchDevelopmentTaskResponse, Status>> + Send {
    async_stream::stream! {
        let mut cursor = request.after_cursor.unwrap_or(0);
        if request.after_cursor.is_none() {
            let query = request.clone();
            match with_store(&state, move |store| store.initial_events(
                DEVELOPMENT_SERVICE, None, Some(&query.development_task_id))).await {
                Ok((events, high_water)) => {
                    for event in events {
                        match development_update(&event) {
                            Ok(response) => yield Ok(response),
                            Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                        }
                    }
                    cursor = high_water;
                }
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
        }
        loop {
            let notifications = state.database.notifications();
            let notified = notifications.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let query = request.clone();
            let events = with_store(&state, move |store| store.events_after(
                DEVELOPMENT_SERVICE, cursor, None, Some(&query.development_task_id))).await;
            match events {
                Ok(events) => for event in events {
                    cursor = event.cursor;
                    match development_update(&event) {
                        Ok(response) => yield Ok(response),
                        Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                    }
                },
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
            tokio::select! {
                () = notified => {},
                () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { return; }
            }
        }
    }
}

pub(super) fn repository_update(
    event: &StoredEvent,
) -> Result<v1::WatchRepositoriesResponse, CleanStoreError> {
    let update = match event.resource_type.as_str() {
        "repository_connection" => v1::watch_repositories_response::Update::RepositoryConnection(
            decode_event(&event.payload)?,
        ),
        "repository_snapshot" => v1::watch_repositories_response::Update::RepositorySnapshot(
            decode_event(&event.payload)?,
        ),
        _ => return Err(CleanStoreError::CorruptState),
    };
    Ok(v1::WatchRepositoriesResponse {
        cursor: event.cursor,
        update: Some(update),
    })
}

pub(super) fn profile_update(
    event: &StoredEvent,
) -> Result<v1::WatchRuntimeProfilesResponse, CleanStoreError> {
    let update = match event.resource_type.as_str() {
        "runtime_profile" => v1::watch_runtime_profiles_response::Update::RuntimeProfile(
            decode_event(&event.payload)?,
        ),
        "runtime_profile_revision" => {
            v1::watch_runtime_profiles_response::Update::Revision(decode_event(&event.payload)?)
        }
        "runtime_artifact" => v1::watch_runtime_profiles_response::Update::RuntimeArtifact(
            decode_event(&event.payload)?,
        ),
        "automation_installation" => {
            v1::watch_runtime_profiles_response::Update::AutomationInstallation(decode_event(
                &event.payload,
            )?)
        }
        _ => return Err(CleanStoreError::CorruptState),
    };
    Ok(v1::WatchRuntimeProfilesResponse {
        cursor: event.cursor,
        update: Some(update),
    })
}

pub(super) fn development_update(
    event: &StoredEvent,
) -> Result<v1::WatchDevelopmentTaskResponse, CleanStoreError> {
    let update = match event.resource_type.as_str() {
        "development_task" => v1::watch_development_task_response::Update::DevelopmentTask(
            decode_event(&event.payload)?,
        ),
        "development_attempt" => {
            v1::watch_development_task_response::Update::Attempt(decode_event(&event.payload)?)
        }
        _ => return Err(CleanStoreError::CorruptState),
    };
    Ok(v1::WatchDevelopmentTaskResponse {
        cursor: event.cursor,
        update: Some(update),
    })
}

pub(super) fn decode_event<M: Message + Default>(payload: &[u8]) -> Result<M, CleanStoreError> {
    M::decode(payload).map_err(|_| CleanStoreError::CorruptState)
}

pub(super) fn stream_status(error: &CleanStoreError, request_id: &str) -> Status {
    let (code, reason) = if matches!(error, CleanStoreError::WatchCursorExpired) {
        (Code::OutOfRange, v1::ErrorReason::WatchCursorExpired)
    } else {
        (Code::Internal, v1::ErrorReason::Internal)
    };
    let message = crate::truncate_utf8(&error.to_string(), 512);
    let detail = v1::ErrorDetail {
        reason: reason as i32,
        message: message.clone(),
        resource_type: None,
        resource_id: None,
        metadata: Vec::new(),
        request_id: Some(request_id.to_owned()),
    };
    let rich = RichStatus {
        code: code as i32,
        message: message.clone(),
        details: vec![Any {
            type_url: ERROR_DETAIL_TYPE_URL.to_owned(),
            value: detail.encode_to_vec(),
        }],
    };
    Status::with_details(code, message, rich.encode_to_vec().into())
}

pub(super) fn page_response<T>(page: &crate::adapters::sqlite::Page<T>) -> v1::PageResponse {
    v1::PageResponse {
        next_page_token: page.next_page_token.clone(),
        snapshot_cursor: page.snapshot_cursor,
    }
}
pub(super) fn validate_id(
    value: &str,
    field: &'static str,
) -> Result<(), api_validation::ValidationError> {
    api_validation::resource_id(value, field)
}
pub(super) fn validate_optional_id(
    value: Option<&str>,
    field: &'static str,
) -> Result<(), api_validation::ValidationError> {
    value.map_or(Ok(()), |value| validate_id(value, field))
}
pub(super) fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
pub(super) fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct RichStatus {
    #[prost(int32, tag = "1")]
    pub(super) code: i32,
    #[prost(string, tag = "2")]
    pub(super) message: String,
    #[prost(message, repeated, tag = "3")]
    pub(super) details: Vec<Any>,
}
