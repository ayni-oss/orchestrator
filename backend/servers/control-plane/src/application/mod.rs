//! Control-plane application operations and shared state.
mod state;
#[cfg(test)]
pub use state::with_store;
pub use state::with_store_async;
pub use state::{AppState, ReconciliationError};

pub mod onboarding;
pub mod reconciliation;
