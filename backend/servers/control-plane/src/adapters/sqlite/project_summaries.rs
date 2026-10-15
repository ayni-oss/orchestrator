//! Accepted worker state owns conversation outcomes; model prose grants no authority.
use super::{CleanStore, CleanStoreError};
impl CleanStore {
    pub(crate) fn queue_project_outcome_summaries(&mut self) -> Result<(), CleanStoreError> {
        self.sync_project_worker_updates()
    }
}
