//! Bounded, explicitly authorized assessment through the normal conversation ledger.
use super::{StoreError, WorkflowStore, support};
use crate::plans::PlanScope;
use serde::{Deserialize, Serialize};
use tokio_postgres::Transaction;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentAuthorization {
    pub id: String,
    pub configuration_revision_id: String,
    pub max_assessments: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentStatus {
    pub authorization: AssessmentAuthorization,
    pub admitted_assessments: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct OutcomeFact {
    task_id: String,
    attempt_id: String,
    coding_state: String,
    provider_state: String,
    quality_state: String,
    publication_state: String,
    coding_digest: String,
    quality_digest: String,
    publication_digest: String,
}

async fn snapshot(tx: &Transaction<'_>, scope: PlanScope<'_>) -> Result<Vec<u8>, StoreError> {
    // Scope includes the admitted origin. No transcript, patch or interpretation
    // enters another conversation's assessment. Hashes identify exact evidence.
    let rows = tx.query(
        "WITH latest AS (SELECT DISTINCT ON (task_id) task_id,id,created_at FROM workflow_task_attempts WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 ORDER BY task_id,created_at DESC,id DESC)
         SELECT a.task_id,a.id,r.state,r.provider_state,r.outcome,q.outcome,p.outcome
         FROM latest a JOIN workflow_task_runs r ON r.attempt_id=a.id
         LEFT JOIN workflow_task_quality q ON q.attempt_id=a.id
         LEFT JOIN workflow_task_publications p ON p.attempt_id=a.id
         WHERE r.outcome IS NOT NULL OR q.outcome IS NOT NULL OR p.outcome IS NOT NULL OR r.state IN ('failed','cancelled','interrupted','uncertain')
         ORDER BY r.assessment_outcome_at DESC,a.id DESC LIMIT 16",
        &[&scope.workspace,&scope.project,&scope.conversation],
    ).await?;
    let digest = |value: Option<Vec<u8>>| {
        value.map_or_else(String::new, |v| crate::plans::content_digest(&v))
    };
    let mut facts: Vec<_> = rows
        .iter()
        .map(|row| {
            let coding: Option<Vec<u8>> = row.get(4);
            let quality: Option<Vec<u8>> = row.get(5);
            let publication: Option<Vec<u8>> = row.get(6);
            let state = |value: &Option<Vec<u8>>| -> Result<String, StoreError> {
                value
                    .as_ref()
                    .map(|bytes| {
                        let parsed: serde_json::Value =
                            serde_json::from_slice(bytes).map_err(|_| StoreError::CorruptState)?;
                        parsed
                            .get("state")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .ok_or(StoreError::CorruptState)
                    })
                    .transpose()
                    .map(Option::unwrap_or_default)
            };
            Ok(OutcomeFact {
                task_id: row.get(0),
                attempt_id: row.get(1),
                coding_state: if coding.is_some() {
                    state(&coding)?
                } else {
                    row.get(2)
                },
                provider_state: row.get(3),
                quality_state: state(&quality)?,
                publication_state: state(&publication)?,
                coding_digest: digest(coding),
                quality_digest: digest(quality),
                publication_digest: digest(publication),
            })
        })
        .collect::<Result<_, StoreError>>()?;
    // Selection follows acceptance recency; serialization order follows identity
    // so a lifecycle timestamp alone never schedules another paid assessment.
    facts.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    serde_json::to_vec(&facts).map_err(|_| StoreError::CorruptState)
}

/// A stale assessment may remain readable, but it cannot materialize a new plan.
pub(in crate::store) async fn proposal_is_current(
    tx: &Transaction<'_>,
    turn: &str,
    scope: PlanScope<'_>,
) -> Result<bool, StoreError> {
    let Some(row) = tx
        .query_opt(
            "SELECT snapshot FROM workflow_outcome_assessments WHERE turn_id=$1",
            &[&turn],
        )
        .await?
    else {
        return Ok(true);
    };
    Ok(row.get::<_, Vec<u8>>(0) == snapshot(tx, scope).await?)
}

impl WorkflowStore {
    /// Read retained consent and consumed allowance under the exact origin.
    /// # Errors
    /// Rejects invalid or foreign scope, unavailable authority and corrupt state.
    pub async fn outcome_assessment_status(
        &self,
        scope: PlanScope<'_>,
    ) -> Result<AssessmentStatus, StoreError> {
        support::validate_scope(scope.workspace, scope.project, scope.conversation)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        support::conversation(&tx, scope.workspace, scope.project, scope.conversation).await?;
        let row=tx.query_opt("SELECT id,configuration_revision_id,max_assessments,(SELECT count(*) FROM workflow_outcome_assessments s WHERE s.authorization_id=a.id) FROM workflow_assessment_authorizations a WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 ORDER BY created_at DESC,id DESC LIMIT 1", &[&scope.workspace,&scope.project,&scope.conversation]).await?;
        let status = if let Some(row) = row {
            AssessmentStatus {
                authorization: AssessmentAuthorization {
                    id: row.get(0),
                    configuration_revision_id: row.get(1),
                    max_assessments: u32::try_from(row.get::<_, i64>(2))
                        .map_err(|_| StoreError::CorruptState)?,
                },
                admitted_assessments: u32::try_from(row.get::<_, i64>(3))
                    .map_err(|_| StoreError::CorruptState)?,
            }
        } else {
            AssessmentStatus {
                authorization: AssessmentAuthorization {
                    id: String::new(),
                    configuration_revision_id: String::new(),
                    max_assessments: 0,
                },
                admitted_assessments: 0,
            }
        };
        tx.commit().await?;
        driver.abort();
        Ok(status)
    }
    /// Permit at most ten future assessments with this exact configuration.
    /// Zero revokes admission. Already admitted turns retain cancellation controls.
    /// # Errors
    /// Rejects invalid scope, conflicting replay, changed configuration or limits.
    pub async fn authorize_outcome_assessments(
        &self,
        scope: PlanScope<'_>,
        key: &str,
        expected_configuration: &str,
        max_assessments: u32,
    ) -> Result<AssessmentAuthorization, StoreError> {
        support::validate_scope(scope.workspace, scope.project, scope.conversation)?;
        support::validate_key(key)?;
        support::validate_id(expected_configuration, "configuration_revision_id")?;
        if max_assessments > 10 {
            return Err(StoreError::InvalidInput("max_assessments"));
        }
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        support::conversation(&tx, scope.workspace, scope.project, scope.conversation).await?;
        tx.query_one(
            "SELECT id FROM workflow_conversations WHERE id=$1 FOR UPDATE",
            &[&scope.conversation],
        )
        .await?;
        let replay = tx.query_opt("SELECT id,configuration_revision_id,max_assessments FROM workflow_assessment_authorizations WHERE conversation_id=$1 AND idempotency_key=$2", &[&scope.conversation,&key]).await?;
        let authorization = if let Some(row) = replay {
            let retained = AssessmentAuthorization {
                id: row.get(0),
                configuration_revision_id: row.get(1),
                max_assessments: u32::try_from(row.get::<_, i64>(2))
                    .map_err(|_| StoreError::CorruptState)?,
            };
            if retained.configuration_revision_id != expected_configuration
                || retained.max_assessments != max_assessments
            {
                return Err(StoreError::IdempotencyConflict);
            }
            retained
        } else {
            let row=tx.query_one("SELECT revision_id FROM workflow_agent_configuration_heads WHERE workspace_id=$1 FOR SHARE", &[&scope.workspace]).await?;
            if row.get::<_, String>(0) != expected_configuration {
                return Err(StoreError::ConfigurationRevisionConflict);
            }
            let id = support::new_id();
            tx.execute("INSERT INTO workflow_assessment_authorizations(id,workspace_id,project_id,conversation_id,idempotency_key,configuration_revision_id,max_assessments) VALUES($1,$2,$3,$4,$5,$6,$7)", &[&id,&scope.workspace,&scope.project,&scope.conversation,&key,&expected_configuration,&i64::from(max_assessments)]).await?;
            AssessmentAuthorization {
                id,
                configuration_revision_id: expected_configuration.into(),
                max_assessments,
            }
        };
        tx.commit().await?;
        driver.abort();
        Ok(authorization)
    }

    /// Coalesce accepted outcomes into one normal turn per idle originating conversation.
    /// Progress-only updates never schedule a model; a changed configuration pauses consent.
    /// # Errors
    /// Returns invalid limits, unavailable authority, admission capacity or corrupt state.
    pub async fn admit_outcome_assessments(&self, limit: u32) -> Result<u32, StoreError> {
        if !(1..=32).contains(&limit) {
            return Err(StoreError::InvalidInput("limit"));
        }
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        // Serialize sweepers without blocking normal conversation admission.
        tx.query_one(
            "SELECT pg_advisory_xact_lock($1)",
            &[&(crate::migration_lock_id() + 9)],
        )
        .await?;
        let candidates=tx.query(
            "WITH latest AS (SELECT DISTINCT ON(conversation_id) * FROM workflow_assessment_authorizations ORDER BY conversation_id,created_at DESC,id DESC)
             SELECT a.id,a.workspace_id,a.project_id,a.conversation_id FROM latest a JOIN workflow_agent_configuration_heads h ON h.workspace_id=a.workspace_id AND h.revision_id=a.configuration_revision_id
             WHERE a.max_assessments>(SELECT count(*) FROM workflow_outcome_assessments s WHERE s.authorization_id=a.id)
             AND COALESCE((SELECT max(cursor) FROM workflow_updates u WHERE u.conversation_id=a.conversation_id),0)>COALESCE((SELECT max(observed_cursor) FROM workflow_assessment_scans z WHERE z.authorization_id=a.id),-1)
             AND NOT EXISTS(SELECT 1 FROM workflow_turns t WHERE t.conversation_id=a.conversation_id AND t.execution_state NOT IN('completed','failed')) ORDER BY a.id LIMIT $1", &[&i64::from(limit)]).await?;
        let mut admitted = 0;
        for row in candidates {
            let authorization: String = row.get(0);
            let workspace: String = row.get(1);
            let project: String = row.get(2);
            let conversation: String = row.get(3);
            let scope = PlanScope {
                workspace: &workspace,
                project: &project,
                conversation: &conversation,
            };
            // Normal turns and consent updates use the same origin lock.
            tx.query_one(
                "SELECT id FROM workflow_conversations WHERE id=$1 FOR UPDATE",
                &[&conversation],
            )
            .await?;
            // Keep the reviewed configuration stable through normal input freezing.
            tx.query_one("SELECT revision_id FROM workflow_agent_configuration_heads WHERE workspace_id=$1 FOR SHARE", &[&workspace]).await?;
            let still_authorized: bool = tx.query_one(
                "SELECT EXISTS(SELECT 1 FROM workflow_assessment_authorizations a WHERE a.id=$1 AND a.id=(SELECT id FROM workflow_assessment_authorizations WHERE conversation_id=$2 ORDER BY created_at DESC,id DESC LIMIT 1) AND a.configuration_revision_id=(SELECT revision_id FROM workflow_agent_configuration_heads WHERE workspace_id=$3) AND a.max_assessments>(SELECT count(*) FROM workflow_outcome_assessments WHERE authorization_id=a.id))",
                &[&authorization,&conversation,&workspace],
            ).await?.get(0);
            if !still_authorized {
                continue;
            }
            let active:bool=tx.query_one("SELECT EXISTS(SELECT 1 FROM workflow_turns WHERE conversation_id=$1 AND execution_state NOT IN('completed','failed'))", &[&conversation]).await?.get(0);
            if active {
                continue;
            }
            let observed_cursor: i64 = tx
                .query_one(
                    "SELECT COALESCE(max(cursor),0) FROM workflow_updates WHERE conversation_id=$1",
                    &[&conversation],
                )
                .await?
                .get(0);
            tx.execute("INSERT INTO workflow_assessment_scans(authorization_id,observed_cursor) VALUES($1,$2) ON CONFLICT DO NOTHING", &[&authorization,&observed_cursor]).await?;
            let bytes = snapshot(&tx, scope).await?;
            if bytes == b"[]" {
                continue;
            }
            let digest = crate::plans::content_digest(&bytes);
            if tx.query_opt("SELECT id FROM workflow_outcome_assessments WHERE authorization_id=$1 AND snapshot_digest=$2", &[&authorization,&digest]).await?.is_some(){continue;}
            let id = support::new_id();
            let content = format!(
                "Authorized task outcome assessment. Assess only the following retained facts from this conversation. Evidence digests identify exact accepted outcomes. Ordinary progress is not a new outcome. Explain uncertainty and separate coding, quality and publication. You may propose a reviewable follow-up plan with the existing proposal tool; never claim approval, merge or project completion. Selection is bounded to the 16 most recent accepted task outcomes.\n{}",
                std::str::from_utf8(&bytes).map_err(|_| StoreError::CorruptState)?
            );
            support::validate_text(&content, 16_384, false, "assessment_content")?;
            let receipt =
                super::admit_message(&tx, &id, &workspace, &project, &conversation, &content)
                    .await?;
            tx.execute("INSERT INTO workflow_outcome_assessments(id,workspace_id,project_id,conversation_id,authorization_id,snapshot_digest,snapshot,turn_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)", &[&id,&workspace,&project,&conversation,&authorization,&digest,&bytes,&receipt.turn_id]).await?;
            admitted += 1;
        }
        tx.commit().await?;
        driver.abort();
        Ok(admitted)
    }
}
