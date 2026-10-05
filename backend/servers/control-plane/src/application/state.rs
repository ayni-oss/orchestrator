//! Shared application state and durable reconciliation entry points.
//!
//! This module does not own either server transport. Resource and persistence
//! boundaries remain in the store while they are separated incrementally.

use crate::{
    adapters::github as runtime_source,
    adapters::models::ModelCatalog,
    adapters::sqlite::{CleanStore, CleanStoreError},
};
use std::sync::{Arc, Mutex};
use thiserror::Error;

type StoreHandle = Arc<Mutex<CleanStore>>;

/// A periodic clean-v1 reconciliation pass could not inspect durable state.
#[derive(Debug, Error)]
pub enum ReconciliationError {
    #[error("durable store lock was poisoned")]
    StoreLockPoisoned,
    #[error("durable clean-v1 reconciliation failed")]
    Store,
}

/// Shared clean-v1 API state, deliberately independent from runtime transports.
#[derive(Clone)]
pub struct AppState {
    pub(crate) store: StoreHandle,
    model_catalog: Option<ModelCatalog>,
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
        let store = CleanStore::open(database_path, max_attempts)?;
        store.integrity_check()?;
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            model_catalog: None,
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

    /// Reconciles shared, provider-neutral durable deadlines and retention.
    /// Profile-specific external observations are supplied by later adapters.
    ///
    /// # Errors
    ///
    /// Returns an error when durable state cannot be locked or reconciled safely.
    pub fn reconcile_once(&self) -> Result<(), ReconciliationError> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| ReconciliationError::StoreLockPoisoned)?;
        store
            .reconcile_once()
            .map_err(|_| ReconciliationError::Store)
    }
}

pub fn with_store<T>(
    state: &AppState,
    operation: impl FnOnce(&mut CleanStore) -> Result<T, CleanStoreError>,
) -> Result<T, CleanStoreError> {
    let mut store = state
        .store
        .lock()
        .map_err(|_| CleanStoreError::CorruptState)?;
    operation(&mut store)
}
