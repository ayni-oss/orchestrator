//! Reconcile cold repository preparation Jobs and publish readiness diagnostics.

use crate::{
    AppState, PublisherState,
    application_metrics::{ReconcilerKind, record_reconciliation_error},
    preparation::{PreparationStatus, PreparationView},
};
use k8s_openapi::api::{batch::v1::Job, core::v1::Secret};
use kube::{
    Api, Client,
    api::{DeleteParams, Patch, PatchParams, PostParams},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs, sync::Arc};
use tracing::{info, warn};

#[derive(Clone)]
pub struct PreparationReconciler {
    state: AppState,
    client: Client,
    namespace: String,
    worker_image: String,
    publisher: Arc<PublisherState>,
}

#[derive(Deserialize)]
struct Manifest {
    schema_version: u8,
    preparation_id: String,
    revision: String,
    lock_sha256: String,
    duration_seconds: u64,
}

impl PreparationReconciler {
    #[must_use]
    pub const fn new(
        state: AppState,
        client: Client,
        namespace: String,
        worker_image: String,
        publisher: Arc<PublisherState>,
    ) -> Self {
        Self {
            state,
            client,
            namespace,
            worker_image,
            publisher,
        }
    }

    pub async fn reconcile_once(&self) {
        let targets = match self.with_store(|store| store.preparation_targets()) {
            Ok(targets) => targets,
            Err(error) => {
                record_reconciliation_error(ReconcilerKind::Preparation);
                warn!(%error, "could not load repository preparations");
                return;
            }
        };
        for target in targets {
            if let Err(error) = self.reconcile(&target).await {
                record_reconciliation_error(ReconcilerKind::Preparation);
                warn!(preparation_id = target.id, %error, "repository preparation failed");
            }
        }
    }

