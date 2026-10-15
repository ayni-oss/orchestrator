//! Opt-in Docker Desktop proof using real resident workers and a disposable local model stub.
use super::*;
use crate::configuration::ExecutionConfiguration;
use ayni_orchestrator_runtime::project_agent::{ProjectAgentRequest, ProjectSessionTurn};
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::{Client, Config, config::KubeConfigOptions};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn env(name: &str) -> TestResult<String> {
    Ok(std::env::var(name)?)
}
fn safe(reason: &'static str) -> io::Error {
    io::Error::other(reason)
}
async fn executor() -> TestResult<RuntimeKubernetes> {
    let namespace = env("AYNI_SESSION_TEST_NAMESPACE")?;
    if !namespace.starts_with("ayni-sessions-") {
        return Err(safe("test namespace must be disposable").into());
    }
    let mut config = Config::from_kubeconfig(&KubeConfigOptions {
        context: Some("docker-desktop".into()),
        ..Default::default()
    })
    .await?;
    config.auth_info.impersonate = Some(format!("system:serviceaccount:{namespace}:control-plane"));
    Ok(RuntimeKubernetes {
        client: Client::try_from(config)?,
        configuration: ExecutionConfiguration {
            namespace,
            worker_image: env("AYNI_SESSION_TEST_IMAGE")?,
            output_claim: "runtime-outputs".into(),
            model_secret: "planner-model".into(),
            allow_emulation: false,
            engine_mode: "local-development".into(),
        },
    })
}
async fn stub(path: &str, post: bool) -> TestResult<Value> {
    let address = env("AYNI_SESSION_TEST_STUB")?;
    let mut stream = tokio::net::TcpStream::connect(&address).await?;
    let method = if post { "POST" } else { "GET" };
    stream.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").as_bytes()).await?;
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut bytes)).await??;
    let body = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| safe("invalid stub response"))?
        + 4;
    Ok(serde_json::from_slice(&bytes[body..])?)
}
async fn ensure(runtime: &RuntimeKubernetes, generation: u64, secret: &str) -> TestResult<String> {
    let secrets: Api<Secret> =
        Api::namespaced(runtime.client.clone(), &runtime.configuration.namespace);
    let metadata = secrets.get_metadata(secret).await?.metadata;
    let uid = metadata.uid.ok_or_else(|| safe("model UID missing"))?;
    let revision = metadata
        .resource_version
        .ok_or_else(|| safe("model revision missing"))?;
    for _ in 0..120 {
        match runtime
            .ensure_project_session(
                "planner-direct",
                "fixture-direct",
                generation,
                secret,
                &uid,
                &revision,
            )
            .await
        {
            Ok(uid) => return Ok(uid),
            Err(
                "project_session_starting"
                | "project_session_transport_unavailable"
                | "project_session_command_unavailable",
            ) => tokio::time::sleep(Duration::from_secs(1)).await,
            Err(reason) => return Err(safe(reason).into()),
        }
    }
    Err(safe("session Pod did not become available").into())
}
fn turn(uid: &str, generation: u64, sequence: u64) -> TestResult<ProjectSessionTurn> {
    Ok(ProjectSessionTurn {
        session_id: "fixture-direct".into(),
        generation,
        turn_sequence: sequence,
        pod_uid: uid.into(),
        deadline_at_unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 180,
        request: ProjectAgentRequest {
            worker_profile: None,
            profile: None,
            run_id: format!("fixture-direct-{generation}-{sequence}"),
            project_id: "fixture-project".into(),
            model: "gpt-5.6-sol".into(),
            history: "Preserve the original fixture requirement.".into(),
            conversation_context: Value::Null,
            repositories: vec![],
            context: json!({"purpose":"research"}).to_string(),
            max_model_calls: 4,
            deadline_seconds: 180,
        },
    })
}
async fn dispatch(runtime: &RuntimeKubernetes, turn: &ProjectSessionTurn) -> TestResult {
    for attempt in 0..120 {
        match runtime
            .dispatch_project_session("planner-direct", turn, BTreeMap::new())
            .await
        {
            Ok(()) => return Ok(()),
            Err(
                reason @ ("project_session_starting"
                | "project_session_transport_unavailable"
                | "project_session_command_unavailable"),
            ) => {
                if attempt % 10 == 0 {
                    eprintln!("dispatch retry {attempt}: {reason}");
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(reason) => return Err(safe(reason).into()),
        }
    }
    Err(safe("session IPC did not become ready").into())
}
async fn completed(runtime: &RuntimeKubernetes, turn: &ProjectSessionTurn) -> TestResult {
    for _ in 0..120 {
        match runtime
            .observe_project_session("planner-direct", turn)
            .await
        {
            Ok(ProjectObservation::Completed(result)) => {
                assert_eq!(result.run_id, turn.request.run_id);
                assert!(result.message.contains("Planner fixture completed turn"));
                return Ok(());
            }
            Ok(ProjectObservation::Missing) => return Err(safe("session Pod disappeared").into()),
            Ok(ProjectObservation::Running)
            | Err(
                "project_session_starting"
                | "project_session_transport_unavailable"
                | "project_session_command_unavailable",
            ) => tokio::time::sleep(Duration::from_secs(1)).await,
            Ok(ProjectObservation::Failed(reason)) | Err(reason) => return Err(safe(reason).into()),
        }
    }
    Err(safe("session turn did not complete").into())
}
async fn remove(runtime: &RuntimeKubernetes, uid: &str) -> TestResult {
    runtime
        .cleanup_project_session("planner-direct", uid)
        .await
        .map_err(safe)?;
    let pods: Api<Pod> = Api::namespaced(runtime.client.clone(), &runtime.configuration.namespace);
    for _ in 0..60 {
        if pods.get_opt("planner-direct").await?.is_none() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(safe("owned session Pod cleanup did not finish").into())
}
async fn duplicate_and_reuse(runtime: &RuntimeKubernetes, uid: &str) -> TestResult {
    let initial = stub("/fixture/stats", false).await?["calls"]
        .as_u64()
        .ok_or_else(|| safe("missing call count"))?;
    let first = turn(uid, 1, 1)?;
    dispatch(runtime, &first).await?;
    completed(runtime, &first).await?;
    dispatch(runtime, &first).await?;
    completed(runtime, &first).await?;
    assert_eq!(stub("/fixture/stats", false).await?["calls"], initial + 1);
    let mut conflicting = first.clone();
    conflicting.request.history = "Changed replay must be rejected".into();
    assert!(
        runtime
            .dispatch_project_session("planner-direct", &conflicting, BTreeMap::new())
            .await
            .is_err()
    );
    let second = turn(uid, 1, 2)?;
    dispatch(runtime, &second).await?;
    completed(runtime, &second).await?;
    assert_eq!(stub("/fixture/stats", false).await?["calls"], initial + 2);
    assert_eq!(ensure(runtime, 1, "planner-model").await?, uid);
    println!(
        "resident worker: two real turns reused one Pod UID; duplicate dispatch made no extra model call"
    );
    Ok(())
}
async fn lost_active_and_rotation(runtime: &RuntimeKubernetes, uid: &str) -> TestResult {
    stub("/fixture/hold", true).await?;
    let active = turn(uid, 1, 3)?;
    dispatch(runtime, &active).await?;
    let mut held = false;
    for _ in 0..60 {
        if stub("/fixture/stats", false).await?["pending"] == 1 {
            held = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(
        held,
        "the actual worker must reach the model before its Pod is lost"
    );
    remove(runtime, uid).await?;
    assert!(matches!(
        runtime
            .observe_project_session("planner-direct", &active)
            .await
            .map_err(safe)?,
        ProjectObservation::Missing
    ));
    stub("/fixture/release", true).await?;
    let replacement = ensure(runtime, 2, "planner-model-rotated").await?;
    assert_ne!(replacement, uid);
    assert!(
        runtime
            .observe_project_session("planner-direct", &active)
            .await
            .is_err()
    );
    assert!(
        runtime
            .dispatch_project_session("planner-direct", &active, BTreeMap::new())
            .await
            .is_err()
    );
    let next = turn(&replacement, 2, 1)?;
    dispatch(runtime, &next).await?;
    completed(runtime, &next).await?;
    remove(runtime, &replacement).await?;
    println!(
        "resident worker: lost active Pod observed, stale UID fenced, rotated model binding used a new generation"
    );
    Ok(())
}
#[tokio::test]
#[ignore = "requires disposable Docker Desktop namespace, synthetic model Secrets and local Responses stub"]
async fn resident_planner_session_on_docker_desktop() -> TestResult {
    let runtime = executor().await?;
    let uid = ensure(&runtime, 1, "planner-model").await?;
    duplicate_and_reuse(&runtime, &uid).await?;
    lost_active_and_rotation(&runtime, &uid).await
}
