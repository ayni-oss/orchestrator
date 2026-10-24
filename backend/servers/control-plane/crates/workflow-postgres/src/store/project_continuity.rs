//! Project facts freeze with command bytes; private working context never enters this projection.
use super::StoreError;
use ayni_orchestrator_events::{
    task_execution::TaskOutcome,
    task_publication::PublicationOutcome,
    task_quality::QualityOutcome,
    v1::{ProjectStatusSnapshot, ProjectTaskFact},
};
use tokio_postgres::Transaction;

pub(super) async fn freeze(
    tx: &Transaction<'_>,
    workspace: &str,
    project: &str,
    snapshot_id: &str,
    context_revision: u64,
) -> Result<ProjectStatusSnapshot, StoreError> {
    // One MVCC statement binds the objective, total and bounded lifecycle facts.
    // Creation ordering is deterministic; it does not claim an acceptance timestamp.
    let rows = tx.query(
        "WITH latest AS (SELECT DISTINCT ON (task_id) task_id,id,created_at
           FROM workflow_task_attempts WHERE workspace_id=$1 AND project_id=$2
           ORDER BY task_id,created_at DESC,id DESC),
         facts AS (SELECT t.id AS task_id,a.id AS attempt_id,a.created_at,
           COALESCE(r.state,CASE WHEN a.id IS NULL THEN 'awaiting_execution' ELSE 'prepared' END) AS coding_state,
           c.parent_attempt_id,q.outcome AS quality,p.state AS publication_state,
           p.publication_id,p.outcome AS publication,r.outcome AS coding,r.provider_state
         FROM workflow_tasks t
         LEFT JOIN latest a ON a.task_id=t.id
         LEFT JOIN workflow_task_runs r ON r.attempt_id=a.id
         LEFT JOIN workflow_task_quality q ON q.attempt_id=a.id
         LEFT JOIN workflow_task_publications p ON p.attempt_id=a.id
         LEFT JOIN workflow_task_recovery_confirmations c ON c.attempt_id=a.id
         WHERE t.workspace_id=$1 AND t.project_id=$2),
         bounded AS (SELECT * FROM facts ORDER BY created_at DESC NULLS LAST,task_id DESC LIMIT 20)
         SELECT project.goal,project.resource_version,(SELECT count(*) FROM facts),
           bounded.task_id,bounded.attempt_id,bounded.coding_state,bounded.parent_attempt_id,
           bounded.quality,bounded.publication_state,bounded.publication_id,bounded.publication,bounded.coding,bounded.provider_state
         FROM workflow_projects project LEFT JOIN bounded ON true
         WHERE project.workspace_id=$1 AND project.id=$2
         ORDER BY bounded.created_at DESC NULLS LAST,bounded.task_id DESC",
        &[&workspace, &project],
    ).await?;
    let first = rows.first().ok_or(StoreError::NotFound)?;
    let mut snapshot = ProjectStatusSnapshot {
        snapshot_id: snapshot_id.into(),
        project_id: project.into(),
        objective: first.get(0),
        status: "registered".into(),
        context_revision,
        project_resource_version: super::support::unsigned(first.get(1))?,
        total_tasks: Some(super::support::unsigned(first.get(2))?),
        ..Default::default()
    };
    for row in rows {
        let Some(task_id) = row.get::<_, Option<String>>(3) else {
            continue;
        };
        let quality = row
            .get::<_, Option<Vec<u8>>>(7)
            .map(|bytes| serde_json::from_slice::<QualityOutcome>(&bytes))
            .transpose()
            .map_err(|_| StoreError::CorruptState)?;
        let publication = row
            .get::<_, Option<Vec<u8>>>(10)
            .map(|bytes| serde_json::from_slice::<PublicationOutcome>(&bytes))
            .transpose()
            .map_err(|_| StoreError::CorruptState)?;
        let coding = row
            .get::<_, Option<Vec<u8>>>(11)
            .map(|bytes| serde_json::from_slice::<TaskOutcome>(&bytes))
            .transpose()
            .map_err(|_| StoreError::CorruptState)?;
        snapshot.tasks.push(ProjectTaskFact {
            provider_state: row.get::<_, Option<String>>(12).unwrap_or_default(),
            accepted_coding_state: coding.map_or_else(String::new, |c| c.state),
            task_id,
            attempt_id: row.get::<_, Option<String>>(4).unwrap_or_default(),
            parent_attempt_id: row.get::<_, Option<String>>(6).unwrap_or_default(),
            coding_state: row.get(5),
            quality_state: quality
                .as_ref()
                .map_or_else(String::new, |q| q.state.clone()),
            quality_artifact_digest: quality.map_or_else(String::new, |q| q.artifact_digest),
            publication_state: row.get::<_, Option<String>>(8).unwrap_or_default(),
            publication_id: row.get::<_, Option<String>>(9).unwrap_or_default(),
            pull_request_url: publication.map_or_else(String::new, |p| p.pr_url),
        });
    }
    snapshot.has_more_tasks = snapshot
        .total_tasks
        .is_some_and(|total| total > snapshot.tasks.len() as u64);
    Ok(snapshot)
}
