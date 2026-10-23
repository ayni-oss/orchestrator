use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use ayni_orchestrator_control_plane::console::{Console, router};
use futures_util::StreamExt as _;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use std::{error::Error, net::SocketAddr, time::Duration};
use tower::ServiceExt as _;
use uuid::Uuid;

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
async fn call(app: &axum::Router, method: &str, body: Value) -> Result<(StatusCode, Value)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/{method}"))
                .method("POST")
                .header("host", "127.0.0.1:3000")
                .header("origin", "http://127.0.0.1:3000")
                .header("x-console-request", "1")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    let status = response.status();
    let body = response.into_body().collect().await?.to_bytes();
    Ok((status, serde_json::from_slice(&body)?))
}

pub async fn exercise(address: SocketAddr) -> Result {
    let app = router(Console::new("127.0.0.1:3000".parse()?, address)?);
    let command =
        json!({"idempotencyKey":Uuid::now_v7().to_string(),"displayName":"Console integration"});
    let (status, workspace) = call(&app, "CreateWorkspace", command.clone()).await?;
    assert_eq!(status, StatusCode::OK);
    let (_, replay) = call(&app, "CreateWorkspace", command).await?;
    assert_eq!(workspace, replay);
    let workspace_id = &workspace["workspace"]["id"];
    let (_, project) = call(&app, "CreateProject", json!({"idempotencyKey":Uuid::now_v7().to_string(),"workspaceId":workspace_id,"displayName":"Console project"})).await?;
    let project_id = &project["project"]["id"];
    let mut conversations = Vec::new();
    for _ in 0..3 {
        let (status, result) = call(&app, "CreateConversation", json!({"idempotencyKey":Uuid::now_v7().to_string(),"workspaceId":workspace_id,"projectId":project_id,"title":"Browser test"})).await?;
        assert_eq!(status, StatusCode::OK);
        conversations.push(result["conversation"]["id"].clone());
    }
    let (_, projects) = call(&app, "ListProjects", json!({"workspaceId":workspace_id})).await?;
    assert_eq!(projects["projects"][0]["id"], *project_id);
    let (_, page) = call(
        &app,
        "ListConversations",
        json!({"workspaceId":workspace_id,"projectId":project_id,"limit":2}),
    )
    .await?;
    assert_eq!(
        page["conversations"]
            .as_array()
            .ok_or("missing page")?
            .len(),
        2
    );
    let (_, next) = call(&app, "ListConversations", json!({"workspaceId":workspace_id,"projectId":project_id,"limit":2,"afterId":page["nextAfterId"]})).await?;
    assert_eq!(
        next["conversations"]
            .as_array()
            .ok_or("missing page")?
            .len(),
        1
    );
    assert_ne!(
        page["conversations"][0]["id"],
        next["conversations"][0]["id"]
    );
    let (status, _) = call(
        &app,
        "ListConversations",
        json!({"workspaceId":Uuid::now_v7().to_string(),"projectId":project_id}),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "ListWorkspaces", json!({"limit":101})).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&app, "ListProjects", json!({"workspaceId":"invalid"})).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let scope = json!({"workspaceId":workspace_id,"projectId":project_id,"conversationId":conversations[0]});
    plan_review(&app, &scope).await?;
    replay_watch(&app, scope).await
}

async fn plan_review(app: &axum::Router, scope: &Value) -> Result {
    let mut command = scope.clone();
    command["idempotencyKey"] = json!(Uuid::now_v7().to_string());
    command["content"] =
        json!({"title":"Review plan","objective":"Retain origin","scope":"Console"});
    let (status, created) = call(app, "CreatePlan", command.clone()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(call(app, "CreatePlan", command).await?.1, created);
    let mut read = scope.clone();
    read["planId"] = created["version"]["planId"].clone();
    let (status, current) = call(app, "GetPlanVersion", read.clone()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current["version"], created["version"]);
    let mut revise = read.clone();
    revise["idempotencyKey"] = json!(Uuid::now_v7().to_string());
    revise["expectedVersionId"] = created["version"]["id"].clone();
    revise["content"] = json!({"title":"Revised plan","objective":"Retain origin","scope":"Console",
            "tasks":[{"key":"console","objective":"Show admitted work","scope":"Console",
                "acceptanceCriteria":["Origin is visible"],"unresolvedInputs":["Select runtime"]}]});
    let (status, updated) = call(app, "RevisePlan", revise.clone()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["version"]["ordinal"], "2");
    revise["idempotencyKey"] = json!(Uuid::now_v7().to_string());
    let (status, error) = call(app, "RevisePlan", revise).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["reason"], "ERROR_REASON_PLAN_VERSION_CONFLICT");
    let (_, history) = call(app, "ListPlanVersions", read.clone()).await?;
    assert_eq!(history["versions"].as_array().ok_or("history")?.len(), 2);
    read["versionId"] = created["version"]["id"].clone();
    assert_eq!(
        call(app, "GetPlanVersion", read).await?.1["version"],
        created["version"]
    );
    let (_, plans) = call(app, "ListPlans", scope.clone()).await?;
    assert_eq!(
        plans["plans"][0]["currentVersionId"],
        updated["version"]["id"]
    );
    let (_, snapshot) = call(app, "GetConversationSnapshot", scope.clone()).await?;
    assert_eq!(snapshot["snapshot"]["plans"], plans["plans"]);
    admission(app, scope, &updated["version"]).await
}

