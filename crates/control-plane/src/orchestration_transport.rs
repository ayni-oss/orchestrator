/// The Kubernetes observation for an attempt-owned Job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobObservation {
    Pending,
    Running,
    Succeeded,
    Failed,
    Missing,
}

/// Creates, observes, and deletes only deterministic attempt-owned Jobs.
pub trait JobTransport: Clone + Send + Sync + 'static {
    /// Returns the immutable worker image identity used for newly scheduled attempts.
    fn worker_image(&self) -> &str;
    /// Ensures an attempt's deterministic Job exists.
    fn ensure_job(
        &self,
        target: &ReconciliationTarget,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>>;
    /// Reads an attempt-owned Job state.
    fn observe_job(
        &self,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<JobObservation, TransportError>> + Send + '_>>;
    /// Requests deletion for an attempt-owned Job.
    fn delete_job(
        &self,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>>;
}

/// Reconciles persistent attempt state to Kubernetes Jobs.
#[derive(Clone)]
pub struct Reconciler<T> {
    store: StoreHandle,
    transport: T,
}

impl<T: JobTransport> Reconciler<T> {
    /// Builds a reconciler around one durable store and transport.
    #[must_use]
    pub const fn new(store: StoreHandle, transport: T) -> Self {
        Self { store, transport }
    }

    /// Reconciles every non-terminal attempt once.
    pub async fn reconcile_once(&self) {
        let targets = match with_store(&self.store, |store| store.reconciliation_targets()) {
            Ok(targets) => targets,
            Err(error) => {
                warn!(%error, "could not load reconciliation targets");
                return;
            }
        };
        for target in targets {
            if let Err(error) = self.reconcile_target(target).await {
                warn!(%error, "attempt reconciliation failed");
            }
        }
    }

    async fn reconcile_target(&self, target: ReconciliationTarget) -> Result<(), StoreError> {
        if target.attempt.state == AttemptState::Queued
            && target.desired_state == TaskDesiredState::Active
        {
            let job_name = job_name(target.attempt.id);
            self.transport
                .ensure_job(&target, &job_name)
                .await
                .map_err(StoreError::Transport)?;
            with_store(&self.store, |store| {
                store.mark_scheduled(target.attempt.id, &job_name, self.transport.worker_image())
            })?;
            info!(task_id = %target.task_id, attempt_id = %target.attempt.id, job_name, "scheduled attempt job");
            return Ok(());
        }
        if target.attempt.state == AttemptState::CancellationRequested {
            return self.reconcile_cancellation(&target).await;
        }
        let Some(job_name) = target.attempt.job_name.as_deref() else {
            return Ok(());
        };
        let state = match self
            .transport
            .observe_job(job_name)
            .await
            .map_err(StoreError::Transport)?
        {
            JobObservation::Pending => AttemptState::Scheduled,
            JobObservation::Running => AttemptState::Running,
            JobObservation::Succeeded => AttemptState::Succeeded,
            JobObservation::Failed => AttemptState::Failed,
            JobObservation::Missing => AttemptState::Interrupted,
        };
        with_store(&self.store, |store| store.observe(target.attempt.id, state))
    }

    async fn reconcile_cancellation(&self, target: &ReconciliationTarget) -> Result<(), StoreError> {
        let Some(job_name) = target.attempt.job_name.as_deref() else {
            return with_store(&self.store, |store| store.mark_cancelled(target.attempt.id));
        };
        self.transport.delete_job(job_name).await.map_err(StoreError::Transport)?;
        if self.transport.observe_job(job_name).await.map_err(StoreError::Transport)?
            == JobObservation::Missing {
            with_store(&self.store, |store| store.mark_cancelled(target.attempt.id))?;
        }
        Ok(())
    }
}

/// Kubernetes API transport using the control-plane service account.
#[derive(Clone)]
pub struct KubernetesJobTransport {
    client: Client,
    namespace: String,
    worker_image: String,
}

impl KubernetesJobTransport {
    /// Builds a transport scoped to one namespace.
    #[must_use]
    pub const fn new(client: Client, namespace: String, worker_image: String) -> Self {
        Self {
            client,
            namespace,
            worker_image,
        }
    }

    fn jobs(&self) -> Api<Job> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }
}

impl JobTransport for KubernetesJobTransport {
    fn worker_image(&self) -> &str {
        &self.worker_image
    }

    fn ensure_job(
        &self,
        target: &ReconciliationTarget,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
        let target = target.clone();
        let job_name = job_name.to_owned();
        Box::pin(async move {
            let job = managed_check_job(&target, &job_name, &self.worker_image)?;
            match self.jobs().create(&PostParams::default(), &job).await {
                Ok(_) => Ok(()),
                Err(create_error) => match self.jobs().get_opt(&job_name).await {
                    Ok(Some(_)) => Ok(()),
                    Ok(None) => Err(TransportError::Kubernetes(create_error.to_string())),
                    Err(get_error) => Err(TransportError::Kubernetes(get_error.to_string())),
                },
            }
        })
    }

    fn observe_job(
        &self,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<JobObservation, TransportError>> + Send + '_>> {
        let job_name = job_name.to_owned();
        Box::pin(async move {
            let Some(job) = self
                .jobs()
                .get_opt(&job_name)
                .await
                .map_err(|error| TransportError::Kubernetes(error.to_string()))?
            else {
                return Ok(JobObservation::Missing);
            };
            let status = job.status;
            if status
                .as_ref()
                .and_then(|status| status.succeeded)
                .unwrap_or_default()
                > 0
            {
                return Ok(JobObservation::Succeeded);
            }
            if status
                .as_ref()
                .and_then(|status| status.failed)
                .unwrap_or_default()
                > 0
            {
                return Ok(JobObservation::Failed);
            }
            if status
                .as_ref()
                .and_then(|status| status.active)
                .unwrap_or_default()
                > 0
            {
                Ok(JobObservation::Running)
            } else {
                Ok(JobObservation::Pending)
            }
        })
    }

    fn delete_job(
        &self,
        job_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
        let job_name = job_name.to_owned();
        Box::pin(async move {
            if self
                .jobs()
                .get_opt(&job_name)
                .await
                .map_err(|error| TransportError::Kubernetes(error.to_string()))?
                .is_some()
            {
                self.jobs()
                    .delete(&job_name, &DeleteParams::default())
                    .await
                    .map_err(|error| TransportError::Kubernetes(error.to_string()))?;
            }
            Ok(())
        })
    }
}

fn managed_check_job(
    target: &ReconciliationTarget,
    name: &str,
    worker_image: &str,
) -> Result<Job, TransportError> {
    serde_json::from_value(json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": { "name": name, "labels": { "app.kubernetes.io/name": "ayni-orchestrator-managed-check", "app.kubernetes.io/part-of": "ayni-orchestrator", "orchestrator.ayni.dev/attempt-id": target.attempt.id.to_string() } },
        "spec": {
            "backoffLimit": 0, "activeDeadlineSeconds": 900, "ttlSecondsAfterFinished": 3600,
            "template": { "metadata": { "labels": { "app.kubernetes.io/name": "ayni-orchestrator-managed-check", "orchestrator.ayni.dev/attempt-id": target.attempt.id.to_string() } }, "spec": {
                "restartPolicy": "Never", "automountServiceAccountToken": false,
                "securityContext": { "seccompProfile": { "type": "RuntimeDefault" } },
                "initContainers": [{ "name": "engine", "image": "docker:27.5.1-dind@sha256:aa3df78ecf320f5fafdce71c659f1629e96e9de0968305fe1de670e0ca9176ce", "restartPolicy": "Always", "env": [{ "name": "DOCKER_TLS_CERTDIR", "value": "" }], "securityContext": { "privileged": true }, "resources": { "requests": { "cpu": "4", "memory": "9Gi" }, "limits": { "cpu": "4", "memory": "9Gi" } }, "volumeMounts": [{ "name": "workspace", "mountPath": "/workspace" }, { "name": "engine-storage", "mountPath": "/var/lib/docker" }] }],
                "containers": [{ "name": "worker", "image": worker_image, "imagePullPolicy": "Never", "env": [
                    { "name": "DOCKER_HOST", "value": "tcp://127.0.0.1:2375" }, { "name": "DOCKER_BUILDKIT", "value": "1" }, { "name": "DOCKER_CONFIG", "value": "/tmp/docker" }, { "name": "TMPDIR", "value": "/workspace" }, { "name": "ORCHESTRATOR_ATTEMPT_ID", "value": target.attempt.id.to_string() },
                    { "name": "ORCHESTRATOR_REPOSITORY_URL", "value": target.input.repository_url }, { "name": "ORCHESTRATOR_REVISION", "value": target.input.revision },
                    { "name": "ORCHESTRATOR_LOCK_SHA256", "value": target.input.lock_sha256 }, { "name": "ORCHESTRATOR_AYNI_VERSION", "value": target.input.ayni_version },
                    { "name": "ORCHESTRATOR_WORKSPACE", "value": "/workspace/repository" }, { "name": "ORCHESTRATOR_EVIDENCE_ROOT", "value": "/evidence" }
                ], "securityContext": { "allowPrivilegeEscalation": false, "capabilities": { "drop": ["ALL"] }, "readOnlyRootFilesystem": true, "runAsNonRoot": true }, "resources": { "requests": { "cpu": "250m", "memory": "256Mi" }, "limits": { "cpu": "500m", "memory": "512Mi" } }, "volumeMounts": [{ "name": "workspace", "mountPath": "/workspace" }, { "name": "evidence", "mountPath": "/evidence" }, { "name": "scratch", "mountPath": "/tmp" }] }],
                "volumes": [{ "name": "workspace", "emptyDir": {} }, { "name": "evidence", "persistentVolumeClaim": { "claimName": "managed-check-evidence" } }, { "name": "scratch", "emptyDir": {} }, { "name": "engine-storage", "emptyDir": { "sizeLimit": "20Gi" } }]
            }}
        }
    })).map_err(|error| TransportError::Kubernetes(format!("invalid static Job template: {error}")))
}

