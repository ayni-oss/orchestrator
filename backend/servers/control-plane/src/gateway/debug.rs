use super::{ApiError, Gateway};
use axum::{
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use ayni_orchestrator_api::v1::{
    ReadLogsRequest, WatchLogsRequest, debug_logs_service_client::DebugLogsServiceClient,
};
use futures_util::StreamExt as _;
use std::{convert::Infallible, io};

pub(super) async fn watch(
    State(gateway): State<Gateway>,
    Query(mut query): Query<ReadLogsRequest>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(cursor) = headers.get("last-event-id").and_then(|v| v.to_str().ok()) {
        query.after = cursor
            .parse()
            .map_err(|_| ApiError::from(tonic::Status::invalid_argument("invalid cursor")))?;
    }
    let mut client = DebugLogsServiceClient::new(gateway.channel.clone());
    let stream = client
        .watch_logs(WatchLogsRequest { query: Some(query) })
        .await
        .map_err(ApiError::from)?
        .into_inner();
    let stream = stream.map(|result| {
        let event = match result {
            Ok(value) => {
                let cursor = value.page.as_ref().map_or(0, |p| p.next_cursor);
                Event::default()
                    .event("logs")
                    .id(cursor.to_string())
                    .json_data(value.page)
                    .unwrap_or_else(|_| {
                        Event::default()
                            .event("unavailable")
                            .data("log encoding unavailable")
                    })
            }
            Err(_) => Event::default()
                .event("unavailable")
                .data("log stream disconnected"),
        };
        Ok::<_, Infallible>(event)
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub(super) async fn download(
    State(gateway): State<Gateway>,
    Query(mut query): Query<ReadLogsRequest>,
) -> Result<Response, ApiError> {
    let mut client = DebugLogsServiceClient::new(gateway.channel.clone());
    // Authorise before returning a successful attachment response.
    let first = client
        .read_logs(query.clone())
        .await
        .map_err(ApiError::from)?
        .into_inner();
    let stream = async_stream::try_stream! {
        let through=first.high_water_cursor;
        let mut page=first;
        loop {
            let next=page.next_cursor;
            for entry in page.entries {
                if entry.cursor>through { continue; }
                if let Some(record)=entry.record&& !record.data.is_empty(){yield record.data;}
            }
            if next<=query.after || next>=through {break;}
            query.after=next;
            page=client.read_logs(query.clone()).await.map_err(|_|io::Error::other("log download interrupted"))?.into_inner();
        }
    };
    let body = Body::from_stream(Box::pin(stream)
        as std::pin::Pin<
            Box<dyn futures_util::Stream<Item = Result<Vec<u8>, io::Error>> + Send>,
        >);
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=ayni-debug-logs.bin",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response())
}
