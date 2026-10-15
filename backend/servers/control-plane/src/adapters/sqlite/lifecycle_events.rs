//! Bounded public lifecycle facts share accepted-fact delivery and retention.
use super::*;
use ayni_orchestrator_events::{EventIdentity, lifecycle, v1::LifecycleDecisionRecorded};
pub(super) fn append<M: Message>(
    connection: &Connection,
    resource_type: &str,
    resource_id: &str,
    revision: u64,
    cursor: u64,
    value: &M,
) -> Result<(), CleanStoreError> {
    if !matches!(
        resource_type,
        "connection"
            | "connection_deleted"
            | "certificate"
            | "certificate_deleted"
            | "project"
            | "repository_connection"
            | "repository_snapshot"
            | "repository_removed"
            | "runtime_profile_removed"
    ) {
        return Ok(());
    }
    let pending: i64 = connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM lifecycle_outbox WHERE stored_at IS NULL)+(SELECT COUNT(*) FROM verification_outbox WHERE stored_at IS NULL)+(SELECT COUNT(*) FROM runtime_operation_inbox WHERE decision='accepted' AND stored_at IS NULL)+(SELECT COUNT(*) FROM coding_inbox WHERE decision='accepted' AND stored_at IS NULL)",
            [],
            |r| r.get(0),
        )
        .map_err(CleanStoreError::Database)?;
    if pending >= super::verification_outbox::MAX_PENDING {
        return Err(CleanStoreError::BoundExceeded);
    }
    let payload = value.encode_to_vec();
    let (status, reason, active_revision) = if resource_type.starts_with("connection") {
        let projection = v1::Connection::decode(payload.as_slice())
            .map_err(|_| CleanStoreError::CorruptState)?;
        (
            projection.status,
            if projection.failure_reason.is_empty() {
                String::new()
            } else {
                "connection_operation_failed".into()
            },
            projection.observed_revision,
        )
    } else if resource_type.starts_with("certificate") {
        let projection = v1::Certificate::decode(payload.as_slice())
            .map_err(|_| CleanStoreError::CorruptState)?;
        (
            if resource_type == "certificate_deleted" {
                "deleted".into()
            } else {
                projection.status
            },
            if projection.failure_reason.is_empty() {
                String::new()
            } else {
                "certificate_operation_failed".into()
            },
            projection.revision,
        )
    } else if resource_type == "project" {
        let projection =
            v1::Project::decode(payload.as_slice()).map_err(|_| CleanStoreError::CorruptState)?;
        (
            if projection.state == v1::ProjectState::Archived as i32 {
                "archived".into()
            } else {
                "active".into()
            },
            String::new(),
            String::new(),
        )
    } else if resource_type == "repository_connection" {
        let projection = v1::RepositoryConnection::decode(payload.as_slice())
            .map_err(|_| CleanStoreError::CorruptState)?;
        (
            projection.onboarding_status,
            if projection.onboarding_reason.is_empty() {
                String::new()
            } else {
                "onboarding_operation_failed".into()
            },
            projection.onboarding_revision,
        )
    } else if resource_type.ends_with("removed") {
        ("deleted".into(), String::new(), String::new())
    } else {
        (String::new(), String::new(), String::new())
    };
    let fact = LifecycleDecisionRecorded {
        resource_type: resource_type.into(),
        resource_id: resource_id.into(),
        aggregate_revision: revision,
        event_cursor: cursor,
        projection_sha256: format!("sha256:{:x}", Sha256::digest(&payload)),
        status,
        reason,
        active_revision,
    };
    let identity = EventIdentity {
        source: "ayni://orchestrator/controllers/lifecycle".into(),
        id: Uuid::now_v7().to_string(),
    };
    let bytes = lifecycle::encode(&identity, &fact).map_err(|_| CleanStoreError::CorruptState)?;
    let now = unix_seconds()?;
    connection.execute("INSERT INTO lifecycle_journal(id,resource_id,resource_type,event_cursor,projection_sha256,aggregate_revision,payload,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![identity.id, resource_id, resource_type, i64::try_from(cursor).map_err(|_| CleanStoreError::BoundExceeded)?, fact.projection_sha256, i64::try_from(revision).map_err(|_| CleanStoreError::BoundExceeded)?, fact.encode_to_vec(), now]).map_err(CleanStoreError::Database)?;
    connection.execute("INSERT INTO lifecycle_outbox(journal_id,source,event_id,payload,next_attempt_at) VALUES(?1,?2,?1,?3,?4)", params![identity.id,identity.source,bytes,now]).map_err(CleanStoreError::Database)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connection() -> v1::Connection {
        v1::Connection {
            id: "registry".into(),
            kind: "ghcr".into(),
            management_source: "ui".into(),
            resource_version: 1,
            enabled: true,
            status: "ready".into(),
            observed_revision: "active-revision".into(),
            secret_name: "active-secret-reference".into(),
            ..Default::default()
        }
    }
    fn facts(store: &CleanStore) -> Result<Vec<LifecycleDecisionRecorded>, CleanStoreError> {
        let mut query = store
            .connection
            .prepare("SELECT payload FROM lifecycle_outbox ORDER BY rowid")
            .map_err(CleanStoreError::Database)?;
        query
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?
            .map(|r| {
                lifecycle::decode(&r.map_err(CleanStoreError::Database)?)
                    .map(|(_, fact)| fact)
                    .map_err(|_| CleanStoreError::CorruptState)
            })
            .collect()
    }
    #[test]
    fn rejected_candidate_and_delete_facts_survive_restart()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("state.sqlite");
        let mut store = CleanStore::open(&path, 3)?;
        let mut value = connection();
        store.save_connection(&value)?;
        store.save_connection(&value)?;
        assert_eq!(facts(&store)?.len(), 1);
        value.resource_version += 1;
        value.candidate_secret_name = "candidate-reference".into();
        value.status = "pending".into();
        assert!(store.connection_intent("replace", &value, "replace")?);
        assert!(!store.connection_intent("replace", &value, "replace")?);
        value.candidate_secret_name.clear();
        value.status = "ready".into();
        value.failure_reason = "registry_pull_failed".into();
        store.save_connection(&value)?;
        let accepted = facts(&store)?;
        assert_eq!(accepted.len(), 3);
        assert_eq!(accepted[2].active_revision, "active-revision");
        assert_eq!(accepted[2].reason, "connection_operation_failed");
        assert_eq!(
            store.get_connection("registry")?.secret_name,
            "active-secret-reference"
        );
        assert!(
            accepted
                .windows(2)
                .all(|w| w[0].event_cursor < w[1].event_cursor)
        );
        drop(store);
        let mut store = CleanStore::open(&path, 3)?;
        assert_eq!(facts(&store)?, accepted);
        store.delete_connection("registry")?;
        assert_eq!(
            facts(&store)?.last().map(|f| f.resource_type.as_str()),
            Some("connection_deleted")
        );
        drop(store);
        let store = CleanStore::open(&path, 3)?;
        assert!(store.get_connection("registry").is_err());
        assert_eq!(facts(&store)?.len(), 4);
        Ok(())
    }
    #[test]
    fn default_switch_records_both_resources_and_stale_mutation_records_nothing()
    -> Result<(), CleanStoreError> {
        let mut store = CleanStore::in_memory()?;
        let one = v1::Certificate {
            id: "one".into(),
            enabled: true,
            is_default: true,
            resource_version: 1,
            public_key: "f".repeat(64),
            ..Default::default()
        };
        let mut two = one.clone();
        two.id = "two".into();
        store.certificate_commit(&one, "one", "create", 0, b"one")?;
        store.certificate_commit(&two, "two", "create", 0, b"two")?;
        assert_eq!(facts(&store)?.len(), 3);
        assert_eq!(facts(&store)?[1].resource_id, "one");
        assert_eq!(facts(&store)?[1].aggregate_revision, 2);
        assert!(matches!(
            store.certificate_commit(&two, "stale", "update", 0, b"two"),
            Err(CleanStoreError::StaleVersion)
        ));
        assert_eq!(facts(&store)?.len(), 3);
        store.certificate_commit(&two, "delete", "delete", 1, b"two")?;
        assert_eq!(facts(&store)?[3].resource_type, "certificate_deleted");
        Ok(())
    }
    #[test]
    fn schema_seventeen_upgrade_and_delivery_retry_preserve_retained_facts()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("upgrade.sqlite");
        let mut store = CleanStore::open(&path, 3)?;
        store.save_connection(&connection())?;
        let retained = store.get_connection("registry")?;
        store.connection.execute_batch("DROP TABLE project_planner_sessions; DROP TABLE project_context_summaries; DROP TABLE project_context_records; DROP TABLE workspaces; DROP TABLE project_conversation_actions; DROP TABLE project_conversations; DROP TABLE project_agent_requests; DROP TABLE project_outcome_summaries; DROP TABLE lifecycle_outbox; DROP TABLE lifecycle_journal; UPDATE clean_schema SET version=17;")?;
        drop(store);
        let mut store = CleanStore::open(&path, 3)?;
        assert_eq!(store.schema_version()?, SCHEMA_VERSION);
        assert_eq!(store.get_connection("registry")?, retained);
        let mut next = retained;
        next.enabled = false;
        next.resource_version += 1;
        store.save_connection(&next)?;
        let now = unix_seconds()?;
        let publication = store.claim_lifecycle_publications(now, 8)?;
        assert_eq!(publication.len(), 1);
        assert!(store.claim_lifecycle_publications(now, 8)?.is_empty());
        store.record_lifecycle_publication(&publication[0], None)?;
        let future = now + super::super::verification_outbox::HISTORY_SECONDS + 1;
        store.prune_lifecycle_history(future)?;
        assert_eq!(facts(&store)?.len(), 1);
        drop(store);
        let mut store = CleanStore::open(&path, 3)?;
        assert_eq!(store.claim_lifecycle_publications(now + 2, 8)?, publication);
        store.record_lifecycle_publication(&publication[0], Some(9))?;
        assert!(store.claim_lifecycle_publications(now + 600, 8)?.is_empty());
        drop(store);
        let mut store = CleanStore::open(&path, 3)?;
        assert_eq!(facts(&store)?.len(), 1);
        // Delivery acknowledgement may have occurred in a later clock second.
        let acknowledged_future =
            unix_seconds()? + super::super::verification_outbox::HISTORY_SECONDS + 1;
        store.prune_lifecycle_history(acknowledged_future)?;
        assert!(facts(&store)?.is_empty());
        Ok(())
    }

    #[test]
    fn reopening_rejects_corrupt_journal_identity_and_delivery_bookkeeping()
    -> Result<(), Box<dyn std::error::Error>> {
        let corruptions = [
            "UPDATE lifecycle_journal SET resource_id='other'",
            "UPDATE lifecycle_journal SET resource_type='project'",
            "UPDATE lifecycle_journal SET event_cursor=event_cursor+1",
            "UPDATE lifecycle_journal SET projection_sha256='sha256:invalid'",
            "UPDATE lifecycle_journal SET aggregate_revision=aggregate_revision+1",
            "UPDATE lifecycle_journal SET payload=x'00'",
            "UPDATE lifecycle_journal SET created_at=-1",
            "UPDATE lifecycle_outbox SET source='ayni://orchestrator/controllers/other'",
            "UPDATE lifecycle_outbox SET event_id='other'",
            "UPDATE lifecycle_outbox SET payload=x'00'",
            "UPDATE lifecycle_outbox SET attempts=-1",
            "UPDATE lifecycle_outbox SET attempts=1000001",
            "UPDATE lifecycle_outbox SET next_attempt_at=-1",
            "UPDATE lifecycle_outbox SET stored_at=-1,broker_sequence=1",
            "UPDATE lifecycle_outbox SET broker_sequence=1",
            "UPDATE lifecycle_outbox SET stored_at=1",
            "UPDATE lifecycle_outbox SET stored_at=1,broker_sequence=0",
            "UPDATE lifecycle_outbox SET last_error='untrusted'",
            "UPDATE lifecycle_outbox SET stored_at=1,broker_sequence=1,last_error='broker_unavailable'",
        ];
        for sql in corruptions {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("corrupt.sqlite");
            let mut store = CleanStore::open(&path, 3)?;
            store.save_connection(&connection())?;
            store.connection.execute_batch(sql)?;
            drop(store);
            assert!(
                matches!(
                    CleanStore::open(&path, 3),
                    Err(CleanStoreError::CorruptState)
                ),
                "corruption accepted: {sql}"
            );
        }
        Ok(())
    }

    #[test]
    fn failed_outbox_write_rolls_back_intent_projection_and_stream() -> Result<(), CleanStoreError>
    {
        let mut store = CleanStore::in_memory()?;
        store.connection.execute_batch("CREATE TRIGGER reject_publication BEFORE INSERT ON lifecycle_outbox BEGIN SELECT RAISE(ABORT,'publication failure'); END;").map_err(CleanStoreError::Database)?;
        assert!(
            store
                .connection_intent("create", &connection(), "create")
                .is_err()
        );
        assert!(store.get_connection("registry").is_err());
        for table in [
            "connection_operations",
            "resource_events",
            "lifecycle_journal",
            "lifecycle_outbox",
        ] {
            let count: i64 = store
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .map_err(CleanStoreError::Database)?;
            assert_eq!(count, 0);
        }
        Ok(())
    }
}

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE lifecycle_journal(id TEXT PRIMARY KEY,resource_id TEXT NOT NULL,resource_type TEXT NOT NULL,event_cursor INTEGER NOT NULL,projection_sha256 TEXT NOT NULL,aggregate_revision INTEGER NOT NULL,payload BLOB NOT NULL,created_at INTEGER NOT NULL,UNIQUE(resource_type,resource_id,event_cursor));
CREATE TABLE lifecycle_outbox(journal_id TEXT PRIMARY KEY REFERENCES lifecycle_journal(id),source TEXT NOT NULL,event_id TEXT NOT NULL,payload BLOB NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,stored_at INTEGER,next_attempt_at INTEGER NOT NULL DEFAULT 0,broker_sequence INTEGER,last_error TEXT NOT NULL DEFAULT '',UNIQUE(source,event_id));
CREATE INDEX lifecycle_outbox_pending ON lifecycle_outbox(next_attempt_at,journal_id) WHERE stored_at IS NULL;
UPDATE clean_schema SET version=18 WHERE component='control_plane'; COMMIT;").map_err(CleanStoreError::Database)
}
impl CleanStore {
    pub(crate) fn claim_lifecycle_publications(
        &mut self,
        now: i64,
        remaining: usize,
    ) -> Result<Vec<super::verification_outbox::Publication>, CleanStoreError> {
        self.verification_transaction(|store| {
            let publications = {
                let mut query = store.connection.prepare("SELECT source,event_id,payload FROM lifecycle_outbox WHERE stored_at IS NULL AND next_attempt_at<=?1 ORDER BY next_attempt_at,journal_id LIMIT ?2").map_err(CleanStoreError::Database)?;
                query.query_map(params![now, i64::try_from(remaining.min(8)).map_err(|_| CleanStoreError::BoundExceeded)?], |row| Ok(super::verification_outbox::Publication { identity: EventIdentity {source:row.get(0)?,id:row.get(1)?}, bytes:row.get(2)? })).map_err(CleanStoreError::Database)?.collect::<Result<Vec<_>,_>>().map_err(CleanStoreError::Database)?
            };
            for publication in &publications {
                let attempts: u32 = store.connection.query_row("SELECT attempts FROM lifecycle_outbox WHERE event_id=?1", [&publication.identity.id], |r| r.get(0)).map_err(CleanStoreError::Database)?;
                let delay = (1_i64 << attempts.saturating_add(1).min(9)).min(300);
                store.connection.execute("UPDATE lifecycle_outbox SET attempts=MIN(attempts+1,1000000),next_attempt_at=?2 WHERE event_id=?1", params![publication.identity.id,now.saturating_add(delay)]).map_err(CleanStoreError::Database)?;
            }
            Ok(publications)
        })
    }
    pub(crate) fn record_lifecycle_publication(
        &mut self,
        publication: &super::verification_outbox::Publication,
        sequence: Option<u64>,
    ) -> Result<(), CleanStoreError> {
        let stored = sequence.map(|_| unix_seconds()).transpose()?;
        let sequence = sequence
            .map(i64::try_from)
            .transpose()
            .map_err(|_| CleanStoreError::CorruptState)?;
        if sequence.is_some_and(|value| value <= 0) {
            return Err(CleanStoreError::CorruptState);
        }
        let changed = self.connection.execute("UPDATE lifecycle_outbox SET stored_at=?4,broker_sequence=?5,last_error=?6 WHERE source=?1 AND event_id=?2 AND payload=?3 AND stored_at IS NULL", params![publication.identity.source,publication.identity.id,publication.bytes,stored,sequence,if stored.is_some() { "" } else { "broker_unavailable" }]).map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        Ok(())
    }
    pub(super) fn validate_lifecycle_events(&self) -> Result<(), CleanStoreError> {
        let mut query = self.connection.prepare("SELECT j.id,j.resource_id,j.resource_type,j.event_cursor,j.projection_sha256,j.aggregate_revision,j.payload,o.source,o.event_id,o.payload,j.created_at,o.attempts,o.next_attempt_at,o.stored_at,o.broker_sequence,o.last_error FROM lifecycle_journal j LEFT JOIN lifecycle_outbox o ON o.journal_id=j.id").map_err(CleanStoreError::Database)?;
        let mut rows = query.query([]).map_err(CleanStoreError::Database)?;
        while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
            let bytes: Vec<u8> = row.get(9).map_err(CleanStoreError::Database)?;
            let (identity, fact) =
                lifecycle::decode(&bytes).map_err(|_| CleanStoreError::CorruptState)?;
            if !journal_identity_matches(row, &identity, &fact)? || !delivery_state_valid(row)? {
                return Err(CleanStoreError::CorruptState);
            }
        }
        Ok(())
    }
    pub(super) fn prune_lifecycle_history(&mut self, now: i64) -> Result<(), CleanStoreError> {
        self.verification_transaction(|store| {
            let ids = {
                let mut query = store.connection.prepare("SELECT journal_id FROM lifecycle_outbox WHERE stored_at<?1 ORDER BY stored_at,journal_id LIMIT 128").map_err(CleanStoreError::Database)?;
                query.query_map([now.saturating_sub(super::verification_outbox::HISTORY_SECONDS)], |r| r.get::<_,String>(0)).map_err(CleanStoreError::Database)?.collect::<Result<Vec<_>,_>>().map_err(CleanStoreError::Database)?
            };
            for id in ids {
                store.connection.execute("DELETE FROM lifecycle_outbox WHERE journal_id=?1", [&id]).map_err(CleanStoreError::Database)?;
                store.connection.execute("DELETE FROM lifecycle_journal WHERE id=?1", [&id]).map_err(CleanStoreError::Database)?;
            }
            Ok(())
        })
    }
}

