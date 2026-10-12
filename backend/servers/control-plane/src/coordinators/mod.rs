//! Control-plane application operations and shared state.
mod state;
pub use state::with_store_async;
pub use state::{AppState, ReconciliationError};

pub mod onboarding;
pub mod reconciliation;

mod verification;
