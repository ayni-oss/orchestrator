use super::watch::{decode_event, stream_status};
use super::*;

pub(super) fn connections(
    state: AppState,
    mut shutdown: shutdown_watch::Receiver<bool>,
    request: v1::WatchConnectionsRequest,
    request_id: String,
) -> impl Stream<Item = Result<v1::WatchConnectionsResponse, Status>> + Send {
    async_stream::stream! {
        let mut cursor = request.after_cursor.unwrap_or(0);
        if request.after_cursor.is_none() {
            match with_store(&state, |store| store.initial_events(CONNECTION_SERVICE, None, None)).await {
                Ok((events, high_water)) => {
                    for event in events {
                        match decode_event(&event.payload) {
                            Ok(value) => yield Ok(v1::WatchConnectionsResponse {cursor: event.cursor, connection: Some(value), deleted: false, snapshot_complete: false}),
                            Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
                        }
                    }
                    cursor = high_water;
                    yield Ok(v1::WatchConnectionsResponse {cursor, connection: None, deleted: false, snapshot_complete: true});
                }
                Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
            }
        }
        loop {
            let notifications = state.database.notifications();
            let notified = notifications.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match with_store(&state, move |store| store.events_after(CONNECTION_SERVICE, cursor, None, None)).await {
                Ok(events) => for event in events {
                    cursor = event.cursor;
                    match decode_event(&event.payload) {
                        Ok(value) => yield Ok(v1::WatchConnectionsResponse {cursor, connection: Some(value), deleted: event.resource_type.ends_with("_deleted"), snapshot_complete: false}),
                        Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
                    }
                },
                Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
            }
            tokio::select! {
                () = notified => {},
                () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() {return;}
            }
        }
    }
}

pub(super) fn certificates(
    state: AppState,
    mut shutdown: shutdown_watch::Receiver<bool>,
    request: v1::WatchCertificatesRequest,
    request_id: String,
) -> impl Stream<Item = Result<v1::WatchCertificatesResponse, Status>> + Send {
    async_stream::stream! {
        let mut cursor = request.after_cursor.unwrap_or(0);
        if request.after_cursor.is_none() {
            match with_store(&state, |store| store.initial_events(CERTIFICATE_SERVICE, None, None)).await {
                Ok((events, high_water)) => {
                    for event in events {
                        match decode_event(&event.payload) {
                            Ok(value) => yield Ok(v1::WatchCertificatesResponse {cursor: event.cursor, certificate: Some(value), deleted: false, snapshot_complete: false}),
                            Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
                        }
                    }
                    cursor = high_water;
                    yield Ok(v1::WatchCertificatesResponse {cursor, certificate: None, deleted: false, snapshot_complete: true});
                }
                Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
            }
        }
        loop {
            let notifications = state.database.notifications();
            let notified = notifications.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match with_store(&state, move |store| store.events_after(CERTIFICATE_SERVICE, cursor, None, None)).await {
                Ok(events) => for event in events {
                    cursor = event.cursor;
                    match decode_event(&event.payload) {
                        Ok(value) => yield Ok(v1::WatchCertificatesResponse {cursor, certificate: Some(value), deleted: event.resource_type.ends_with("_deleted"), snapshot_complete: false}),
                        Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
                    }
                },
                Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
            }
            tokio::select! {
                () = notified => {},
                () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() {return;}
            }
        }
    }
}

pub(super) fn projects(
    state: AppState,
    mut shutdown: shutdown_watch::Receiver<bool>,
    request: v1::WatchProjectsRequest,
    request_id: String,
) -> impl Stream<Item = Result<v1::WatchProjectsResponse, Status>> + Send {
    async_stream::stream! {
        let mut cursor = request.after_cursor.unwrap_or(0);
        if request.after_cursor.is_none() {
            match with_store(&state, |store| store.initial_events(PROJECT_SERVICE, None, None)).await {
                Ok((events, high_water)) => {
                    for event in events {
                        match decode_event(&event.payload) {
                            Ok(value) => yield Ok(v1::WatchProjectsResponse {cursor: event.cursor, project: Some(value), deleted: false, snapshot_complete: false}),
                            Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
                        }
                    }
                    cursor = high_water;
                    yield Ok(v1::WatchProjectsResponse {cursor, project: None, deleted: false, snapshot_complete: true});
                }
                Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
            }
        }
        loop {
            let notifications = state.database.notifications();
            let notified = notifications.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match with_store(&state, move |store| store.events_after(PROJECT_SERVICE, cursor, None, None)).await {
                Ok(events) => for event in events {
                    cursor = event.cursor;
                    match decode_event(&event.payload) {
                        Ok(value) => yield Ok(v1::WatchProjectsResponse {cursor, project: Some(value), deleted: event.resource_type.ends_with("_deleted"), snapshot_complete: false}),
                        Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
                    }
                },
                Err(error) => {yield Err(stream_status(&error, &request_id)); return;}
            }
            tokio::select! {
                () = notified => {},
                () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() {return;}
            }
        }
    }
}