    #[tracing::instrument(
        skip_all,
        fields(preparation_id = preparation.id, stage = ?preparation.status)
    )]
    async fn reconcile(&self, preparation: &PreparationView) -> Result<(), String> {
        match preparation.status {
            PreparationStatus::Queued => self.start(preparation).await,
            PreparationStatus::Preparing => self.finish(preparation).await,
            PreparationStatus::Ready
            | PreparationStatus::SetupRequired
            | PreparationStatus::Failed => Ok(()),
        }
    }

    async fn start(&self, preparation: &PreparationView) -> Result<(), String> {
        if self.delete_terminal_job(preparation).await? {
            return Ok(());
        }
        let repository =
            self.with_store(|store| store.repository_view(&preparation.repository_id))?;
        let token = match self.publisher.clone_token(&repository.binding).await {
            Ok(token) => token,
            Err(error) => {
                self.with_store(|store| {
                    store.fail_preparation(
                        &preparation.id,
                        PreparationStatus::SetupRequired,
                        &format!("Repository access unavailable: {error}"),
                    )
                })?;
                return Ok(());
            }
        };
        self.ensure_secret(preparation, &token).await?;
        if let Err(error) = self.ensure_job(preparation, &repository.binding).await {
            self.delete_secret(preparation).await?;
            return Err(error);
        }
        self.with_store(|store| {
            store.set_preparation_status(
                &preparation.id,
                PreparationStatus::Queued,
                PreparationStatus::Preparing,
            )
        })?;
        Ok(())
    }

    async fn finish(&self, preparation: &PreparationView) -> Result<(), String> {
        let Some(job) = self
            .jobs()
            .get_opt(&job_name(&preparation.id))
            .await
            .map_err(|error| error.to_string())?
        else {
            self.fail(preparation, "Preparation Job disappeared before completion")?;
            self.delete_secret(preparation).await?;
            return Ok(());
        };
        let status = job.status.as_ref();
        if status.and_then(|value| value.succeeded).unwrap_or(0) > 0 {
            let directory = self.publisher.preparation_root().join(&preparation.id);
            let manifest: Manifest = serde_json::from_slice(
                &fs::read(directory.join("manifest.json"))
                    .map_err(|_| "Preparation completed without a retained manifest")?,
            )
            .map_err(|_| "Preparation manifest is malformed")?;
            if manifest.schema_version != 1
                || manifest.preparation_id != preparation.id
                || manifest.revision != preparation.revision
                || manifest.lock_sha256 != preparation.lock_sha256
            {
                self.fail(
                    preparation,
                    "Preparation manifest does not match requested inputs",
                )?;
            } else {
                self.with_store(|store| {
                    store.complete_preparation(&preparation.id, manifest.duration_seconds)
                })?;
                info!(
                    preparation_id = preparation.id,
                    "repository preparation is ready"
                );
            }
            self.delete_secret(preparation).await?;
        } else if status.and_then(|value| value.failed).unwrap_or(0) > 0 {
            self.fail(
                preparation,
                "Preparation failed; inspect the preparation Job logs for the failing checkout, lock, build, or doctor step",
            )?;
            self.delete_secret(preparation).await?;
        }
        Ok(())
    }

    fn fail(&self, preparation: &PreparationView, diagnostic: &str) -> Result<(), String> {
        self.with_store(|store| {
            store.fail_preparation(&preparation.id, PreparationStatus::Failed, diagnostic)
        })?;
        Ok(())
    }

    fn with_store<T>(
        &self,
        operation: impl FnOnce(
            &mut crate::orchestration::Store,
        ) -> Result<T, crate::orchestration::StoreError>,
    ) -> Result<T, String> {
        let mut store = self.state.store.lock().map_err(|_| "store lock poisoned")?;
        operation(&mut store).map_err(|error| error.to_string())
    }

    fn jobs(&self) -> Api<Job> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }

    fn secrets(&self) -> Api<Secret> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }

    async fn ensure_secret(
        &self,
        preparation: &PreparationView,
        token: &str,
    ) -> Result<(), String> {
        let secret: Secret = serde_json::from_value(json!({
            "apiVersion":"v1", "kind":"Secret",
            "metadata":{"name":secret_name(&preparation.id),
                "labels":{"orchestrator.ayni.dev/preparation-id":suffix(&preparation.id)}},
            "type":"Opaque", "stringData":{"token":token}
        }))
        .map_err(|_| "could not build preparation clone Secret")?;
        self.secrets()
            .patch(
                &secret_name(&preparation.id),
                &PatchParams::apply("ayni-orchestrator-preparation").force(),
                &Patch::Apply(&secret),
            )
            .await
            .map_err(|error| format!("preparation Secret update failed: {error}"))?;
        Ok(())
    }

    async fn delete_secret(&self, preparation: &PreparationView) -> Result<(), String> {
        let name = secret_name(&preparation.id);
        if self
            .secrets()
            .get_opt(&name)
            .await
            .map_err(|error| error.to_string())?
            .is_some()
        {
            self.secrets()
                .delete(&name, &DeleteParams::default())
                .await
                .map_err(|error| format!("preparation Secret deletion failed: {error}"))?;
        }
        Ok(())
    }

    async fn delete_terminal_job(&self, preparation: &PreparationView) -> Result<bool, String> {
        let name = job_name(&preparation.id);
        let Some(job) = self
            .jobs()
            .get_opt(&name)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(false);
        };
        let status = job.status.as_ref();
        let terminal = status.and_then(|value| value.succeeded).unwrap_or(0) > 0
            || status.and_then(|value| value.failed).unwrap_or(0) > 0;
        if !terminal {
            return Ok(false);
        }
        self.jobs()
            .delete(&name, &DeleteParams::default())
            .await
            .map_err(|error| format!("terminal preparation Job deletion failed: {error}"))?;
        Ok(true)
    }

    #[tracing::instrument(
        skip_all,
        fields(preparation_id = preparation.id, job_name = %job_name(&preparation.id))
    )]
    async fn ensure_job(
        &self,
        preparation: &PreparationView,
        repository: &crate::development::RepositoryBinding,
    ) -> Result<(), String> {
        let job = self.build_job(preparation, repository)?;
        match self.jobs().create(&PostParams::default(), &job).await {
            Ok(_) => Ok(()),
            Err(error) => {
                let existing = self
                    .jobs()
                    .get_opt(&job_name(&preparation.id))
                    .await
                    .map_err(|lookup| {
                        format!("Job create failed: {error}; lookup failed: {lookup}")
                    })?
                    .ok_or_else(|| format!("Job create failed: {error}"))?;
                if existing
                    .metadata
                    .labels
                    .unwrap_or_default()
                    .get("orchestrator.ayni.dev/preparation-id")
                    .map(String::as_str)
                    != Some(suffix(&preparation.id))
                {
                    return Err("deterministic preparation Job belongs to another input".into());
                }
                Ok(())
            }
        }
    }

    fn build_job(
        &self,
        preparation: &PreparationView,
        repository: &crate::development::RepositoryBinding,
    ) -> Result<Job, String> {
        let labels = json!({"app.kubernetes.io/name":"ayni-orchestrator-preparation",
            "app.kubernetes.io/part-of":"ayni-orchestrator",
            "orchestrator.ayni.dev/preparation-id":suffix(&preparation.id),
            "orchestrator.ayni.dev/development-stage":"preparation"});
        let repository_url = format!(
            "https://github.com/{}/{}.git",
            repository.owner, repository.repository
        );
        let checkout = json!({"name":"checkout","image":self.worker_image,
            "imagePullPolicy":"Never","command":["/bin/sh","-ec"],
            "args":["AUTH=$(printf 'x-access-token:%s' \"$GITHUB_CLONE_TOKEN\" | base64 | tr -d '\\n'); git -c http.https://github.com/.extraheader=\"Authorization: Basic $AUTH\" init /workspace/repository; git -C /workspace/repository -c http.https://github.com/.extraheader=\"Authorization: Basic $AUTH\" fetch --depth=1 \"$REPOSITORY_URL\" \"$START_REVISION\"; git -C /workspace/repository checkout --detach FETCH_HEAD; test \"$(git -C /workspace/repository rev-parse HEAD)\" = \"$START_REVISION\""],
            "env":[{"name":"GITHUB_CLONE_TOKEN","valueFrom":{"secretKeyRef":{"name":secret_name(&preparation.id),"key":"token"}}},
                {"name":"REPOSITORY_URL","value":repository_url},
                {"name":"START_REVISION","value":preparation.revision}],
            "securityContext":worker_security(),
            "volumeMounts":[{"name":"workspace","mountPath":"/workspace"}]});
        let engine = json!({"name":"engine","image":"docker:27.5.1-dind@sha256:aa3df78ecf320f5fafdce71c659f1629e96e9de0968305fe1de670e0ca9176ce",
            "restartPolicy":"Always","env":[{"name":"DOCKER_TLS_CERTDIR","value":""}],
            "securityContext":{"privileged":true},
            "resources":{"requests":{"cpu":"4","memory":"9Gi"},"limits":{"cpu":"4","memory":"9Gi"}},
            "volumeMounts":[{"name":"workspace","mountPath":"/workspace"},{"name":"engine-storage","mountPath":"/var/lib/docker"}]});
        let preparer = json!({"name":"preparer","image":self.worker_image,"imagePullPolicy":"Never",
            "command":["/usr/local/bin/prepare-repository"],
            "env":[{"name":"DEVELOPMENT_STAGE","value":"preparation"},
                {"name":"JOB_NAME","valueFrom":{"fieldRef":{"fieldPath":"metadata.name"}}},
                {"name":"DOCKER_HOST","value":"tcp://127.0.0.1:2375"},
                {"name":"DOCKER_BUILDKIT","value":"1"},{"name":"DOCKER_CONFIG","value":"/tmp/docker"},
                {"name":"TMPDIR","value":"/workspace"},{"name":"DEVELOPMENT_WORKSPACE","value":"/workspace/repository"},
                {"name":"DEVELOPMENT_PREPARATION_DIR","value":format!("/preparations/{}",preparation.id)},
                {"name":"DEVELOPMENT_PREPARATION_ID","value":preparation.id},
                {"name":"DEVELOPMENT_START_REVISION","value":preparation.revision},
                {"name":"DEVELOPMENT_LOCK_SHA256","value":preparation.lock_sha256}],
            "securityContext":worker_security(),
            "resources":{"requests":{"cpu":"250m","memory":"256Mi"},"limits":{"cpu":"1","memory":"1Gi"}},
            "volumeMounts":[{"name":"workspace","mountPath":"/workspace"},
                {"name":"preparations","mountPath":"/preparations"},{"name":"scratch","mountPath":"/tmp"}]});
        serde_json::from_value(json!({"apiVersion":"batch/v1","kind":"Job",
            "metadata":{"name":job_name(&preparation.id),"labels":labels},
            "spec":{"backoffLimit":0,"activeDeadlineSeconds":3600,"ttlSecondsAfterFinished":3600,
                "template":{"metadata":{"labels":labels},"spec":{"restartPolicy":"Never",
                    "automountServiceAccountToken":false,
                    "securityContext":{"seccompProfile":{"type":"RuntimeDefault"},"fsGroup":10001,"fsGroupChangePolicy":"OnRootMismatch"},
                    "initContainers":[checkout,engine],"containers":[preparer],
                    "volumes":[{"name":"workspace","emptyDir":{"sizeLimit":"5Gi"}},
                        {"name":"engine-storage","emptyDir":{"sizeLimit":"20Gi"}},
                        {"name":"preparations","persistentVolumeClaim":{"claimName":"prepared-environments"}},
                        {"name":"scratch","emptyDir":{}}]}}}
        })).map_err(|error| format!("invalid preparation Job: {error}"))
    }
}

fn suffix(id: &str) -> &str {
    id.strip_prefix("prep-")
        .and_then(|value| value.get(..20))
        .unwrap_or("invalid")
}

fn job_name(id: &str) -> String {
    format!("repository-preparation-{}", suffix(id))
}

fn secret_name(id: &str) -> String {
    format!("repository-preparation-{}", suffix(id))
}

fn worker_security() -> Value {
    json!({"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]},
        "readOnlyRootFilesystem":true,"runAsNonRoot":true,"runAsUser":10001})
}
