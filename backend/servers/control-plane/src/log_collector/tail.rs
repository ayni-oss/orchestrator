use super::{LogSource, Spool, observed};
use ayni_orchestrator_api::v1::TelemetryRecord;
use ayni_orchestrator_execution::now_ms;
use futures_util::io::{AsyncBufReadExt as _, AsyncReadExt as _};
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, api::LogParams};
use serde::{Deserialize, Serialize};
use std::{io, sync::Arc, time::Duration};

#[derive(Default, Serialize, Deserialize)]
struct Position {
    timestamp: String,
    chunks: u64,
}

pub(super) async fn collect(
    api: Api<Pod>,
    spool: Arc<Spool>,
    mut source: LogSource,
    previous: bool,
) -> io::Result<()> {
    let key = format!("position:{}", source.id);
    let saved: Position = spool
        .checkpoint(&key)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let resume = saved.timestamp.parse::<jiff::Timestamp>().ok();
    let since_time = resume.and_then(|t| t.checked_sub(jiff::SignedDuration::from_secs(1)).ok());
    let params = LogParams {
        container: Some(source.container.clone()),
        follow: !previous,
        previous,
        timestamps: true,
        since_time,
        ..Default::default()
    };
    let stream = api.log_stream(&source.pod_name, &params).await;
    let Ok(mut stream) = stream else {
        source.capture_status = "unavailable".into();
        observed(&spool, &source, "logs_unavailable", Vec::new())?;
        tokio::time::sleep(Duration::from_secs(5)).await;
        return Ok(());
    };
    let mut position = Position::default();
    let mut at_line_start = true;
    let mut replayed = 0u64;
    // Timestamps plus occurrence counts reconcile overlap without deleting identical lines.
    // An interrupted partial line can still be ambiguous and is explicitly labelled.
    if resume.is_some() {
        observed(
            &spool,
            &source,
            "api_reconnect_completeness_unknown",
            Vec::new(),
        )?;
    }
    loop {
        let mut bytes = Vec::with_capacity(8192);
        let count = (&mut stream)
            .take(8192)
            .read_until(b'\n', &mut bytes)
            .await?;
        if count == 0 {
            break;
        }
        let ended = bytes.last() == Some(&b'\n');
        if at_line_start
            && let Some(space) = bytes.iter().position(|b| *b == b' ')
            && let Ok(stamp) = std::str::from_utf8(&bytes[..space])
            && stamp.parse::<jiff::Timestamp>().is_ok()
        {
            if position.timestamp != stamp {
                position = Position {
                    timestamp: stamp.into(),
                    chunks: 0,
                };
            }
            bytes.drain(..=space);
        }
        at_line_start = ended;
        position.chunks += 1;
        let timestamp = position.timestamp.parse::<jiff::Timestamp>().ok();
        if let (Some(timestamp), Some(resume)) = (timestamp, resume) {
            if timestamp < resume {
                continue;
            }
            if timestamp == resume && replayed < saved.chunks {
                replayed += 1;
                continue;
            }
        }
        let record = TelemetryRecord {
            source: Some(source.clone()),
            event: "output".into(),
            stream: "combined".into(),
            data: bytes,
            observed_at_unix_ms: timestamp.map_or_else(now_ms, jiff::Timestamp::as_millisecond),
            ..Default::default()
        };
        spool.append(record)?;
        // Crash between append/checkpoint may replay overlap; source remains uncertain.
        spool.set_checkpoint(
            &key,
            &serde_json::to_string(&position).map_err(io::Error::other)?,
        )?;
    }
    if source.capture_status == "terminated_completeness_unknown" {
        spool.set_checkpoint(&format!("finished:{}", source.id), "collected")?;
    }
    if previous {
        spool.set_checkpoint(&format!("previous:{}", source.id), "collected")?;
    }
    observed(&spool, &source, "api_stream_ended", Vec::new())?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(())
}
