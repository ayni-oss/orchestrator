//! Bounded execution of synchronous database operations on a dedicated thread.
use super::{CleanStore, CleanStoreError};
use std::sync::Arc;
use tokio::sync::{Notify, mpsc, oneshot, watch};

const REQUEST_CAPACITY: usize = 64;
type Operation = Box<dyn FnOnce(&mut CleanStore) + Send>;
enum Request {
    Execute(Operation),
    Stop,
}

#[derive(Clone)]
pub(crate) struct Database {
    requests: mpsc::Sender<Request>,
    stopped: watch::Receiver<bool>,
    committed: Arc<Notify>,
}

#[cfg(test)]
mod tests;

impl Database {
    pub(crate) fn start(mut store: CleanStore) -> Result<Self, CleanStoreError> {
        let (requests, mut receiver) = mpsc::channel::<Request>(REQUEST_CAPACITY);
        let (finished, stopped) = watch::channel(false);
        std::thread::Builder::new()
            .name("control-plane-database".into())
            .spawn(move || {
                while let Some(request) = receiver.blocking_recv() {
                    match request {
                        Request::Execute(operation) => operation(&mut store),
                        Request::Stop => {
                            // Reject new work, then drain everything already accepted.
                            receiver.close();
                            break;
                        }
                    }
                }
                while let Some(request) = receiver.blocking_recv() {
                    if let Request::Execute(operation) = request {
                        operation(&mut store);
                    }
                }
                drop(store);
                let _ = finished.send(true);
            })
            .map_err(CleanStoreError::DatabaseWorker)?;
        Ok(Self {
            requests,
            stopped,
            committed: Arc::new(Notify::new()),
        })
    }

    pub(crate) async fn execute<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut CleanStore) -> Result<T, CleanStoreError> + Send + 'static,
    ) -> Result<T, CleanStoreError> {
        self.call(operation).await?
    }

    pub(crate) async fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut CleanStore) -> T + Send + 'static,
    ) -> Result<T, CleanStoreError> {
        let (reply, response) = oneshot::channel();
        let committed = self.committed.clone();
        self.requests
            .send(Request::Execute(Box::new(move |store| {
                let before = store.event_watermark();
                let result = operation(store);
                let after = store.event_watermark();
                if before.ok() != after.ok() {
                    committed.notify_waiters();
                }
                // Accepted work completes even if the request caller disconnects.
                // Its durable idempotency receipt makes a retry safe.
                let _ = reply.send(result);
            })))
            .await
            .map_err(|_| CleanStoreError::StoreUnavailable)?;
        response
            .await
            .map_err(|_| CleanStoreError::StoreUnavailable)
    }

    pub(crate) async fn wait_stopped(&self) -> Result<(), CleanStoreError> {
        let mut stopped = self.stopped.clone();
        if !*stopped.borrow() {
            stopped
                .changed()
                .await
                .map_err(|_| CleanStoreError::StoreUnavailable)?;
        }
        if *stopped.borrow() {
            Ok(())
        } else {
            Err(CleanStoreError::StoreUnavailable)
        }
    }

    pub(crate) async fn shutdown(&self) -> Result<(), CleanStoreError> {
        if !*self.stopped.borrow() {
            // Another shutdown caller may already have closed the queue.
            let _ = self.requests.send(Request::Stop).await;
        }
        self.wait_stopped().await
    }

    pub(crate) fn notifications(&self) -> Arc<Notify> {
        self.committed.clone()
    }
}
