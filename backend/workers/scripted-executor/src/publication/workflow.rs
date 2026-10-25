use super::{
    Result,
    config::Configuration,
    git::{self, Prepared},
    github,
};
use ayni_orchestrator_events::task_publication::{
    PublicationOutcome, PublicationRun, PublicationSupervisorRequest, PublicationWorkerRequest,
    PublicationWorkerResponse,
};
use ayni_orchestrator_observability::workflow::{Workflow, finish, identity_context};
use reqwest::Client;
use serde::de::DeserializeOwned;
use tracing::Instrument as _;

async fn authority<T: DeserializeOwned>(
    http: &Client,
    c: &Configuration,
    path: &str,
    request: &(impl serde::Serialize + Sync),
) -> Result<T> {
    let mut request_builder = http
        .post(format!("{}{path}", c.authority.trim_end_matches('/')))
        .bearer_auth(&c.authority_token)
        .header("x-publication-pod-uid", &c.pod_uid)
        .json(request);
    if let Some(context) = ayni_orchestrator_observability::current_context() {
        request_builder = request_builder.header("traceparent", context.traceparent);
    }
    let response = request_builder
        .send()
        .await
        .map_err(|_| "publication authority unavailable")?;
    Ok(
        serde_json::from_slice(&github::bytes(response, 40 * 1024 * 1024).await?)
            .map_err(|_| "publication authority response invalid")?,
    )
}
async fn worker(
    http: &Client,
    c: &Configuration,
    run: &PublicationRun,
    request: &PublicationWorkerRequest,
) -> Result<PublicationWorkerResponse> {
    authority(
        http,
        c,
        &format!(
            "/v1/publication-worker/{}/{}",
            run.command.publication_id, run.generation
        ),
        request,
    )
    .await
}
pub async fn poll(http: &Client, c: &Configuration) -> Result {
    let run: Option<PublicationRun> = authority(
        http,
        c,
        "/v1/publication-supervisor",
        &PublicationSupervisorRequest::Poll {
            workspace_id: c.workspace_id.clone(),
            repository: c.repository.clone(),
            pod_uid: c.pod_uid.clone(),
        },
    )
    .await?;
    if let Some(run) = run {
        let command = &run.command;
        let span = Workflow {
            workspace_id: command.workspace_id.clone(),
            project_id: command.project_id.clone(),
            conversation_id: command.conversation_id.clone(),
            task_id: command.task_id.clone(),
            attempt_id: command.attempt_id.clone(),
            generation: run.generation,
            ..Workflow::default()
        }
        .span_with_context(
            "task.publication.reconcile",
            identity_context(&command.attempt_id).as_ref(),
            consumed(&run),
        );
        let result = execute(http, c, run).instrument(span.clone()).await;
        finish(&span, result.is_ok());
        result?;
    }
    Ok(())
}
async fn report(
    http: &Client,
    c: &Configuration,
    run: &PublicationRun,
    outcome: PublicationOutcome,
) -> Result {
    let response = worker(http, c, run, &PublicationWorkerRequest::Report { outcome }).await?;
    if !response.authorized {
        return Err("publication result not accepted".into());
    }
    Ok(())
}
fn failure(run: &PublicationRun, state: &str, reason: &str) -> PublicationOutcome {
    PublicationOutcome {
        state: state.into(),
        safe_reason: reason.into(),
        commit_sha: run.commit_sha.clone(),
        ..Default::default()
    }
}
async fn execute(http: &Client, c: &Configuration, mut run: PublicationRun) -> Result {
    c.validate(&run.command)?;
    if consumed(&run) {
        return recover(http, c, &run).await;
    }
    let input = worker(http, c, &run, &PublicationWorkerRequest::Input).await?;
    if input.command.as_ref() != Some(&run.command) {
        return Err("publication frozen input conflict".into());
    }
    let prepared = match git::prepare(c, &run.command, &input.source_pack).await {
        Ok(prepared) => prepared,
        Err(error) => {
            let reason = if error.to_string() == "publication_no_changes" {
                "publication_no_changes"
            } else {
                "publication_source_invalid"
            };
            return report(http, c, &run, failure(&run, "blocked", reason)).await;
        }
    };
    let response = worker(
        http,
        c,
        &run,
        &PublicationWorkerRequest::Prepared {
            commit_sha: prepared.commit.clone(),
            tree_sha: prepared.tree.clone(),
        },
    )
    .await?;
    if !response.authorized {
        return Err("publication preparation fenced".into());
    }
    run = response
        .run
        .ok_or("publication retained preparation missing")?;
    if run.commit_sha != prepared.commit || run.tree_sha != prepared.tree {
        return Err("publication commit identity conflict".into());
    }
    if consumed(&run) {
        return recover(http, c, &run).await;
    }
    if !remote_branch(http, c, &run, &prepared).await? {
        return Ok(());
    }
    let existing = match github::reconcile(http, c, &run.command, &prepared.commit).await {
        Ok(existing) => existing,
        Err(error) => {
            if identity_conflict(&error.to_string()) {
                return report(
                    http,
                    c,
                    &run,
                    failure(&run, "blocked", "publication_identity_conflict"),
                )
                .await;
            }
            return Err(error);
        }
    };
    if expired(&run) {
        return report(
            http,
            c,
            &run,
            failure(&run, "blocked", "publication_expired"),
        )
        .await;
    }
    // Even an already observed matching PR consumes the creation permit: this
    // records its identity without authorizing another outbound POST.
    let permit = worker(http, c, &run, &PublicationWorkerRequest::BeginCreate).await?;
    if !permit.authorized {
        return Err("publication create fenced".into());
    }
    let outcome = match existing {
        Some(outcome) => outcome,
        None => github::create(http, c, &run.command, &prepared.commit)
            .await
            .unwrap_or_else(|_| failure(&run, "uncertain", "publication_create_uncertain")),
    };
    report(http, c, &run, outcome).await
}
fn identity_conflict(reason: &str) -> bool {
    matches!(
        reason,
        "publication pull identity conflict"
            | "publication multiple matching pulls"
            | "publication history limit"
    )
}
async fn remote_branch(
    http: &Client,
    c: &Configuration,
    run: &PublicationRun,
    prepared: &Prepared,
) -> Result<bool> {
    let existing = git::head(prepared, c, &run.command.branch).await?;
    if existing.as_deref() == Some(&prepared.commit) {
        return Ok(true);
    }
    if existing.is_some() {
        report(
            http,
            c,
            run,
            failure(run, "blocked", "publication_ref_conflict"),
        )
        .await?;
        return Ok(false);
    }
    if matches!(run.state.as_str(), "creating" | "uncertain") {
        report(
            http,
            c,
            run,
            failure(run, "uncertain", "publication_create_uncertain"),
        )
        .await?;
        return Ok(false);
    }
    if expired(run) {
        report(http, c, run, failure(run, "blocked", "publication_expired")).await?;
        return Ok(false);
    }
    // Refresh application ownership immediately before the create-only Git write.
    let permit = worker(
        http,
        c,
        run,
        &PublicationWorkerRequest::Prepared {
            commit_sha: prepared.commit.clone(),
            tree_sha: prepared.tree.clone(),
        },
    )
    .await?;
    if !permit.authorized {
        return Err("publication push fenced".into());
    }
    // A lost push response is reconciled by its exact immutable remote ref.
    let _result = git::push(prepared, c, &run.command.branch).await;
    if git::head(prepared, c, &run.command.branch)
        .await?
        .as_deref()
        != Some(&prepared.commit)
    {
        return Err("publication push awaits reconciliation".into());
    }
    Ok(true)
}

fn expired(run: &PublicationRun) -> bool {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(true, |elapsed| {
            elapsed.as_secs() >= u64::try_from(run.command.deadline_seconds).unwrap_or(0)
        })
}

fn consumed(run: &PublicationRun) -> bool {
    matches!(run.state.as_str(), "creating" | "uncertain")
}
async fn recover(http: &Client, configuration: &Configuration, run: &PublicationRun) -> Result {
    if !super::config::sha(&run.commit_sha) || !super::config::sha(&run.tree_sha) {
        return Err("publication retained identity invalid".into());
    }
    // Positive remote evidence survives branch deletion and local scratch loss.
    // Consumed mutation authority can only reconcile; it cannot push or create.
    let outcome = github::reconcile(http, configuration, &run.command, &run.commit_sha)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| failure(run, "uncertain", "publication_create_uncertain"));
    report(http, configuration, run, outcome).await
}
