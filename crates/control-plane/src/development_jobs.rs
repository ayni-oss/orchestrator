//! Reconcile M3's isolated coding, validation, and PR-author Jobs.

use crate::{
    AppState, PublisherState,
    development::{DevelopmentPhase, DevelopmentTask},
};
use ayni_orchestrator_domain::{GitRevision, TaskId};
use k8s_openapi::api::{batch::v1::Job, core::v1::Secret};
use kube::{
    Api, Client,
    api::{DeleteParams, Patch, PatchParams, PostParams},
};
use serde_json::{Value, json};
use std::{fs, sync::Arc};
use tracing::{info, warn};

#[derive(Clone)]
pub struct DevelopmentReconciler {
    state: AppState,
    client: Client,
    namespace: String,
    worker_image: String,
    publisher: Arc<PublisherState>,
}

#[derive(Clone, Copy)]
enum Stage {
    Code,
    Validate,
    Pr,
}

impl Stage {
    const fn name(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Validate => "validate",
            Self::Pr => "pr",
        }
    }
}

impl DevelopmentReconciler {
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

    fn jobs(&self) -> Api<Job> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }
    fn secrets(&self) -> Api<Secret> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }

    pub async fn reconcile_once(&self) {
        let targets = match self.with_store(|store| store.development_targets()) {
            Ok(targets) => targets,
            Err(error) => {
                warn!(%error, "could not load M3 tasks");
                return;
            }
        };
        for task in targets {
            if let Err(error) = self.reconcile_task(&task).await {
                warn!(task_id = %task.id, %error, "M3 task reconciliation failed");
            }
        }
    }

    async fn reconcile_task(&self, task: &DevelopmentTask) -> Result<(), String> {
        match task.phase {
            DevelopmentPhase::Queued => self.start_stage(task, Stage::Code).await,
            DevelopmentPhase::Coding => self.finish_coding(task).await,
            DevelopmentPhase::ValidationQueued => self.start_stage(task, Stage::Validate).await,
            DevelopmentPhase::Validating => self.finish_validation(task).await,
            DevelopmentPhase::PrQueued => self.start_stage(task, Stage::Pr).await,
            DevelopmentPhase::PreparingPr => self.finish_pr(task).await,
            DevelopmentPhase::Cancelled => self.cleanup_cancelled(task.id).await,
            DevelopmentPhase::Completed
            | DevelopmentPhase::Failed
            | DevelopmentPhase::Unsupported => Ok(()),
        }
    }

    async fn start_stage(&self, task: &DevelopmentTask, stage: Stage) -> Result<(), String> {
        if matches!(stage, Stage::Code) {
            let token = self
                .publisher
                .clone_token(task.repository.installation_id)
                .await?;
            self.ensure_clone_secret(task.id, &token).await?;
        }
        self.ensure_job(task, stage).await?;
        let (from, to) = match stage {
            Stage::Code => (DevelopmentPhase::Queued, DevelopmentPhase::Coding),
            Stage::Validate => (
                DevelopmentPhase::ValidationQueued,
                DevelopmentPhase::Validating,
            ),
            Stage::Pr => (DevelopmentPhase::PrQueued, DevelopmentPhase::PreparingPr),
        };
        self.set_phase(task.id, from, to)
    }

    async fn finish_coding(&self, task: &DevelopmentTask) -> Result<(), String> {
        match self.observe(task.id, Stage::Code).await? {
            JobState::Succeeded => {
                let directory = self.publisher.checkpoint_root().join(task.id.to_string());
                let candidate = fs::read_to_string(directory.join("candidate-commit.txt"))
                    .map_err(|_| "coding Job completed without a candidate commit")?;
                GitRevision::parse(candidate.trim()).map_err(|_| "candidate commit is invalid")?;
                if !directory.join("candidate.bundle").is_file() {
                    return self.fail(task.id, "coding Job completed without a Git bundle");
                }
                self.with_store(|store| store.record_candidate(task.id, candidate.trim()))?;
                self.delete_clone_secret(task.id).await?;
            }
            JobState::Failed | JobState::Missing => {
                self.fail(task.id, "coding Job failed or disappeared")?;
                self.delete_clone_secret(task.id).await?;
            }
            JobState::Pending => {}
        }
        Ok(())
    }

    async fn finish_validation(&self, task: &DevelopmentTask) -> Result<(), String> {
        match self.observe(task.id, Stage::Validate).await? {
            JobState::Succeeded => {
                let candidate = task
                    .candidate_commit
                    .as_deref()
                    .ok_or("missing candidate")?;
                let (digest, tests_edited) = self.publisher.verify_manifest(
                    task.id,
                    candidate,
                    &task.input.start_revision,
                    &task.repository.lock_sha256,
                )?;
                self.with_store(|store| store.record_validation(task.id, &digest, tests_edited))?;
            }
            JobState::Failed | JobState::Missing => self.record_failed_validation(task)?,
            JobState::Pending => {}
        }
        Ok(())
    }

    async fn finish_pr(&self, task: &DevelopmentTask) -> Result<(), String> {
        match self.observe(task.id, Stage::Pr).await? {
            JobState::Failed | JobState::Missing => self.recover_publication(task).await?,
            JobState::Succeeded => {
                let refreshed = self.with_store(|store| store.development_task(task.id))?;
                if refreshed.phase != DevelopmentPhase::Completed {
                    self.fail(task.id, "PR author completed without a durable draft PR")?;
                }
            }
            JobState::Pending => {}
        }
        Ok(())
    }

    async fn cleanup_cancelled(&self, id: TaskId) -> Result<(), String> {
        for stage in [Stage::Code, Stage::Validate, Stage::Pr] {
            self.delete_job(id, stage).await?;
        }
        self.delete_clone_secret(id).await
    }

    fn record_failed_validation(&self, task: &DevelopmentTask) -> Result<(), String> {
        let record = task.candidate_commit.as_deref().and_then(|candidate| {
            self.publisher.failed_validation_record(
                task.id,
                candidate,
                &task.input.start_revision,
                &task.repository.lock_sha256,
            )
        });
        if let Some((digest, edited, outcome)) = record {
            self.with_store(|store| {
                store.fail_validation(task.id, &outcome, Some(&digest), Some(edited))
            })?;
        } else {
            self.fail(task.id, "validation Job failed or disappeared")?;
        }
        Ok(())
    }

    async fn recover_publication(&self, task: &DevelopmentTask) -> Result<(), String> {
        let Ok(action) = self.with_store(|store| store.publication_action(task.id)) else {
            return self.fail(task.id, "PR author Job failed before publication");
        };
        let candidate = task
            .candidate_commit
            .as_deref()
            .ok_or("missing candidate")?;
        let (digest, _) = self.publisher.verify_manifest(
            task.id,
            candidate,
            &task.input.start_revision,
            &task.repository.lock_sha256,
        )?;
        if task.validation_manifest.as_deref() != Some(&digest)
            || action.candidate_commit != candidate
        {
            return self.fail(task.id, "publication evidence changed");
        }
        let bundle = self
            .publisher
            .checkpoint_root()
            .join(task.id.to_string())
            .join("candidate.bundle");
        let (number, url) = self
            .publisher
            .publish_existing(task, &bundle, &action.title, &action.body)
            .await?;
        self.with_store(|store| store.complete_publication(task.id, candidate, number, &url))
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

    fn set_phase(
        &self,
        id: TaskId,
        from: DevelopmentPhase,
        to: DevelopmentPhase,
    ) -> Result<(), String> {
        self.with_store(|store| store.development_phase(id, from, to))?;
        Ok(())
    }

    fn fail(&self, id: TaskId, reason: &str) -> Result<(), String> {
        self.with_store(|store| store.fail_development(id, reason))?;
        warn!(task_id = %id, reason, "M3 task failed");
        Ok(())
    }

    async fn ensure_clone_secret(&self, id: TaskId, token: &str) -> Result<(), String> {
        let name = secret_name(id);
        let secret: Secret = serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": name, "labels": {"orchestrator.ayni.dev/m3-task-id": id.to_string()}},
            "type": "Opaque", "stringData": {"token": token}
        })).map_err(|_| "could not build clone secret")?;
        self.secrets()
            .patch(
                &name,
                &PatchParams::apply("ayni-orchestrator-m3").force(),
                &Patch::Apply(&secret),
            )
            .await
            .map_err(|error| format!("clone secret update failed: {error}"))?;
        Ok(())
    }

    async fn delete_clone_secret(&self, id: TaskId) -> Result<(), String> {
        let name = secret_name(id);
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
                .map_err(|error| format!("clone secret deletion failed: {error}"))?;
        }
        Ok(())
    }

    async fn ensure_job(&self, task: &DevelopmentTask, stage: Stage) -> Result<(), String> {
        let name = job_name(task.id, stage);
        let job = self.build_job(task, stage)?;
        match self.jobs().create(&PostParams::default(), &job).await {
            Ok(_) => {
                info!(task_id = %task.id, job_name = name, "M3 Job created");
                Ok(())
            }
            Err(error) => {
                let existing = self
                    .jobs()
                    .get_opt(&name)
                    .await
                    .map_err(|lookup| {
                        format!("Job create failed: {error}; lookup failed: {lookup}")
                    })?
                    .ok_or_else(|| format!("Job create failed: {error}"))?;
                let labels = existing.metadata.labels.unwrap_or_default();
                if labels.get("orchestrator.ayni.dev/m3-task-id") != Some(&task.id.to_string())
                    || labels.get("orchestrator.ayni.dev/m3-stage")
                        != Some(&stage.name().to_owned())
                {
                    return Err("deterministic Job name belongs to another task".into());
                }
                Ok(())
            }
        }
    }

    async fn observe(&self, id: TaskId, stage: Stage) -> Result<JobState, String> {
        let Some(job) = self
            .jobs()
            .get_opt(&job_name(id, stage))
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(JobState::Missing);
        };
        let status = job.status;
        if status
            .as_ref()
            .and_then(|status| status.succeeded)
            .unwrap_or(0)
            > 0
        {
            Ok(JobState::Succeeded)
        } else if status
            .as_ref()
            .and_then(|status| status.failed)
            .unwrap_or(0)
            > 0
        {
            Ok(JobState::Failed)
        } else {
            Ok(JobState::Pending)
        }
    }

    async fn delete_job(&self, id: TaskId, stage: Stage) -> Result<(), String> {
        let name = job_name(id, stage);
        if let Some(job) = self
            .jobs()
            .get_opt(&name)
            .await
            .map_err(|error| error.to_string())?
        {
            let labels = job.metadata.labels.unwrap_or_default();
            if labels.get("orchestrator.ayni.dev/m3-task-id") != Some(&id.to_string()) {
                return Err("refusing to delete unowned Job".into());
            }
            self.jobs()
                .delete(&name, &DeleteParams::default())
                .await
                .map_err(|error| format!("Job deletion failed: {error}"))?;
        }
        Ok(())
    }

    fn build_job(&self, task: &DevelopmentTask, stage: Stage) -> Result<Job, String> {
        let name = job_name(task.id, stage);
        let task_id = task.id.to_string();
        let labels = json!({"app.kubernetes.io/name":"ayni-orchestrator-m3",
            "orchestrator.ayni.dev/m3-task-id": task_id,
            "orchestrator.ayni.dev/m3-stage": stage.name()});
        let (init_containers, container, volumes) = match stage {
            Stage::Code => self.code_spec(task)?,
            Stage::Validate => self.validation_spec(task)?,
            Stage::Pr => self.pr_spec(task)?,
        };
        serde_json::from_value(json!({
            "apiVersion":"batch/v1", "kind":"Job",
            "metadata":{"name":name,"labels":labels},
            "spec":{"backoffLimit":0,"activeDeadlineSeconds":3600,"ttlSecondsAfterFinished":3600,
                "template":{"metadata":{"labels":labels},"spec":{
                    "restartPolicy":"Never","automountServiceAccountToken":false,
                    "securityContext":{"seccompProfile":{"type":"RuntimeDefault"},
                        "fsGroup":10001,"fsGroupChangePolicy":"OnRootMismatch"},
                    "initContainers":init_containers,"containers":[container],"volumes":volumes
                }}}
        }))
        .map_err(|error| format!("invalid M3 Job template: {error}"))
    }

    fn code_spec(&self, task: &DevelopmentTask) -> Result<(Vec<Value>, Value, Vec<Value>), String> {
        let repository = format!(
            "https://github.com/{}/{}.git",
            task.repository.owner, task.repository.repository
        );
        let init = json!({"name":"checkout","image":self.worker_image,"imagePullPolicy":"Never",
            "command":["/bin/sh","-ec"],
            "args":["AUTH=$(printf 'x-access-token:%s' \"$M3_CLONE_TOKEN\" | base64 | tr -d '\\n'); git -c http.https://github.com/.extraheader=\"Authorization: Basic $AUTH\" clone \"$M3_REPOSITORY_URL\" /workspace/repository; git -C /workspace/repository checkout --detach \"$M3_START_REVISION\"; test \"$(git -C /workspace/repository rev-parse HEAD)\" = \"$M3_START_REVISION\"; mkdir -p /workspace/.cargo; cp -R /usr/local/cargo/. /workspace/.cargo/"],
            "env":[{"name":"M3_CLONE_TOKEN","valueFrom":{"secretKeyRef":{"name":secret_name(task.id),"key":"token"}}},
                {"name":"M3_REPOSITORY_URL","value":repository},
                {"name":"M3_START_REVISION","value":task.input.start_revision}],
            "securityContext":worker_security(),
            "volumeMounts":[{"name":"workspace","mountPath":"/workspace"}]
        });
        let criteria = serde_json::to_string(&task.input.acceptance_criteria)
            .map_err(|error| error.to_string())?;
        let container = json!({"name":"coding-agent","image":self.worker_image,
            "imagePullPolicy":"Never","command":["/usr/local/bin/development-agent"],
            "env":[{"name":"M3_WORKSPACE","value":"/workspace/repository"},
                {"name":"M3_CHECKPOINT","value":"/checkpoint"},
                {"name":"M3_START_REVISION","value":task.input.start_revision},
                {"name":"M3_REQUEST","value":task.input.request},
                {"name":"M3_ACCEPTANCE_CRITERIA","value":criteria},
                {"name":"M3_MODEL","value":task.agent_profile.model},
                {"name":"CARGO_HOME","value":"/workspace/.cargo"},
                {"name":"OPENAI_API_KEY","valueFrom":{"secretKeyRef":{"name":task.agent_profile.secret_ref,"key":"api-key"}}}],
            "securityContext":worker_security(),
            "resources":{"requests":{"cpu":"500m","memory":"1Gi"},
                "limits":{"cpu":"2","memory":"3Gi"}},
            "volumeMounts":[{"name":"workspace","mountPath":"/workspace"},
                {"name":"checkpoint","mountPath":"/checkpoint","subPath":task.id.to_string()},
                {"name":"scratch","mountPath":"/tmp"}]
        });
        Ok((
            vec![init],
            container,
            vec![
                json!({"name":"workspace","emptyDir":{"sizeLimit":"5Gi"}}),
                json!({"name":"checkpoint","persistentVolumeClaim":{"claimName":"m3-checkpoints"}}),
                json!({"name":"scratch","emptyDir":{}}),
            ],
        ))
    }

    fn validation_spec(
        &self,
        task: &DevelopmentTask,
    ) -> Result<(Vec<Value>, Value, Vec<Value>), String> {
        let candidate = task
            .candidate_commit
            .as_deref()
            .ok_or("validation has no candidate")?;
        let engine = json!({"name":"engine","image":"docker:27.5.1-dind@sha256:aa3df78ecf320f5fafdce71c659f1629e96e9de0968305fe1de670e0ca9176ce",
            "restartPolicy":"Always","env":[{"name":"DOCKER_TLS_CERTDIR","value":""}],
            "securityContext":{"privileged":true},
            "resources":{"requests":{"cpu":"4","memory":"9Gi"},
                "limits":{"cpu":"4","memory":"9Gi"}},
            "volumeMounts":[{"name":"workspace","mountPath":"/workspace"},
                {"name":"engine-storage","mountPath":"/var/lib/docker"}]});
        let container = json!({"name":"validator","image":self.worker_image,
            "imagePullPolicy":"Never","command":["/usr/local/bin/validate-candidate"],
            "env":[{"name":"DOCKER_HOST","value":"tcp://127.0.0.1:2375"},
                {"name":"DOCKER_BUILDKIT","value":"1"},
                {"name":"DOCKER_CONFIG","value":"/tmp/docker"},
                {"name":"TMPDIR","value":"/workspace"},
                {"name":"M3_CANDIDATE_BUNDLE","value":"/candidate/candidate.bundle"},
                {"name":"M3_CANDIDATE_COMMIT","value":candidate},
                {"name":"M3_START_REVISION","value":task.input.start_revision},
                {"name":"M3_LOCK_SHA256","value":task.repository.lock_sha256},
                {"name":"M3_WORKSPACE","value":"/workspace/repository"},
                {"name":"M3_EVIDENCE_DIR","value":"/evidence"}],
            "securityContext":worker_security(),
            "resources":{"requests":{"cpu":"250m","memory":"256Mi"},
                "limits":{"cpu":"500m","memory":"512Mi"}},
            "volumeMounts":[{"name":"workspace","mountPath":"/workspace"},
                {"name":"candidate","mountPath":"/candidate","subPath":task.id.to_string(),"readOnly":true},
                {"name":"evidence","mountPath":"/evidence","subPath":task.id.to_string()},
                {"name":"scratch","mountPath":"/tmp"}]
        });
        Ok((
            vec![engine],
            container,
            vec![
                json!({"name":"workspace","emptyDir":{"sizeLimit":"5Gi"}}),
                json!({"name":"candidate","persistentVolumeClaim":{"claimName":"m3-checkpoints","readOnly":true}}),
                json!({"name":"evidence","persistentVolumeClaim":{"claimName":"managed-check-evidence"}}),
                json!({"name":"scratch","emptyDir":{}}),
                json!({"name":"engine-storage","emptyDir":{"sizeLimit":"20Gi"}}),
            ],
        ))
    }

    fn pr_spec(&self, task: &DevelopmentTask) -> Result<(Vec<Value>, Value, Vec<Value>), String> {
        let candidate = task
            .candidate_commit
            .as_deref()
            .ok_or("PR author has no candidate")?;
        let context = format!(
            "Request: {}\nAcceptance criteria: {:?}\nCandidate commit: {}\nValidation manifest: {}\nPolicy or test edits: {}\nChanged paths: {}\nAgent summary: {}",
            task.input.request,
            task.input.acceptance_criteria,
            candidate,
            task.validation_manifest.as_deref().unwrap_or("none"),
            task.policy_or_test_edits.unwrap_or(false),
            fs::read_to_string(
                self.publisher
                    .evidence_root()
                    .join(task.id.to_string())
                    .join("git-diff.stdout")
            )
            .unwrap_or_default(),
            fs::read_to_string(
                self.publisher
                    .checkpoint_root()
                    .join(task.id.to_string())
                    .join("agent-summary.txt")
            )
            .unwrap_or_default()
        );
        let endpoint = format!(
            "http://control-plane.{}.svc.cluster.local:8080/v1/development-tasks/{}/publish",
            self.namespace, task.id
        );
        let container = json!({"name":"pr-agent","image":self.worker_image,
            "imagePullPolicy":"Never","command":["/usr/local/bin/pr-agent"],
            "env":[{"name":"M3_MODEL","value":task.agent_profile.model},
                {"name":"M3_PR_CONTEXT","value":context},
                {"name":"M3_PUBLISH_ENDPOINT","value":endpoint},
                {"name":"M3_PUBLISH_TOKEN","valueFrom":{"secretKeyRef":{"name":"m3-publisher","key":"token"}}},
                {"name":"OPENAI_API_KEY","valueFrom":{"secretKeyRef":{"name":task.agent_profile.secret_ref,"key":"api-key"}}}],
            "securityContext":worker_security(),
            "resources":{"requests":{"cpu":"250m","memory":"256Mi"},
                "limits":{"cpu":"1","memory":"1Gi"}},
            "volumeMounts":[{"name":"scratch","mountPath":"/tmp"}]
        });
        Ok((
            vec![],
            container,
            vec![json!({"name":"scratch","emptyDir":{}})],
        ))
    }
}

#[derive(Clone, Copy)]
enum JobState {
    Missing,
    Pending,
    Succeeded,
    Failed,
}

fn job_name(id: TaskId, stage: Stage) -> String {
    format!("m3-{}-{}", stage.name(), id.to_string().replace('-', ""))
}

fn secret_name(id: TaskId) -> String {
    format!("m3-clone-{}", id.to_string().replace('-', ""))
}

fn worker_security() -> Value {
    json!({"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]},
        "readOnlyRootFilesystem":true,"runAsNonRoot":true,"runAsUser":10001})
}