/// Retained envelope identity, public projection and indexed authority must agree.
fn journal_identity_matches(
    row: &rusqlite::Row<'_>,
    identity: &EventIdentity,
    fact: &LifecycleDecisionRecorded,
) -> Result<bool, CleanStoreError> {
    Ok(
        identity.id == row.get::<_, String>(0).map_err(CleanStoreError::Database)?
            && identity.id == row.get::<_, String>(8).map_err(CleanStoreError::Database)?
            && identity.source == row.get::<_, String>(7).map_err(CleanStoreError::Database)?
            && fact.resource_id == row.get::<_, String>(1).map_err(CleanStoreError::Database)?
            && fact.resource_type == row.get::<_, String>(2).map_err(CleanStoreError::Database)?
            && i64::try_from(fact.event_cursor).ok()
                == Some(row.get::<_, i64>(3).map_err(CleanStoreError::Database)?)
            && fact.projection_sha256
                == row.get::<_, String>(4).map_err(CleanStoreError::Database)?
            && i64::try_from(fact.aggregate_revision).ok()
                == Some(row.get::<_, i64>(5).map_err(CleanStoreError::Database)?)
            && fact.encode_to_vec()
                == row
                    .get::<_, Vec<u8>>(6)
                    .map_err(CleanStoreError::Database)?,
    )
}

/// Delivery acknowledgement and retry metadata never alter application authority.
fn delivery_state_valid(row: &rusqlite::Row<'_>) -> Result<bool, CleanStoreError> {
    let stored: Option<i64> = row.get(13).map_err(CleanStoreError::Database)?;
    let sequence: Option<i64> = row.get(14).map_err(CleanStoreError::Database)?;
    let error: String = row.get(15).map_err(CleanStoreError::Database)?;
    Ok(
        row.get::<_, i64>(10).map_err(CleanStoreError::Database)? >= 0
            && (0..=1_000_000).contains(&row.get::<_, i64>(11).map_err(CleanStoreError::Database)?)
            && row.get::<_, i64>(12).map_err(CleanStoreError::Database)? >= 0
            && stored.is_none_or(|value| value >= 0)
            && stored.is_some() == sequence.is_some()
            && sequence.is_none_or(|value| value > 0)
            && ["", "broker_unavailable"].contains(&error.as_str())
            && (stored.is_none() || error.is_empty()),
    )
}