fn job_name(attempt_id: AttemptId) -> String {
    format!(
        "managed-check-{}",
        attempt_id.to_string().replace('-', "")[..20].to_owned()
    )
}

fn with_store<T>(
    handle: &StoreHandle,
    operation: impl FnOnce(&mut Store) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let mut store = handle.lock().map_err(|_| StoreError::StoreLockPoisoned)?;
    operation(&mut store)
}

/// A durable store or reconciliation operation failed.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The persistent store directory could not be created.
    #[error("could not create durable store directory")]
    CreateStoreDirectory(#[source] std::io::Error),
    /// `SQLite` rejected an operation.
    #[error("durable store operation failed")]
    Database(#[source] rusqlite::Error),
    /// A persisted value no longer satisfies the domain schema.
    #[error("persisted lifecycle value is invalid")]
    InvalidLifecycleData(#[source] ayni_orchestrator_domain::LifecycleError),
    /// A submitted immutable identity was malformed.
    #[error("task input identity is invalid")]
    InvalidIdentity(#[source] ayni_orchestrator_domain::IdentityError),
    /// A submitted task field was invalid.
    #[error("invalid task input: {0}")]
    InvalidInput(&'static str),
    /// A requested task was absent.
    #[error("task {0} was not found")]
    TaskNotFound(TaskId),
    /// A requested attempt was absent.
    #[error("attempt {0} was not found")]
    AttemptNotFound(AttemptId),
    /// No attempt existed for a task that needed one.
    #[error("task {0} has no attempts")]
    MissingAttempt(TaskId),
    /// A retry was requested for a non-retryable attempt state.
    #[error("attempt state {0:?} cannot be retried")]
    RetryNotAllowed(AttemptState),
    /// A task has reached its immutable attempt limit.
    #[error("task has reached its maximum of {max_attempts} attempts")]
    MaxAttemptsReached {
        /// The durable attempt limit selected when the task was submitted.
        max_attempts: u32,
    },
    /// The configured per-task attempt limit did not allow even one attempt.
    #[error("maximum attempts must be greater than zero, got {0}")]
    InvalidMaxAttempts(u32),
    /// A cancelled task cannot have new work.
    #[error("task {0} is cancelled")]
    TaskCancelled(TaskId),
    /// The caller reused a key for a different state-changing action.
    #[error("idempotency key was already used for a different task: {0}")]
    ActionKeyReused(String),
    /// An attempt number exceeded the supported range.
    #[error("attempt ordinal overflow")]
    AttemptOrdinalOverflow,
    /// The local clock precedes Unix epoch.
    #[error("system clock precedes Unix epoch")]
    ClockBeforeEpoch(#[source] std::time::SystemTimeError),
    /// The Unix timestamp does not fit the store representation.
    #[error("system clock timestamp overflows durable store representation")]
    ClockOverflow,
    /// A state value was not part of the M2 model.
    #[error("unknown persisted desired state {0:?}")]
    UnknownDesiredState(String),
    /// An attempt state value was not part of the M2 model.
    #[error("unknown persisted attempt state {0:?}")]
    UnknownAttemptState(String),
    /// The process could not lock its one `SQLite` connection.
    #[error("durable store lock was poisoned")]
    StoreLockPoisoned,
    /// Kubernetes communication failed.
    #[error(transparent)]
    Transport(#[from] TransportError),
}

/// A Kubernetes Job operation failed.
#[derive(Debug, Error)]
pub enum TransportError {
    /// The Kubernetes API returned an error.
    #[error("Kubernetes API error: {0}")]
    Kubernetes(String),
}
