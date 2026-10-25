//! Read recorded task phase boundaries without treating them as worker CPU time.
use super::{StoreError, WorkflowStore, support};
use crate::{model::Timestamp, plans::PlanScope};
use tokio_postgres::Row;

#[derive(Debug)]
pub struct TaskTiming {
    pub queue_accepted_at: Option<Timestamp>,
    pub claim_accepted_at: Option<Timestamp>,
    pub coding_result_accepted_at: Option<Timestamp>,
    pub quality_result_accepted_at: Option<Timestamp>,
    pub execution_stopped_at: Option<Timestamp>,
    pub publication_authorized_at: Option<Timestamp>,
    pub publication_claim_accepted_at: Option<Timestamp>,
    pub publication_result_accepted_at: Option<Timestamp>,
    pub observed_at: Option<Timestamp>,
}

fn timestamp(row: &Row, column: usize) -> Result<Option<Timestamp>, StoreError> {
    let seconds: Option<i64> = row.get(column);
    seconds
        .map(|seconds| {
            Ok(Timestamp {
                seconds,
                nanos: i32::try_from(row.get::<_, i64>(column + 1))
                    .map_err(|_| StoreError::CorruptState)?,
            })
        })
        .transpose()
}

impl WorkflowStore {
    /// Read timing only for the complete immutable attempt origin. Claims install
    /// `deadline_at` = `clock_timestamp()` + the frozen `max_seconds`, so subtracting
    /// that budget recovers the same database claim timestamp.
    ///
    /// # Errors
    /// Rejects missing scope/attempt and unavailable or corrupt retained state.
    pub async fn get_task_timing(
        &self,
        scope: PlanScope<'_>,
        task: &str,
        attempt: &str,
        max_seconds: u32,
    ) -> Result<TaskTiming, StoreError> {
        support::validate_scope(scope.workspace, scope.project, scope.conversation)?;
        support::validate_id(task, "task_id")?;
        support::validate_id(attempt, "attempt_id")?;
        let (client, driver) = crate::connect(&self.database_url).await?;
        let result = client.query_opt(
            "WITH times AS (SELECT r.created_at AS queue, \
             r.deadline_at - make_interval(secs => $6) AS claim, \
             (SELECT min(a.accepted_at) FROM workflow_execution_timeline a \
                WHERE a.conversation_id=$3 AND a.affected_resource_ids @> ARRAY[$5] \
                AND a.safe_reason='task_result_retained') AS coding, \
             q.created_at AS quality, \
             (SELECT min(a.accepted_at) FROM workflow_execution_timeline a \
                WHERE a.conversation_id=$3 AND a.affected_resource_ids @> ARRAY[$5] \
                AND a.safe_reason='task_execution_stopped') AS stopped, \
             p.created_at AS publication, \
             (SELECT min(a.accepted_at) FROM workflow_execution_timeline a \
                WHERE a.conversation_id=$3 AND a.affected_resource_ids @> ARRAY[$5] \
                AND a.safe_reason='publication_claimed') AS publication_claim, \
             (SELECT min(a.accepted_at) FROM workflow_execution_timeline a \
                WHERE a.conversation_id=$3 AND a.affected_resource_ids @> ARRAY[$5] \
                AND a.safe_reason IN ('publication_result_retained','publication_expired', \
                    'publication_cancelled_branch_may_exist')) AS publication_result, \
             clock_timestamp() AS observed \
             FROM workflow_task_runs r LEFT JOIN workflow_task_quality q ON q.attempt_id=r.attempt_id \
             LEFT JOIN workflow_task_publications p ON p.attempt_id=r.attempt_id \
             WHERE r.workspace_id=$1 AND r.project_id=$2 AND r.conversation_id=$3 \
               AND r.task_id=$4 AND r.attempt_id=$5) \
             SELECT workflow_ts_seconds(queue),workflow_ts_nanos(queue), \
             workflow_ts_seconds(claim),workflow_ts_nanos(claim), \
             workflow_ts_seconds(coding),workflow_ts_nanos(coding), \
             workflow_ts_seconds(quality),workflow_ts_nanos(quality), \
             workflow_ts_seconds(stopped),workflow_ts_nanos(stopped), \
             workflow_ts_seconds(publication),workflow_ts_nanos(publication), \
             workflow_ts_seconds(publication_claim),workflow_ts_nanos(publication_claim), \
             workflow_ts_seconds(publication_result),workflow_ts_nanos(publication_result), \
             workflow_ts_seconds(observed),workflow_ts_nanos(observed) FROM times",
            &[&scope.workspace, &scope.project, &scope.conversation, &task, &attempt, &f64::from(max_seconds)],
        ).await;
        driver.abort();
        let row = result?.ok_or(StoreError::NotFound)?;
        Ok(TaskTiming {
            queue_accepted_at: timestamp(&row, 0)?,
            claim_accepted_at: timestamp(&row, 2)?,
            coding_result_accepted_at: timestamp(&row, 4)?,
            quality_result_accepted_at: timestamp(&row, 6)?,
            execution_stopped_at: timestamp(&row, 8)?,
            publication_authorized_at: timestamp(&row, 10)?,
            publication_claim_accepted_at: timestamp(&row, 12)?,
            publication_result_accepted_at: timestamp(&row, 14)?,
            observed_at: timestamp(&row, 16)?,
        })
    }
}
