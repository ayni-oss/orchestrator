use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::{collections::BTreeMap, fs};

pub fn check() -> Result<()> {
    crate::command::run("buf", &["format", "--diff", "--exit-code", "proto"])?;
    crate::command::run("buf", &["lint", "proto"])?;
    ensure!(
        fs::read_to_string("proto/BASELINE")?.trim() == "workflow-m1-private-v1",
        "unexpected protocol baseline"
    );
    let package = Regex::new(r"(?m)^\s*package\s+([\w.]+)\s*;")?;
    let version = Regex::new(r"^v[0-9]+$")?;
    let service = Regex::new(r"(?s)service\s+(\w+)\s*\{(.*?)\n\}")?;
    let rpc = Regex::new(r"\brpc\s+(\w+)\s*\(")?;
    let expected: BTreeMap<String, BTreeMap<String, Vec<String>>> =
        serde_json::from_str(include_str!("../../contracts/api-services.json"))?;
    let legacy = Regex::new(
        "GetWorkspace|UpdateWorkspace|ArchiveWorkspace|WatchProjects|ListProjectSummaries|GetProject|UpdateProject|ArchiveProject|UpdateProjectConversationSettings|CancelProjectAgentRun|GetProjectConversationContext|ListProjectConversationHistory|WatchProjectConversation|CreateProjectConversation|ListProjectConversations|GetProjectConversation|SendProjectMessage|DecideProjectPlan|DecideProjectProposal",
    )?;
    for path in crate::command::files()? {
        if !std::path::Path::new(&path).is_file() {
            continue;
        }
        if path.starts_with("proto/ayni/")
            && std::path::Path::new(&path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("proto"))
        {
            let source = fs::read_to_string(&path)?;
            let packages: Vec<_> = package
                .captures_iter(&source)
                .map(|c| c[1].to_owned())
                .collect();
            ensure!(
                packages.len() == 1 && packages[0].starts_with("ayni."),
                "unexpected package in {path}"
            );
            ensure!(
                packages[0]
                    .split('.')
                    .filter(|part| version.is_match(part))
                    .eq(["v1"]),
                "owned interface must remain v1: {path}"
            );
            ensure!(
                path.split('/')
                    .filter(|part| version.is_match(part))
                    .eq(["v1"]),
                "parallel interface version: {path}"
            );
            if let Some(name) = path.strip_prefix("proto/ayni/orchestrator/v1/")
                && let Some(expected) = expected.get(name)
            {
                let actual: BTreeMap<_, _> = service
                    .captures_iter(&source)
                    .map(|c| {
                        (
                            c[1].to_owned(),
                            rpc.captures_iter(&c[2])
                                .map(|r| r[1].to_owned())
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect();
                ensure!(
                    &actual == expected,
                    "unexpected canonical RPC surface in {path}"
                );
            }
        }
        if ["backend/", "clients/", "apps/", "dev/"]
            .iter()
            .any(|prefix| path.starts_with(prefix))
            && !path.contains("/src/generated/")
            && let Ok(source) = fs::read_to_string(&path)
        {
            ensure!(!legacy.is_match(&source), "superseded API consumer: {path}");
        }
    }
    listener_checks()
}

fn listener_checks() -> Result<()> {
    let workflow = fs::read_to_string("proto/ayni/orchestrator/v1/workflow.proto")?;
    for name in [
        "CommandReceipt",
        "ConversationSnapshot",
        "ConversationHistoryRecord",
        "ConversationUpdate",
        "SchedulingState",
        "DispatchState",
        "ExecutionState",
        "PresentationState",
    ] {
        ensure!(
            Regex::new(&format!(r"\b(message|enum)\s+{name}\b"))?.is_match(&workflow),
            "missing canonical symbol: {name}"
        );
    }
    let configuration = fs::read_to_string("backend/servers/control-plane/src/configuration.rs")?;
    ensure!(
        configuration.contains("DEFAULT_GRPC_BIND_ADDRESS")
            && configuration.contains("\"127.0.0.1:50051\""),
        "trusted API default is not loopback"
    );
    let workflow = fs::read_to_string("backend/servers/control-plane/src/workflow.rs")?;
    ensure!(
        workflow.contains("if !address.ip().is_loopback()")
            && workflow.contains("DebugLogsServiceServer::new"),
        "workflow/debug-read authority must remain on loopback"
    );
    let telemetry = fs::read_to_string("backend/servers/control-plane/src/telemetry.rs")?;
    let listener = telemetry
        .split_once("pub async fn serve(")
        .context("missing telemetry listener")?
        .1
        .split("#[tonic::async_trait]")
        .next()
        .context("missing listener boundary")?;
    ensure!(
        listener.matches(".add_service(").count() == 1
            && listener.contains(".add_service(ExecutionTelemetryServiceServer::new")
            && listener.contains("telemetry authority unavailable"),
        "telemetry listener must be authenticated and ingestion-only"
    );
    println!("Canonical v1 contracts and loopback authority passed");
    Ok(())
}
