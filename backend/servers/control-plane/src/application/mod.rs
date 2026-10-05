//! Control-plane application operations and shared state.
mod state;
pub use state::with_store;
pub use state::{AppState, ReconciliationError};

pub mod onboarding;
pub mod reconciliation;
