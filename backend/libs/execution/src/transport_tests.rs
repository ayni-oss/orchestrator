use crate::{Spool, transport::deliver};
use ayni_orchestrator_telemetry::v1::{
    self,
    execution_telemetry_service_server::{
        ExecutionTelemetryService, ExecutionTelemetryServiceServer,
    },
};
use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tonic::{Request, Response, Status};

#[derive(Clone, Default)]
struct Receiver {
    records: Arc<Mutex<BTreeMap<u64, v1::TelemetryRecord>>>,
    lost: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    delay: Duration,
}
impl Receiver {
    fn commit(&self, record: &v1::TelemetryRecord) -> Result<(), Status> {
        let mut committed = self.records.lock().map_err(|_| Status::internal("lock"))?;
        if let Some(previous) = committed.get(&record.sequence) {
            assert_eq!(previous, record);
        } else {
            committed.insert(record.sequence, record.clone());
        }
        drop(committed);
        Ok(())
    }
}
#[tonic::async_trait]
impl ExecutionTelemetryService for Receiver {
    type ExchangeTelemetryStream = Pin<
        Box<dyn tokio_stream::Stream<Item = Result<v1::ExchangeTelemetryResponse, Status>> + Send>,
    >;
    async fn exchange_telemetry(
        &self,
        request: Request<tonic::Streaming<v1::ExchangeTelemetryRequest>>,
    ) -> Result<Response<Self::ExchangeTelemetryStream>, Status> {
        assert_eq!(
            request
                .metadata()
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer fixture")
        );
        let receiver = self.clone();
        let mut incoming = request.into_inner();
        Ok(Response::new(Box::pin(async_stream::try_stream! {
            while let Some(message)=incoming.message().await? {
                let record=message.record.ok_or_else(||Status::invalid_argument("record"))?;
                receiver.requests.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(receiver.delay).await;
                let source=record.source.as_ref().ok_or_else(||Status::invalid_argument("source"))?.id.clone();
                receiver.commit(&record)?;
                if !receiver.lost.swap(true,Ordering::SeqCst) {Err(Status::unavailable("fixture loses committed acknowledgement"))?;}
                yield v1::ExchangeTelemetryResponse{source_id:source,sequence:record.sequence,cancel_requested:false};
            }
        })))
    }
}

#[tokio::test]
async fn slow_acknowledgements_do_not_queue_duplicate_batches()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let spool = Arc::new(Spool::open(&directory.path().join("journal"), 1024 * 1024)?);
    for _ in 0..96 {
        spool.append(v1::TelemetryRecord {
            source: Some(v1::LogSource {
                id: "producer".into(),
                ..Default::default()
            }),
            data: b"repeated output\n".to_vec(),
            ..Default::default()
        })?;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let receiver = Receiver {
        lost: Arc::new(AtomicBool::new(true)),
        delay: Duration::from_millis(25),
        ..Default::default()
    };
    let service = receiver.clone();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(ExecutionTelemetryServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    let (stop, signal) = tokio::sync::watch::channel(false);
    let journal = spool.clone();
    let task = tokio::spawn(async move {
        deliver(journal, &format!("http://{address}"), "fixture", signal).await
    });
    let drained = tokio::time::timeout(Duration::from_secs(10), async {
        while !spool.pending().is_ok_and(|p| p.is_empty()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    stop.send(true)?;
    task.await??;
    server.abort();
    assert_eq!(
        receiver.requests.load(Ordering::SeqCst),
        96,
        "a healthy stream must send each record once, even with slow commits"
    );
    drained?;
    assert_eq!(receiver.records.lock().map_err(|_| "lock")?.len(), 96);
    Ok(())
}

#[tokio::test]
async fn reconnect_after_lost_ack_replays_exact_records_and_clears_journal()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let spool = Arc::new(Spool::open(&directory.path().join("journal"), 65536)?);
    for data in [b"same\n".as_slice(), b"same\n".as_slice(), &[0, 255, 1]] {
        spool.append(v1::TelemetryRecord {
            source: Some(v1::LogSource {
                id: "producer".into(),
                ..Default::default()
            }),
            data: data.to_vec(),
            ..Default::default()
        })?;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let receiver = Receiver::default();
    let service = receiver.clone();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder().add_service(ExecutionTelemetryServiceServer::new(service)).serve_with_incoming(async_stream::stream! {loop {match listener.accept().await {Ok((socket,_))=>yield Ok::<_,std::io::Error>(socket),Err(e)=>{yield Err(e);break;}}}}).await
    });
    let (stop, signal) = tokio::sync::watch::channel(false);
    let journal = spool.clone();
    let task = tokio::spawn(async move {
        deliver(journal, &format!("http://{address}"), "fixture", signal).await
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while !spool.pending().is_ok_and(|p| p.is_empty()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    assert_eq!(receiver.records.lock().map_err(|_| "lock")?.len(), 3);
    stop.send(true)?;
    task.await??;
    server.abort();
    assert!(spool.pending()?.is_empty());
    Ok(())
}
