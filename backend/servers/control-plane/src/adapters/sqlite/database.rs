//! Bounded execution of synchronous database operations on a dedicated thread.
use super::{CleanStore, CleanStoreError};
use std::sync::Arc;
use tokio::sync::{Notify, mpsc, oneshot};

const REQUEST_CAPACITY: usize = 64;
type Operation = Box<dyn FnOnce(&mut CleanStore) + Send>;

#[derive(Clone)]
pub(crate) struct Database {
    requests: mpsc::Sender<Operation>,
    committed: Arc<Notify>,
}

#[cfg(test)]
mod tests;

impl Database {
    pub(crate) fn start(mut store: CleanStore) -> Result<Self, CleanStoreError> {
        let (requests, mut receiver) = mpsc::channel::<Operation>(REQUEST_CAPACITY);
        std::thread::Builder::new()
            .name("control-plane-database".into())
            .spawn(move || {
                while let Some(operation) = receiver.blocking_recv() {
                    operation(&mut store);
                }
            })
            .map_err(CleanStoreError::DatabaseWorker)?;
        Ok(Self {
            requests,
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
            .send(Box::new(move |store| {
                let before = store.event_watermark();
                let result = operation(store);
                let after = store.event_watermark();
                if before.ok() != after.ok() {
                    committed.notify_waiters();
                }
                // Accepted work completes even if the request caller disconnects.
                // Its durable idempotency receipt makes a retry safe.
                let _ = reply.send(result);
            }))
            .await
            .map_err(|_| CleanStoreError::StoreUnavailable)?;
        response
            .await
            .map_err(|_| CleanStoreError::StoreUnavailable)
    }

    pub(crate) fn notifications(&self) -> Arc<Notify> {
        self.committed.clone()
    }
}
