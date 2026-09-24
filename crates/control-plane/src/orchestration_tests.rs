#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::Mutex};

    fn input() -> TaskInput {
        TaskInput {
            repository_url: "https://github.com/gdurandvadas/ayni.git".to_owned(),
            revision: "42f2605d14aa980d14608a8d2f518bc05fcc47b6".to_owned(),
            lock_sha256: "606972cd98e464c8a8befc5191b770539637a9ef3b872ea3ec52abae92928e4f"
                .to_owned(),
            ayni_version: "0.12.1".to_owned(),
        }
    }
    fn key(value: &str) -> Result<IdempotencyKey, StoreError> {
        IdempotencyKey::try_from(value.to_owned()).map_err(StoreError::InvalidLifecycleData)
    }

    #[test]
    fn submission_is_idempotent_and_persists_the_initial_attempt() -> Result<(), StoreError> {
        let mut store = Store::in_memory()?;
        let input = input();
        let first = store.submit(&key("submit-1")?, &input)?;
        let second = store.submit(&key("submit-1")?, &input)?;
        assert_eq!(first, second);
        assert_eq!(first.attempts.len(), 1);
        assert_eq!(first.attempts[0].state, AttemptState::Queued);
        Ok(())
    }

    #[test]
    fn cancellation_prevents_a_late_success_report_from_reviving_the_attempt()
    -> Result<(), StoreError> {
        let mut store = Store::in_memory()?;
        let input = input();
        let task = store.submit(&key("submit-2")?, &input)?;
        let attempt = task.attempts[0].id;
        store.mark_scheduled(attempt, "managed-check-test", "test/worker:latest")?;
        store.cancel(task.id, &key("cancel-2")?)?;
        store.report(attempt, &key("report-2")?, AttemptOutcome::Succeeded)?;
        let updated = store.task(task.id)?;
        assert_eq!(
            updated.attempts[0].state,
            AttemptState::CancellationRequested
        );
        Ok(())
    }

    #[test]
    fn duplicate_report_delivery_records_one_effect() -> Result<(), StoreError> {
        let mut store = Store::in_memory()?;
        let task = store.submit(&key("submit-report")?, &input())?;
        let attempt = task.attempts[0].id;
        let report_key = key("report-duplicate")?;
        store.report(attempt, &report_key, AttemptOutcome::Failed)?;
        store.report(attempt, &report_key, AttemptOutcome::Failed)?;

        let reports: u32 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM attempt_reports WHERE attempt_id = ?1",
                [attempt.to_string()],
                |row| row.get(0),
            )
            .map_err(StoreError::Database)?;
        assert_eq!(reports, 1);
        assert_eq!(store.task(task.id)?.attempts[0].state, AttemptState::Failed);
        Ok(())
    }

    #[test]
    fn retry_limit_is_persisted_with_the_task() -> Result<(), StoreError> {
        let mut store = Store::in_memory_with_max_attempts(2)?;
        let task = store.submit(&key("submit-retry-limit")?, &input())?;
        assert_eq!(task.max_attempts, 2);
        let first_attempt = task.attempts[0].id;
        store.report(
            first_attempt,
            &key("report-first-failure")?,
            AttemptOutcome::Failed,
        )?;
        let retried = store.retry(task.id, &key("retry-once")?)?;
        assert_eq!(retried.attempts.len(), 2);
        let second_attempt = retried.attempts[1].id;
        store.report(
            second_attempt,
            &key("report-second-failure")?,
            AttemptOutcome::Failed,
        )?;

        let Err(error) = store.retry(task.id, &key("retry-beyond-limit")?) else {
            return Err(StoreError::InvalidInput("retry limit was not enforced"));
        };
        assert!(matches!(
            error,
            StoreError::MaxAttemptsReached { max_attempts: 2 }
        ));
        Ok(())
    }

    #[test]
    fn opening_a_legacy_database_adds_the_attempt_limit_column() -> Result<(), StoreError> {
        let connection = Connection::open_in_memory().map_err(StoreError::Database)?;
        connection
            .execute_batch(
                "CREATE TABLE tasks (
                    id TEXT PRIMARY KEY NOT NULL,
                    repository_url TEXT NOT NULL,
                    revision TEXT NOT NULL,
                    lock_sha256 TEXT NOT NULL,
                    ayni_version TEXT NOT NULL,
                    desired_state TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );",
            )
            .map_err(StoreError::Database)?;
        let store = Store::from_connection(connection, 2)?;
        let has_column: bool = store
            .connection
            .prepare("PRAGMA table_info(tasks)")
            .map_err(StoreError::Database)?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(StoreError::Database)?
            .try_fold(
                false,
                |found, column| Ok(found || column? == "max_attempts"),
            )
            .map_err(StoreError::Database)?;
        assert!(has_column);
        Ok(())
    }

    #[derive(Clone, Default)]
    struct FakeTransport {
        observations: Arc<Mutex<BTreeMap<String, JobObservation>>>,
    }
    impl JobTransport for FakeTransport {
        fn worker_image(&self) -> &'static str {
            "test/worker@sha256:0000000000000000000000000000000000000000000000000000000000000000"
        }

        fn ensure_job(
            &self,
            _target: &ReconciliationTarget,
            name: &str,
        ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
            let name = name.to_owned();
            let observations = Arc::clone(&self.observations);
            Box::pin(async move {
                observations
                    .lock()
                    .map_err(|_| TransportError::Kubernetes("test lock".to_owned()))?
                    .insert(name, JobObservation::Pending);
                Ok(())
            })
        }
        fn observe_job(
            &self,
            name: &str,
        ) -> Pin<Box<dyn Future<Output = Result<JobObservation, TransportError>> + Send + '_>>
        {
            let name = name.to_owned();
            let observations = Arc::clone(&self.observations);
            Box::pin(async move {
                Ok(*observations
                    .lock()
                    .map_err(|_| TransportError::Kubernetes("test lock".to_owned()))?
                    .get(&name)
                    .unwrap_or(&JobObservation::Missing))
            })
        }
        fn delete_job(
            &self,
            name: &str,
        ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
            let name = name.to_owned();
            let observations = Arc::clone(&self.observations);
            Box::pin(async move {
                observations
                    .lock()
                    .map_err(|_| TransportError::Kubernetes("test lock".to_owned()))?
                    .insert(name, JobObservation::Missing);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn reconciliation_recovers_after_a_restart_without_creating_a_second_job()
    -> Result<(), StoreError> {
        let store = Arc::new(Mutex::new(Store::in_memory()?));
        let input = input();
        let task = with_store(&store, |store| store.submit(&key("submit-3")?, &input))?;
        let transport = FakeTransport::default();
        let reconciler = Reconciler::new(Arc::clone(&store), transport.clone());
        reconciler.reconcile_once().await;
        reconciler.reconcile_once().await;
        let task = with_store(&store, |store| store.task(task.id))?;
        assert_eq!(task.attempts[0].state, AttemptState::Scheduled);
        assert_eq!(
            transport
                .observations
                .lock()
                .map_err(|_| StoreError::StoreLockPoisoned)?
                .len(),
            1
        );
        Ok(())
    }
}
