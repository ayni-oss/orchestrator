//! Retained M3 evidence shared by broker-outage and process-restart gates.
use super::{Client, Result, Scope, create_scope, next_frame, snapshot};
use ayni_orchestrator_api::v1::{
    ApprovePlanRequest, CreatePlanRequest, GetPlanApprovalRequest, GetPlanVersionRequest,
    GetTaskRequest, ListTasksRequest, PlanContent, PlanTaskSpecification, RevisePlanRequest,
    project_service_client::ProjectServiceClient, watch_conversation_response::Frame,
};
use prost::Message as _;
use serde::{Deserialize, Serialize};
use std::io;
use tonic::{Code, transport::Channel};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct Evidence {
    scope: Scope,
    create: Vec<u8>,
    created: Vec<u8>,
    revise: Vec<u8>,
    revised: Vec<u8>,
    approve: Vec<u8>,
    approved: Vec<u8>,
    cursor: String,
    updates: Vec<Vec<u8>>,
}

pub async fn prepare(projects: &mut ProjectServiceClient<Channel>, client: &mut Client) -> Result {
    let scope = create_scope(projects, client).await?;
    let initial = snapshot(client, &scope).await?;
    let mut watch = client
        .watch_conversation(scope.watch(Some(initial.resume_cursor.clone())))
        .await?
        .into_inner();
    let create = CreatePlanRequest {
        workspace_id: scope.workspace.clone(),
        project_id: scope.project.clone(),
        conversation_id: scope.conversation.clone(),
        idempotency_key: Uuid::now_v7().to_string(),
        content: Some(PlanContent {
            title: "Retained acceptance plan".into(),
            objective: "Retain exact review and admission through outages".into(),
            scope: "M3 acceptance".into(),
            tasks: ["first", "second"]
                .into_iter()
                .map(|key| PlanTaskSpecification {
                    key: key.into(),
                    objective: format!("Retain {key} task"),
                    scope: "Retained origin".into(),
                    acceptance_criteria: vec!["Original identities survive restart".into()],
                    unresolved_inputs: vec!["Verify source commit in M4".into()],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let created = client.create_plan(create.clone()).await?.into_inner();
    let version = created.version.as_ref().ok_or("created version missing")?;
    let mut content = create.content.clone().ok_or("content missing")?;
    content.title = "Reviewed acceptance plan".into();
    let revise = RevisePlanRequest {
        workspace_id: scope.workspace.clone(),
        project_id: scope.project.clone(),
        conversation_id: scope.conversation.clone(),
        idempotency_key: Uuid::now_v7().to_string(),
        plan_id: version.plan_id.clone(),
        expected_version_id: version.id.clone(),
        content: Some(content),
    };
    let revised = client.revise_plan(revise.clone()).await?.into_inner();
    let head = revised.version.as_ref().ok_or("revised version missing")?;
    let approve = ApprovePlanRequest {
        workspace_id: scope.workspace.clone(),
        project_id: scope.project.clone(),
        conversation_id: scope.conversation.clone(),
        idempotency_key: Uuid::now_v7().to_string(),
        plan_id: head.plan_id.clone(),
        expected_version_id: head.id.clone(),
        content_digest: head.content_digest.clone(),
    };
    let approved = client.approve_plan(approve.clone()).await?.into_inner();
    let approval = approved.approval.as_ref().ok_or("approval missing")?;
    let mut updates = Vec::new();
    for revision in initial.conversation_revision + 1..=approval.conversation_revision {
        let Frame::Update(update) = next_frame(&mut watch).await? else {
            return Err("plan watch unexpectedly returned snapshot".into());
        };
        if update.conversation_revision != revision {
            return Err("plan watch skipped or duplicated revision".into());
        }
        updates.push(update.encode_to_vec());
    }
    if updates.len() != 3 {
        return Err("plan writes did not produce exactly three updates".into());
    }
    let evidence = Evidence {
        scope,
        create: create.encode_to_vec(),
        created: created.encode_to_vec(),
        revise: revise.encode_to_vec(),
        revised: revised.encode_to_vec(),
        approve: approve.encode_to_vec(),
        approved: approved.encode_to_vec(),
        cursor: initial.resume_cursor,
        updates,
    };
    verify(client, &evidence).await?;
    println!("{}", serde_json::to_string(&evidence)?);
    Ok(())
}

pub async fn verify_stdin(client: &mut Client) -> Result {
    let evidence: Evidence = serde_json::from_reader(io::stdin().lock())?;
    verify(client, &evidence).await?;
    println!("{{\"plan_admission_recovery\":\"passed\",\"tasks\":2,\"coding_dispatch\":false}}");
    Ok(())
}

async fn verify(client: &mut Client, evidence: &Evidence) -> Result {
    let created = client
        .create_plan(CreatePlanRequest::decode(evidence.create.as_slice())?)
        .await?
        .into_inner();
    let revised = client
        .revise_plan(RevisePlanRequest::decode(evidence.revise.as_slice())?)
        .await?
        .into_inner();
    let request = ApprovePlanRequest::decode(evidence.approve.as_slice())?;
    let approved = client.approve_plan(request.clone()).await?.into_inner();
    if created.encode_to_vec() != evidence.created
        || revised.encode_to_vec() != evidence.revised
        || approved.encode_to_vec() != evidence.approved
    {
        return Err("restart changed exact plan write or approval response".into());
    }
    let receipt = approved.approval.as_ref().ok_or("approval missing")?;
    let retained = client
        .get_plan_approval(GetPlanApprovalRequest {
            workspace_id: request.workspace_id.clone(),
            project_id: request.project_id.clone(),
            conversation_id: request.conversation_id.clone(),
            plan_id: request.plan_id.clone(),
        })
        .await?
        .into_inner();
    if retained.approval != approved.approval || retained.tasks != approved.tasks {
        return Err("approval read lost original receipt or tasks".into());
    }
    verify_tasks(client, evidence, &approved).await?;
    for original in [created.version, revised.version] {
        let version = original.ok_or("version missing")?;
        let retained = client
            .get_plan_version(GetPlanVersionRequest {
                workspace_id: request.workspace_id.clone(),
                project_id: request.project_id.clone(),
                conversation_id: request.conversation_id.clone(),
                plan_id: request.plan_id.clone(),
                version_id: version.id.clone(),
            })
            .await?
            .into_inner()
            .version;
        if retained.as_ref() != Some(&version) {
            return Err("restart changed immutable version".into());
        }
    }
    verify_snapshot(client, evidence, receipt).await?;
    let mut replay = client
        .watch_conversation(evidence.scope.watch(Some(evidence.cursor.clone())))
        .await?
        .into_inner();
    for expected in &evidence.updates {
        let Frame::Update(update) = next_frame(&mut replay).await? else {
            return Err("plan replay returned snapshot".into());
        };
        if update.encode_to_vec() != *expected {
            return Err("restart changed retained plan observation".into());
        }
    }
    let mut competing = request;
    competing.idempotency_key = Uuid::now_v7().to_string();
    let conflict = client
        .approve_plan(competing)
        .await
        .err()
        .ok_or("second approval accepted")?;
    if conflict.code() != Code::AlreadyExists {
        return Err("competing approval returned wrong conflict".into());
    }
    Ok(())
}

async fn verify_tasks(
    client: &mut Client,
    evidence: &Evidence,
    approved: &ayni_orchestrator_api::v1::ApprovePlanResponse,
) -> Result {
    let receipt = approved.approval.as_ref().ok_or("approval missing")?;
    if approved.tasks.len() != 2 || receipt.task_ids.len() != 2 {
        return Err("approval lost complete task set".into());
    }
    let mut ids = Vec::new();
    let mut after_id = String::new();
    loop {
        let page = client
            .list_tasks(ListTasksRequest {
                workspace_id: evidence.scope.workspace.clone(),
                project_id: evidence.scope.project.clone(),
                conversation_id: evidence.scope.conversation.clone(),
                after_id,
                limit: 1,
            })
            .await?
            .into_inner();
        if page.tasks.len() != 1 || ids.len() >= 2 {
            return Err("task pagination lost or duplicated admitted work".into());
        }
        ids.push(page.tasks[0].id.clone());
        after_id = page.next_after_id;
        if after_id.is_empty() {
            break;
        }
    }
    let mut expected_ids = receipt.task_ids.clone();
    expected_ids.sort();
    ids.sort();
    if ids != expected_ids {
        return Err("task listing changed identities".into());
    }
    for (task, id) in approved.tasks.iter().zip(&receipt.task_ids) {
        let retained = client
            .get_task(GetTaskRequest {
                workspace_id: evidence.scope.workspace.clone(),
                project_id: evidence.scope.project.clone(),
                conversation_id: evidence.scope.conversation.clone(),
                task_id: id.clone(),
            })
            .await?
            .into_inner()
            .task;
        if retained.as_ref() != Some(task)
            || task.id != *id
            || task.workspace_id != evidence.scope.workspace
            || task.project_id != evidence.scope.project
            || task.conversation_id != evidence.scope.conversation
            || task.plan_id != receipt.plan_id
            || task.version_id != receipt.version_id
            || task.content_digest != receipt.content_digest
            || task.approval_id != receipt.id
            || task.state != "awaiting_execution"
            || task.execution_blockers.is_empty()
        {
            return Err("task origin, state, blockers or specification changed".into());
        }
    }
    Ok(())
}

async fn verify_snapshot(
    client: &mut Client,
    evidence: &Evidence,
    receipt: &ayni_orchestrator_api::v1::PlanApprovalReceipt,
) -> Result {
    let state = snapshot(client, &evidence.scope).await?;
    if state.conversation_revision != receipt.conversation_revision
        || state.resume_cursor != receipt.resume_cursor
        || state.plans.len() != 1
        || state.plans[0].approval_id != receipt.id
        || !state.nonterminal_turns.is_empty()
        || !state.recent_terminal_turns.is_empty()
        || state
            .history_counts
            .as_ref()
            .is_none_or(|counts| counts.messages != 0 || counts.turns != 0)
    {
        return Err("plan recovery changed snapshot or dispatched conversation work".into());
    }
    Ok(())
}