async fn admission(app: &axum::Router, scope: &Value, version: &Value) -> Result {
    let mut read = scope.clone();
    read["planId"] = version["planId"].clone();
    let (status, before) = call(app, "GetPlanApproval", read.clone()).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(before.get("approval").is_none());
    let mut approve = read.clone();
    approve["idempotencyKey"] = json!(Uuid::now_v7().to_string());
    approve["expectedVersionId"] = version["id"].clone();
    approve["contentDigest"] = json!(format!("sha256:{}", "0".repeat(64)));
    let (status, invalid) = call(app, "ApprovePlan", approve.clone()).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(invalid["reason"], "ERROR_REASON_PLAN_DIGEST_MISMATCH");
    approve["contentDigest"] = version["contentDigest"].clone();
    let mut spoofed = approve.clone();
    spoofed["operator"] = json!("model");
    assert_eq!(
        call(app, "ApprovePlan", spoofed).await?.0,
        StatusCode::BAD_REQUEST
    );
    let (status, admitted) = call(app, "ApprovePlan", approve.clone()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(call(app, "ApprovePlan", approve.clone()).await?.1, admitted);
    assert_eq!(call(app, "GetPlanApproval", read).await?.1, admitted);
    assert_eq!(admitted["approval"]["versionId"], version["id"]);
    assert_eq!(
        admitted["approval"]["contentDigest"],
        version["contentDigest"]
    );
    let task = &admitted["tasks"][0];
    assert_eq!(task["state"], "awaiting_execution");
    assert_eq!(task["conversationId"], scope["conversationId"]);
    assert_eq!(task["approvalId"], admitted["approval"]["id"]);
    assert_eq!(task["specification"], version["content"]["tasks"][0]);
    assert!(
        !task["executionBlockers"]
            .as_array()
            .ok_or("blockers")?
            .is_empty()
    );
    let mut task_read = scope.clone();
    task_read["taskId"] = task["id"].clone();
    assert_eq!(
        call(app, "GetTask", task_read.clone()).await?.1["task"],
        *task
    );
    assert_eq!(
        call(app, "GetTaskExecution", task_read.clone()).await?.0,
        StatusCode::OK
    );
    assert_eq!(
        call(app, "PrepareTaskExecution", task_read.clone())
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    task_read["conversationId"] = json!(Uuid::now_v7().to_string());
    assert_eq!(
        call(app, "GetTask", task_read).await?.0,
        StatusCode::NOT_FOUND
    );
    let (_, tasks) = call(app, "ListTasks", scope.clone()).await?;
    assert_eq!(tasks["tasks"], admitted["tasks"]);
    let (_, snapshot) = call(app, "GetConversationSnapshot", scope.clone()).await?;
    assert_eq!(
        snapshot["snapshot"]["plans"][0]["approvalId"],
        admitted["approval"]["id"]
    );
    approve["idempotencyKey"] = json!(Uuid::now_v7().to_string());
    let (status, conflict) = call(app, "ApprovePlan", approve).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["reason"], "ERROR_REASON_PLAN_ALREADY_APPROVED");
    let mut revise = scope.clone();
    revise["planId"] = version["planId"].clone();
    revise["idempotencyKey"] = json!(Uuid::now_v7().to_string());
    revise["expectedVersionId"] = version["id"].clone();
    revise["content"] = version["content"].clone();
    let (status, conflict) = call(app, "RevisePlan", revise).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["reason"], "ERROR_REASON_PLAN_ALREADY_APPROVED");
    Ok(())
}

async fn replay_watch(app: &axum::Router, scope: Value) -> Result {
    let workspace_id = &scope["workspaceId"];
    let project_id = &scope["projectId"];
    let conversation_id = &scope["conversationId"];
    let (_, before) = call(app, "GetConversationSnapshot", scope.clone()).await?;
    let mut command = scope.clone();
    command["idempotencyKey"] = json!(Uuid::now_v7().to_string());
    command["content"] = json!("Console message");
    let (status, admitted) = call(app, "SendMessage", command.clone()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(admitted, call(app, "SendMessage", command).await?.1);
    let query = format!(
        "/watch?workspaceId={}&projectId={}&conversationId={}&afterCursor={}",
        workspace_id.as_str().ok_or("workspace")?,
        project_id.as_str().ok_or("project")?,
        conversation_id.as_str().ok_or("conversation")?,
        "invalid-query-cursor"
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(query)
                .header(
                    "last-event-id",
                    before["snapshot"]["resumeCursor"]
                        .as_str()
                        .ok_or("cursor")?,
                )
                .header("host", "127.0.0.1:3000")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut stream = response.into_body().into_data_stream();
    let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await?
        .ok_or("missing frame")??;
    let frame = std::str::from_utf8(&frame)?;
    assert!(frame.contains("event: frame"));
    assert!(frame.contains("id:"));
    assert!(frame.contains("update"));
    drop(stream);
    let (_, after) = call(app, "GetConversationSnapshot", scope).await?;
    assert_eq!(
        after["snapshot"]["recentMessages"]
            .as_array()
            .ok_or("messages")?
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn publication_commands_remain_outside_the_browser_mutation_boundary() -> Result {
    let app = router(Console::new(
        "127.0.0.1:3000".parse()?,
        "127.0.0.1:50051".parse()?,
    )?);
    for method in ["StartTaskPublication", "CancelTaskPublication"] {
        assert_eq!(
            call(&app, method, json!({})).await?.0,
            StatusCode::NOT_FOUND
        );
    }
    Ok(())
}
