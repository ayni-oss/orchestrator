use super::{images, inputs, kube, process, state::Status};
use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

pub fn apply(
    directory: &Path,
    status: &mut Status,
    desired: &BTreeMap<String, String>,
    forced: bool,
) -> Result<()> {
    let frozen = inputs::freeze(desired)?;
    let gateway_target = std::env::current_dir()?.join("target/dev-gateway");
    if status.active.get("control-plane") != desired.get("control-plane") {
        status.phase(
            directory,
            "building",
            "Building the local gateway; the previous gateway stays available",
        )?;
        process::run(
            Command::new("cargo")
                .current_dir(frozen.path())
                .env("CARGO_TARGET_DIR", &gateway_target)
                .args([
                    "build",
                    "--locked",
                    "-p",
                    "ayni-orchestrator-control-plane",
                    "--bin",
                    "workflow-gateway",
                ]),
        )?;
    }
    let mut images = status.images.clone();
    for component in ["control-plane", "conversation-supervisor"] {
        if status.active.get(component) != desired.get(component) || !images.contains_key(component)
        {
            status.phase(
                directory,
                "building",
                format!("Building {component}; the previous backend remains active"),
            )?;
            images.insert(
                component.into(),
                images::build(component, &desired[component], frozen.path())?,
            );
        }
    }
    status.phase(
        directory,
        "building",
        "Preparing pinned local runtime images",
    )?;
    let values = images::runtime(&desired["runtime"], directory, frozen.path())?;
    ensure!(
        *desired == inputs::snapshot()?,
        "source changed during compilation; the next scan will rebuild before rollout"
    );
    if status.active.get("control-plane") != desired.get("control-plane") {
        fs::copy(
            gateway_target.join("debug/workflow-gateway"),
            "target/debug/workflow-gateway.next",
        )?;
        fs::rename(
            "target/debug/workflow-gateway.next",
            "target/debug/workflow-gateway",
        )?;
    }
    if !forced && &status.active == desired && backend_ready()? {
        return status.phase(directory, "ready", "Backend ready; frontend HMR enabled");
    }
    let migration = status.active.get("schema") != desired.get("schema");
    if migration {
        status.phase(
            directory,
            "checking_migrations",
            "Validating retained migration history before quiescing admission",
        )?;
        preflight_migration(&images["control-plane"], directory)?;
    }
    deploy(
        directory,
        status,
        &images,
        &values,
        migration,
        frozen.path(),
    )?;
    status.active = desired.clone();
    status.images = images;
    status.phase(
        directory,
        "ready",
        "Backend ready; debug logs and local runtime workers enabled; frontend HMR enabled",
    )
}

