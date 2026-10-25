//! Stable pages of accepted facts, independent of watch replay retention.
use super::{StoreError, WorkflowStore, support};
use crate::model::Activity;
use tokio_postgres::IsolationLevel;

#[derive(Debug, Eq, PartialEq)]
pub struct TimelineEntry {
    pub activity: Activity,
    pub kind: String,
    pub affected_resource_ids: Vec<String>,
}

#[derive(Debug)]
pub struct TimelinePage {
    pub entries: Vec<TimelineEntry>,
    pub snapshot_revision: u64,
    pub next_before_revision: u64,
    pub retained_from_revision: u64,
}

impl WorkflowStore {
    /// Read newest-first history at a fixed conversation revision. Zero selects
    /// the current boundary; continuation pages retain it while new work commits.
    ///
    /// # Errors
    /// Returns invalid scope/bounds, missing conversation, or database/state errors.
    pub async fn list_execution_timeline(
        &self,
        workspace: &str,
        project: &str,
        conversation: &str,
        snapshot_revision: u64,
        before_revision: u64,
        limit: u32,
    ) -> Result<TimelinePage, StoreError> {
        support::validate_scope(workspace, project, conversation)?;
        if limit > 100 || (snapshot_revision == 0 && before_revision != 0) {
            return Err(StoreError::InvalidInput("timeline_bounds"));
        }
        let limit = if limit == 0 { 50 } else { limit };
        let snapshot = i64::try_from(snapshot_revision)
            .map_err(|_| StoreError::InvalidInput("snapshot_revision"))?;
        let before = i64::try_from(before_revision)
            .map_err(|_| StoreError::InvalidInput("before_revision"))?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let _driver = Driver(driver);
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .start()
            .await?;
        let current = support::conversation(&tx, workspace, project, conversation).await?;
        let current = i64::try_from(current.revision).map_err(|_| StoreError::CorruptState)?;
        let boundary = if snapshot == 0 { current } else { snapshot };
        if boundary > current || (before != 0 && before > boundary) {
            return Err(StoreError::InvalidInput("timeline_bounds"));
        }
        let retained: i64 = tx
            .query_one(
                "SELECT COALESCE(min(conversation_revision), 0) FROM workflow_execution_timeline \
             WHERE conversation_id = $1 AND conversation_revision <= $2",
                &[&conversation, &boundary],
            )
            .await?
            .get(0);
        let rows = tx
            .query(
                "SELECT activity_id, conversation_id, conversation_revision, 0::bigint, turn_id, \
             stage, presentation_state, waiting_on, safe_reason, workflow_ts_seconds(accepted_at), \
             workflow_ts_nanos(accepted_at), safe_detail, kind, affected_resource_ids \
             FROM workflow_execution_timeline WHERE conversation_id = $1 \
             AND conversation_revision <= $2 AND ($3 = 0 OR conversation_revision < $3) \
             ORDER BY conversation_revision DESC LIMIT $4",
                &[&conversation, &boundary, &before, &(i64::from(limit) + 1)],
            )
            .await?;
        let has_more = rows.len() > limit as usize;
        let entries = rows
            .iter()
            .take(limit as usize)
            .map(|row| {
                let mut activity = support::activity_from_row(row)?;
                // Timeline revisions are pagination boundaries, never watch cursors.
                activity.cursor.clear();
                Ok(TimelineEntry {
                    activity,
                    kind: row.get(12),
                    affected_resource_ids: row.get(13),
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        tx.commit().await?;
        let next_before_revision = if has_more {
            entries
                .last()
                .map_or(0, |entry| entry.activity.conversation_revision)
        } else {
            0
        };
        Ok(TimelinePage {
            entries,
            snapshot_revision: support::unsigned(boundary)?,
            next_before_revision,
            retained_from_revision: support::unsigned(retained)?,
        })
    }
}

struct Driver(tokio::task::JoinHandle<()>);
impl Drop for Driver {
    fn drop(&mut self) {
        self.0.abort();
    }
}
