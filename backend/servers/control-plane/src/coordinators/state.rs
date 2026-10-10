//! Shared application state and durable reconciliation entry points.
//!
//! This module does not own either server transport. Resource and persistence
//! boundaries remain in the store while they are separated incrementally.

use crate::{
    adapters::github as runtime_source,
    adapters::models::ModelCatalog,
    adapters::sqlite::{CleanStore, CleanStoreError, database::Database},
};
use std::sync::Arc;
use thiserror::Error;

/// A periodic clean-v1 reconciliation pass could not inspect durable state.
#[derive(Debug, Error)]
pub enum ReconciliationError {
    #[error("durable clean-v1 reconciliation failed")]
    Store,
}

/// Shared clean-v1 API state, deliberately independent from runtime transports.
#[derive(Clone)]
pub struct AppState {
    pub(crate) database: Database,
    model_catalog: Option<ModelCatalog>,
    pub(crate) repository_lifecycle: Arc<tokio::sync::RwLock<()>>,
    pub(crate) connection_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) source: Option<Arc<runtime_source::SourceClient>>,
}

impl AppState {
    /// Opens the versioned clean-v1 store. A retired pilot database is rejected
    /// without reading or mutating its application tables.
    ///
    /// # Errors
    ///
    /// Returns [`CleanStoreError`] when the database cannot be opened safely.
    pub fn open(
        database_path: impl AsRef<std::path::Path>,
        max_attempts: u32,
    ) -> Result<Self, CleanStoreError> {
        let mut store = CleanStore::open(database_path, max_attempts)?;
        if let Some(configuration) =
            crate::process::runtime_configuration::RuntimeConfiguration::from_environment()
                .map_err(CleanStoreError::RuntimeConfiguration)?
        {
            store.configure_worker_limits(
                u32::from(configuration.coding_turns),
                u32::from(configuration.quality_repair_limit),
            );
        }
        store.integrity_check()?;
        let database = Database::start(store)?;
        Ok(Self {
            database,
            model_catalog: None,
            repository_lifecycle: Arc::new(tokio::sync::RwLock::new(())),
            connection_lock: Arc::new(tokio::sync::Mutex::new(())),
            source: None,
        })
    }

    #[must_use]
    pub fn with_model_catalog(mut self, model_catalog: ModelCatalog) -> Self {
        self.model_catalog = Some(model_catalog);
        self
    }

    pub(crate) fn model_catalog(&self) -> Option<ModelCatalog> {
        self.model_catalog.clone()
    }

    /// Observe termination of the database thread, including unexpected failure.
    pub async fn wait_for_database_exit(&self) {
        let _ = self.database.wait_stopped().await;
    }

    /// Close persistence to new requests and finish accepted database operations.
    ///
    /// # Errors
    /// Returns an error if the database thread failed before completing its queue.
    pub async fn shutdown_database(&self) -> Result<(), CleanStoreError> {
        self.database.shutdown().await
    }

    /// Reconciles shared, provider-neutral durable deadlines and retention.
    /// Profile-specific external observations are supplied by later adapters.
    ///
    /// # Errors
    ///
    /// Returns an error when durable state cannot be queued or reconciled safely.
    pub async fn reconcile_once(&self) -> Result<(), ReconciliationError> {
        self.database
            .execute(CleanStore::reconcile_once)
            .await
            .map_err(|_| ReconciliationError::Store)
    }
}

pub async fn with_store_async<T: Send + 'static>(
    state: &AppState,
    operation: impl FnOnce(&mut CleanStore) -> Result<T, CleanStoreError> + Send + 'static,
) -> Result<T, CleanStoreError> {
    state.database.execute(operation).await
}