fn deploy(
    directory: &Path,
    status: &mut Status,
    images: &BTreeMap<String, String>,
    values: &Value,
    migration: bool,
    source: &Path,
) -> Result<()> {
    let values_path = directory.join("runtime-values.json");
    fs::write(&values_path, serde_json::to_vec(values)?)?;
    let helm_home = tempfile::tempdir()?;
    let helm = || {
        let mut cmd = Command::new("helm");
        cmd.current_dir(source);
        cmd.env("HELM_NO_PLUGINS", "1")
            .env(
                "HELM_REPOSITORY_CONFIG",
                helm_home.path().join("repositories.yaml"),
            )
            .env("HELM_REPOSITORY_CACHE", helm_home.path().join("cache"))
            .env("HELM_PLUGINS", helm_home.path().join("plugins"));
        cmd
    };
    process::run(helm().args(["dependency", "build", "dev/charts/ayni-orchestrator-local"]))?;
    if migration {
        status.phase(
            directory,
            "migrating",
            "Quiescing admission for forward migrations; frontend remains available",
        )?;
        process::run(kube().args(["scale", "deployment/control-plane", "--replicas=0"]))?;
        process::run(kube().args([
            "wait",
            "--for=delete",
            "pod",
            "-l",
            "app.kubernetes.io/name=ayni-orchestrator-control-plane",
            "--timeout=120s",
        ]))?;
    }
    process::run(
        helm()
            .args([
                "upgrade",
                "--install",
                "ayni-orchestrator",
                "dev/charts/ayni-orchestrator-local",
                "--reset-then-reuse-values",
                "--kube-context",
                "docker-desktop",
                "--namespace",
                super::NAMESPACE,
                "--values",
            ])
            .arg(values_path)
            .args([
                "--set",
                "ayni-orchestrator.debug.enabled=true",
                "--set-string",
                &format!(
                    "ayni-orchestrator.images.controlPlane={}",
                    images["control-plane"]
                ),
                "--set-string",
                &format!(
                    "ayni-orchestrator.images.conversationSupervisor={}",
                    images["conversation-supervisor"]
                ),
                "--timeout",
                "5m",
            ]),
    )?;
    status.phase(
        directory,
        "deploying",
        "Waiting for migrations and updated workloads to become ready",
    )?;
    // Restore the singleton replicas even after a previous dev stop or failed rollout.
    resume_workloads()?;
    for workload in [
        "statefulset/postgres",
        "statefulset/jetstream",
        "deployment/control-plane",
        "statefulset/conversation-supervisor",
        "deployment/task-supervisor",
        "deployment/runtime-preparation-supervisor",
        "statefulset/log-collector",
    ] {
        process::run(kube().args(["rollout", "status", workload, "--timeout=300s"]))?;
    }
    Ok(())
}
fn backend_ready() -> Result<bool> {
    for name in [
        "deployment/control-plane",
        "deployment/task-supervisor",
        "deployment/runtime-preparation-supervisor",
        "statefulset/conversation-supervisor",
        "statefulset/log-collector",
    ] {
        if process::output(kube().args(["get", name, "-o", "jsonpath={.status.readyReplicas}"]))?
            != "1"
        {
            return Ok(false);
        }
    }
    Ok(true)
}
pub fn resume_workloads() -> Result<()> {
    process::run(kube().args([
        "scale",
        "deployment/control-plane",
        "deployment/task-supervisor",
        "deployment/runtime-preparation-supervisor",
        "statefulset/conversation-supervisor",
        "statefulset/log-collector",
        "--replicas=1",
    ]))
}
fn preflight_migration(image: &str, directory: &Path) -> Result<()> {
    let deployment: Value = serde_json::from_str(&process::output(kube().args([
        "get",
        "deployment/control-plane",
        "-o",
        "json",
    ]))?)?;
    let pod = &deployment["spec"]["template"]["spec"];
    let mut container = pod["initContainers"]
        .as_array()
        .and_then(|values| values.iter().find(|v| v["name"] == "workflow-migrate"))
        .context("normal migration container is missing")?
        .clone();
    container["image"] = image.into();
    container["args"] = json!(["--check"]);
    let volumes: Vec<_> = pod["volumes"]
        .as_array()
        .context("migration volumes missing")?
        .iter()
        .filter(|volume| volume["name"] == "workflow-postgres-migrator")
        .cloned()
        .collect();
    let name = format!("dev-migration-check-{}", uuid::Uuid::new_v4());
    let job = json!({"apiVersion":"batch/v1","kind":"Job","metadata":{"name":name,"namespace":super::NAMESPACE,"labels":{"app.kubernetes.io/part-of":"ayni-orchestrator","orchestrator.ayni.dev/dev-check":"true"}},"spec":{"backoffLimit":0,"activeDeadlineSeconds":120,"ttlSecondsAfterFinished":3600,"template":{"metadata":{"labels":{"app.kubernetes.io/part-of":"ayni-orchestrator","app.kubernetes.io/name":"ayni-orchestrator-control-plane"}},"spec":{"restartPolicy":"Never","automountServiceAccountToken":false,"securityContext":pod["securityContext"],"containers":[container],"volumes":volumes}}}});
    let path = directory.join("migration-check.json");
    fs::write(&path, serde_json::to_vec(&job)?)?;
    process::run(kube().args(["create", "-f"]).arg(path))?;
    let outcome = process::run(kube().args([
        "wait",
        "--for=condition=complete",
        &format!("job/{name}"),
        "--timeout=130s",
    ]));
    let _ = process::run(kube().args(["logs", &format!("job/{name}")]));
    let _ = process::run(kube().args(["delete", "job", &name, "--wait=true"]));
    outcome.context("retained migration preflight failed; the serving backend has not been stopped")
}
pub fn stop_workloads() -> Result<()> {
    // Never scale database/broker StatefulSets or delete PVCs. No broad resource selectors.
    let result = Command::new("kubectl")
        .args([
            "--context",
            "docker-desktop",
            "--namespace",
            super::NAMESPACE,
            "--request-timeout=20s",
            "scale",
            "deployment/control-plane",
            "deployment/task-supervisor",
            "deployment/runtime-preparation-supervisor",
            "statefulset/conversation-supervisor",
            "statefulset/log-collector",
            "--replicas=0",
        ])
        .status()?;
    ensure!(
        result.success(),
        "could not stop application controllers; storage was retained"
    );
    stop_execution_pods()?;
    Ok(())
}
fn stop_execution_pods() -> Result<()> {
    // These namespaces are fixed isolation boundaries, not discovered environments.
    for (suffix, selector) in [
        (
            "sessions",
            "app.kubernetes.io/name in (ayni-conversation-session,ayni-task-worker)",
        ),
        (
            "runtime",
            "app.kubernetes.io/managed-by=ayni-runtime-supervisor",
        ),
    ] {
        let namespace = format!("{}-{suffix}", super::NAMESPACE);
        let output = kube()
            .args([
                "get",
                "namespace",
                &namespace,
                "--ignore-not-found",
                "-o",
                "json",
            ])
            .output()?;
        ensure!(
            output.status.success(),
            "cannot verify execution namespace ownership"
        );
        if output.stdout.is_empty() {
            continue;
        }
        let owner: Value = serde_json::from_slice(&output.stdout)?;
        ensure!(
            owner["metadata"]["annotations"]["meta.helm.sh/release-namespace"] == super::NAMESPACE
                || owner["metadata"]["labels"]["build.ayni/parent-namespace"] == super::NAMESPACE,
            "execution namespace ownership mismatch"
        );
        ensure!(
            kube()
                .args([
                    "delete",
                    "pods",
                    "-n",
                    &namespace,
                    "-l",
                    selector,
                    "--wait=false",
                    "--ignore-not-found"
                ])
                .status()?
                .success(),
            "could not stop owned execution pods"
        );
    }
    ensure!(
        kube()
            .args([
                "delete",
                "pods",
                "-l",
                "app.kubernetes.io/name=ayni-task-publisher",
                "--wait=false",
                "--ignore-not-found"
            ])
            .status()?
            .success(),
        "could not stop owned publisher"
    );
    Ok(())
}
