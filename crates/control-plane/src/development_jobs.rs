//! Reconcile isolated coding, validation, and PR-author Jobs.

use crate::{
    AppState, PublisherState,
    development::{DevelopmentPhase, DevelopmentTask},
    orchestration::unix_seconds,
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
                warn!(%error, "could not load development tasks");
                return;
            }
        };
        for task in targets {
            if let Err(error) = self.reconcile_task(&task).await {
                warn!(task_id = %task.id, %error, "development task reconciliation failed");
            }
        }
    }

    #[tracing::instrument(
        skip_all,
        fields(
            task_id = %task.id,
            repository_id = %task.repository.id,
            stage = task.phase.as_str(),
            provider = %task.agent_profile.provider,
            model = %task.agent_profile.model,
            preparation_id = task.preparation_id.as_deref().unwrap_or("none")
        )
    )]
    async fn reconcile_task(&self, task: &DevelopmentTask) -> Result<(), String> {
        let deadline_expired = task
            .deadline_at
            .is_some_and(|deadline| unix_seconds().map(|now| now >= deadline).unwrap_or(true));
        if deadline_expired
            && matches!(
                task.phase,
                DevelopmentPhase::Queued
                    | DevelopmentPhase::Coding
                    | DevelopmentPhase::ValidationQueued
                    | DevelopmentPhase::Validating
                    | DevelopmentPhase::PrQueued
                    | DevelopmentPhase::PreparingPr
            )
        {
            self.cleanup_cancelled(task.id).await?;
            self.fail(task.id, "task_deadline_exceeded", "task deadline exceeded")?;
            return Ok(());
        }
        match task.phase {
            DevelopmentPhase::Queued => self.start_stage(task, Stage::Code).await,
            DevelopmentPhase::Coding => self.finish_coding(task).await,
            DevelopmentPhase::ValidationQueued => self.start_stage(task, Stage::Validate).await,
            DevelopmentPhase::Validating => self.finish_validation(task).await,
            DevelopmentPhase::PrQueued => self.start_stage(task, Stage::Pr).await,
            DevelopmentPhase::PreparingPr => self.finish_pr(task).await,
            DevelopmentPhase::Cancelled => self.cleanup_cancelled(task.id).await,
            DevelopmentPhase::WaitingForInput
            | DevelopmentPhase::Completed
            | DevelopmentPhase::Failed
            | DevelopmentPhase::Unsupported => Ok(()),
        }
    }

    async fn start_stage(&self, task: &DevelopmentTask, stage: Stage) -> Result<(), String> {
        if matches!(stage, Stage::Code) {
            let token = match self.publisher.clone_token(&task.repository).await {
                Ok(token) => token,
                Err(error) => {
                    self.fail(
                        task.id,
                        "repository_access_unavailable",
                        &format!("Repository access unavailable: {error}"),
                    )?;
                    return Ok(());
                }
            };
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
                let Ok(candidate) = fs::read_to_string(directory.join("candidate-commit.txt"))
                else {
                    self.fail(
                        task.id,
                        "candidate_artifact_missing",
                        "coding Job completed without a candidate commit",
                    )?;
                    self.delete_clone_secret(task.id).await?;
                    return Ok(());
                };
                if GitRevision::parse(candidate.trim()).is_err() {
                    self.fail(
                        task.id,
                        "candidate_identity_invalid",
                        "coding Job produced an invalid candidate commit",
                    )?;
                    self.delete_clone_secret(task.id).await?;
                    return Ok(());
                }
                if !directory.join("candidate.bundle").is_file() {
                    self.fail(
                        task.id,
                        "candidate_artifact_missing",
                        "coding Job completed without a Git bundle",
                    )?;
                    self.delete_clone_secret(task.id).await?;
                    return Ok(());
                }
                self.with_store(|store| store.record_candidate(task.id, candidate.trim()))?;
                self.delete_clone_secret(task.id).await?;
            }
            JobState::Failed | JobState::Missing => {
                let question = self
                    .publisher
                    .checkpoint_root()
                    .join(task.id.to_string())
                    .join("input-request.txt");
                if let Ok(question) = fs::read_to_string(question) {
                    self.with_store(|store| {
                        store.record_requested_input(task.id, question.trim())
                    })?;
                } else {
                    self.fail(
                        task.id,
                        "coding_job_failed",
                        "coding Job failed or disappeared",
                    )?;
                }
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
                let (digest, tests_edited) = match self.publisher.verify_manifest(
                    task.id,
                    candidate,
                    &task.input.start_revision,
                    &task.repository.lock_sha256,
                    &task.input.change_constraints,
                ) {
                    Ok(verified) => verified,
                    Err(error) => {
                        self.fail(
                            task.id,
                            "validation_evidence_rejected",
                            &format!("Final validation rejected: {error}"),
                        )?;
                        return Ok(());
                    }
                };
                if let Err(error) = self
                    .publisher
                    .verify_candidate_checkpoint(task.id, candidate)
                {
                    self.fail(
                        task.id,
                        "candidate_checkpoint_changed",
                        &format!("Validated candidate changed: {error}"),
                    )?;
                    return Ok(());
                }
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
                    self.fail(
                        task.id,
                        "pr_result_missing",
                        "PR author completed without a durable draft PR",
                    )?;
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
                &task.input.change_constraints,
            )
        });
        if let Some((digest, edited, outcome, scope_violations)) = record {
            self.with_store(|store| {
                store.fail_validation(
                    task.id,
                    &outcome,
                    &outcome,
                    Some(&digest),
                    Some(edited),
                    &scope_violations,
                )
            })?;
        } else {
            self.fail(
                task.id,
                "validation_job_failed",
                "validation Job failed or disappeared",
            )?;
        }
        Ok(())
    }

    async fn recover_publication(&self, task: &DevelopmentTask) -> Result<(), String> {
        let Ok(action) = self.with_store(|store| store.publication_action(task.id)) else {
            return self.fail(
                task.id,
                "publication_not_started",
                "PR author Job failed before publication",
            );
        };
        let candidate = task
            .candidate_commit
            .as_deref()
            .ok_or("missing candidate")?;
        let Ok((digest, _)) = self.publisher.verify_manifest(
            task.id,
            candidate,
            &task.input.start_revision,
            &task.repository.lock_sha256,
            &task.input.change_constraints,
        ) else {
            return self.fail(
                task.id,
                "publication_evidence_changed",
                "publication evidence changed",
            );
        };
        if task.validation_manifest.as_deref() != Some(&digest)
            || action.candidate_commit != candidate
        {
            return self.fail(
                task.id,
                "publication_evidence_changed",
                "publication evidence changed",
            );
        }
        if self
            .publisher
            .verify_candidate_checkpoint(task.id, candidate)
            .is_err()
        {
            return self.fail(
                task.id,
                "publication_checkpoint_changed",
                "publication candidate checkpoint changed",
            );
        }
        let bundle = self
            .publisher
            .checkpoint_root()
            .join(task.id.to_string())
            .join("candidate.bundle");
        let (number, url) = self
            .publisher
            .publish_existing(
                task,
                &bundle,
                &action.branch_name,
                &action.title,
                &action.body,
            )
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

    fn fail(&self, id: TaskId, reason_code: &str, diagnostic: &str) -> Result<(), String> {
        self.with_store(|store| store.fail_development(id, reason_code, diagnostic))?;
        warn!(task_id = %id, reason_code, "development task failed");
        Ok(())
    }

    async fn ensure_clone_secret(&self, id: TaskId, token: &str) -> Result<(), String> {
        let name = secret_name(id);
        let secret: Secret = serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": name, "labels": {"orchestrator.ayni.dev/development-task-id": id.to_string()}},
            "type": "Opaque", "stringData": {"token": token}
        })).map_err(|_| "could not build clone secret")?;
        self.secrets()
            .patch(
                &name,
                &PatchParams::apply("ayni-orchestrator-development").force(),
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

    #[tracing::instrument(
        skip_all,
        fields(task_id = %task.id, stage = stage.name(), job_name = %job_name(task.id, stage))
    )]
    async fn ensure_job(&self, task: &DevelopmentTask, stage: Stage) -> Result<(), String> {
        let name = job_name(task.id, stage);
        let job = self.build_job(task, stage)?;
        match self.jobs().create(&PostParams::default(), &job).await {
            Ok(_) => {
                info!(task_id = %task.id, job_name = name, "development Job created");
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
                if labels.get("orchestrator.ayni.dev/development-task-id")
                    != Some(&task.id.to_string())
                    || labels.get("orchestrator.ayni.dev/development-stage")
                        != Some(&stage.name().to_owned())
                {
                    return Err("deterministic Job name belongs to another task".into());
                }
                Ok(())
            }
        }
    }

    #[tracing::instrument(
        skip_all,
        fields(task_id = %id, stage = stage.name(), job_name = %job_name(id, stage))
    )]
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

    #[tracing::instrument(
        skip_all,
        fields(task_id = %id, stage = stage.name(), job_name = %job_name(id, stage))
    )]
    async fn delete_job(&self, id: TaskId, stage: Stage) -> Result<(), String> {
        let name = job_name(id, stage);
        if let Some(job) = self
            .jobs()
            .get_opt(&name)
            .await
            .map_err(|error| error.to_string())?
        {
            let labels = job.metadata.labels.unwrap_or_default();
            if labels.get("orchestrator.ayni.dev/development-task-id") != Some(&id.to_string()) {
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
        let labels = json!({"app.kubernetes.io/name":"ayni-orchestrator-development-task",
            "orchestrator.ayni.dev/development-task-id": task_id,
            "orchestrator.ayni.dev/development-stage": stage.name()});
        let (init_containers, container, volumes) = match stage {
            Stage::Code => self.code_spec(task)?,
            Stage::Validate => self.validation_spec(task)?,
            Stage::Pr => self.pr_spec(task)?,
        };
        let (active_deadline_seconds, termination_grace_period_seconds) = job_deadline(
            task.deadline_at,
            unix_seconds().map_err(|error| error.to_string())?,
        )?;
        serde_json::from_value(json!({
            "apiVersion":"batch/v1", "kind":"Job",
            "metadata":{"name":name,"labels":labels},
            "spec":{"backoffLimit":0,"activeDeadlineSeconds":active_deadline_seconds,
                "ttlSecondsAfterFinished":3600,
                "template":{"metadata":{"labels":labels},"spec":{
                    "restartPolicy":"Never","automountServiceAccountToken":false,
                    "terminationGracePeriodSeconds":termination_grace_period_seconds,
                    "securityContext":{"seccompProfile":{"type":"RuntimeDefault"},
                        "fsGroup":10001,"fsGroupChangePolicy":"OnRootMismatch"},
                    "initContainers":init_containers,"containers":[container],"volumes":volumes
                }}}
        }))
        .map_err(|error| format!("invalid development Job template: {error}"))
    }

    fn code_spec(&self, task: &DevelopmentTask) -> Result<(Vec<Value>, Value, Vec<Value>), String> {
        let repository = format!(
            "https://github.com/{}/{}.git",
            task.repository.owner, task.repository.repository
        );
        let init = json!({"name":"checkout","image":self.worker_image,"imagePullPolicy":"Never",
            "command":["/bin/sh","-ec"],
            "args":["AUTH=$(printf 'x-access-token:%s' \"$GITHUB_CLONE_TOKEN\" | base64 | tr -d '\\n'); git -c http.https://github.com/.extraheader=\"Authorization: Basic $AUTH\" clone \"$DEVELOPMENT_REPOSITORY_URL\" /workspace/repository; git -C /workspace/repository checkout --detach \"$DEVELOPMENT_START_REVISION\"; test \"$(git -C /workspace/repository rev-parse HEAD)\" = \"$DEVELOPMENT_START_REVISION\"; mkdir -p /workspace/.cargo; cp -R /usr/local/cargo/. /workspace/.cargo/"],
            "env":[{"name":"GITHUB_CLONE_TOKEN","valueFrom":{"secretKeyRef":{"name":secret_name(task.id),"key":"token"}}},
                {"name":"DEVELOPMENT_REPOSITORY_URL","value":repository},
                {"name":"DEVELOPMENT_START_REVISION","value":task.input.start_revision}],
            "securityContext":worker_security(),
            "volumeMounts":[{"name":"workspace","mountPath":"/workspace"}]
        });
        let criteria = serde_json::to_string(&task.input.acceptance_criteria)
            .map_err(|error| error.to_string())?;
        let change_constraints = serde_json::to_string(&task.input.change_constraints)
            .map_err(|error| error.to_string())?;
        let container = json!({"name":"coding-agent","image":self.worker_image,
            "imagePullPolicy":"Never","command":["/usr/local/bin/development-agent"],
            "env":[{"name":"DEVELOPMENT_TASK_ID","value":task.id.to_string()},
                {"name":"DEVELOPMENT_STAGE","value":"coding"},
                {"name":"JOB_NAME","valueFrom":{"fieldRef":{"fieldPath":"metadata.name"}}},
                {"name":"AGENT_PROVIDER","value":task.agent_profile.provider},
                {"name":"DEVELOPMENT_WORKSPACE","value":"/workspace/repository"},
                {"name":"DEVELOPMENT_CHECKPOINT","value":"/checkpoint"},
                {"name":"DEVELOPMENT_START_REVISION","value":task.input.start_revision},
                {"name":"DEVELOPMENT_REQUEST","value":task.input.request},
                {"name":"DEVELOPMENT_ACCEPTANCE_CRITERIA","value":criteria},
                {"name":"DEVELOPMENT_CHANGE_CONSTRAINTS","value":change_constraints},
                {"name":"AGENT_MODEL","value":task.agent_profile.model},
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
                json!({"name":"checkpoint","persistentVolumeClaim":{"claimName":"development-checkpoints"}}),
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
        let preparation_id = task
            .preparation_id
            .as_deref()
            .ok_or("validation has no ready environment preparation")?;
        let change_constraints = serde_json::to_string(&task.input.change_constraints)
            .map_err(|error| error.to_string())?;
        let engine = json!({"name":"engine","image":"docker:27.5.1-dind@sha256:aa3df78ecf320f5fafdce71c659f1629e96e9de0968305fe1de670e0ca9176ce",
            "restartPolicy":"Always","env":[{"name":"DOCKER_TLS_CERTDIR","value":""}],
            "securityContext":{"privileged":true},
            "resources":{"requests":{"cpu":"4","memory":"9Gi"},
                "limits":{"cpu":"4","memory":"9Gi"}},
            "volumeMounts":[{"name":"workspace","mountPath":"/workspace"},
                {"name":"engine-storage","mountPath":"/var/lib/docker"}]});
        let container = json!({"name":"validator","image":self.worker_image,
            "imagePullPolicy":"Never","command":["/usr/local/bin/validate-candidate"],
            "env":[{"name":"DEVELOPMENT_TASK_ID","value":task.id.to_string()},
                {"name":"DEVELOPMENT_STAGE","value":"validation"},
                {"name":"JOB_NAME","valueFrom":{"fieldRef":{"fieldPath":"metadata.name"}}},
                {"name":"DOCKER_HOST","value":"tcp://127.0.0.1:2375"},
                {"name":"DOCKER_BUILDKIT","value":"1"},
                {"name":"DOCKER_CONFIG","value":"/tmp/docker"},
                {"name":"TMPDIR","value":"/workspace"},
                {"name":"DEVELOPMENT_CANDIDATE_BUNDLE","value":"/candidate/candidate.bundle"},
                {"name":"DEVELOPMENT_CANDIDATE_COMMIT","value":candidate},
                {"name":"DEVELOPMENT_START_REVISION","value":task.input.start_revision},
                {"name":"DEVELOPMENT_LOCK_SHA256","value":task.repository.lock_sha256},
                {"name":"DEVELOPMENT_WORKSPACE","value":"/workspace/repository"},
                {"name":"DEVELOPMENT_EVIDENCE_DIR","value":"/evidence"},
                {"name":"DEVELOPMENT_PREPARATION_DIR","value":format!("/preparations/{preparation_id}")},
                {"name":"DEVELOPMENT_PREPARATION_ID","value":preparation_id},
                {"name":"DEVELOPMENT_CHANGE_CONSTRAINTS","value":change_constraints}],
            "securityContext":worker_security(),
            "resources":{"requests":{"cpu":"250m","memory":"256Mi"},
                "limits":{"cpu":"500m","memory":"512Mi"}},
            "volumeMounts":[{"name":"workspace","mountPath":"/workspace"},
                {"name":"candidate","mountPath":"/candidate","subPath":task.id.to_string(),"readOnly":true},
                {"name":"evidence","mountPath":"/evidence","subPath":task.id.to_string()},
                {"name":"preparations","mountPath":"/preparations","readOnly":true},
                {"name":"scratch","mountPath":"/tmp"}]
        });
        Ok((
            vec![engine],
            container,
            vec![
                json!({"name":"workspace","emptyDir":{"sizeLimit":"5Gi"}}),
                json!({"name":"candidate","persistentVolumeClaim":{"claimName":"development-checkpoints","readOnly":true}}),
                json!({"name":"evidence","persistentVolumeClaim":{"claimName":"managed-check-evidence"}}),
                json!({"name":"preparations","persistentVolumeClaim":{"claimName":"prepared-environments","readOnly":true}}),
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
            "Request: {}\nAcceptance criteria: {:?}\nChange constraints: {:?}\nCandidate commit: {}\nValidation manifest: {}\nPolicy or test edits: {}\nChanged paths: {}\nAgent summary: {}",
            task.input.request,
            task.input.acceptance_criteria,
            task.input.change_constraints,
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
            "env":[{"name":"DEVELOPMENT_TASK_ID","value":task.id.to_string()},
                {"name":"DEVELOPMENT_STAGE","value":"pr_preparation"},
                {"name":"JOB_NAME","valueFrom":{"fieldRef":{"fieldPath":"metadata.name"}}},
                {"name":"AGENT_PROVIDER","value":task.agent_profile.provider},
                {"name":"AGENT_MODEL","value":task.agent_profile.model},
                {"name":"DEVELOPMENT_PR_CONTEXT","value":context},
                {"name":"DEVELOPMENT_PUBLISH_ENDPOINT","value":endpoint},
                {"name":"DEVELOPMENT_PUBLISH_TOKEN","valueFrom":{"secretKeyRef":{"name":"development-publisher-auth","key":"token"}}},
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

fn job_deadline(deadline_at: Option<i64>, now: i64) -> Result<(i64, Option<i64>), String> {
    let Some(deadline) = deadline_at else {
        return Ok((3_600, None));
    };
    let remaining = deadline.checked_sub(now).ok_or("task deadline exceeded")?;
    if remaining <= 0 {
        return Err("task deadline exceeded".into());
    }
    Ok((remaining.min(3_600), Some(10)))
}

#[derive(Clone, Copy)]
enum JobState {
    Missing,
    Pending,
    Succeeded,
    Failed,
}

fn job_name(id: TaskId, stage: Stage) -> String {
    format!(
        "development-{}-{}",
        stage.name(),
        id.to_string().replace('-', "")
    )
}

fn secret_name(id: TaskId) -> String {
    format!("development-clone-{}", id.to_string().replace('-', ""))
}

fn worker_security() -> Value {
    json!({"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]},
        "readOnlyRootFilesystem":true,"runAsNonRoot":true,"runAsUser":10001})
}

#[cfg(test)]
mod tests {
    use super::job_deadline;

    #[test]
    fn task_deadline_bounds_each_stage_and_shutdown_grace() {
        assert_eq!(job_deadline(None, 100), Ok((3_600, None)));
        assert_eq!(job_deadline(Some(400), 100), Ok((300, Some(10))));
        assert_eq!(job_deadline(Some(5_000), 100), Ok((3_600, Some(10))));
        assert_eq!(
            job_deadline(Some(100), 100),
            Err("task deadline exceeded".into())
        );
        assert_eq!(
            job_deadline(Some(99), 100),
            Err("task deadline exceeded".into())
        );
    }
}
