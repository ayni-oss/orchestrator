//! Serial acceptance against the singleton deployment. Fixtures never create a stack.
use crate::{command, dev};
use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub fn run(profile: Option<&str>) -> Result<()> {
    let directory = std::path::PathBuf::from(std::env::var_os("HOME").context("HOME missing")?)
        .join(".local/state/ayni-orchestrator/dev");
    std::fs::create_dir_all(&directory)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join("acceptance.lock"))?;
    lock.try_lock()
        .context("another deployment acceptance run owns the canonical environment")?;
    ensure!(
        rpc("GetDebugCapabilities", &json!({}))?["enabled"] == true,
        "start cargo xtask dev with debug enabled"
    );
    match profile.unwrap_or("debug") {
        "debug" => debug(),
        "recovery" => recovery(),
        "topology" => super::topology::run(),
        "browser" => browser(),
        "hmr" => browser_suite(
            &["apps/web/tests/hmr.browser.mjs"],
            &std::collections::BTreeMap::new(),
        ),
        other => anyhow::bail!("unknown deployment acceptance profile: {other}"),
    }
}
pub fn kube(args: &[&str]) -> Result<Value> {
    let output = dev::kube().args(args).output()?;
    ensure!(
        output.status.success(),
        "kubectl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}
fn browser_suite(files: &[&str], env: &std::collections::BTreeMap<String, String>) -> Result<()> {
    ensure!(
        Command::new("node")
            .args(["--test", "--test-concurrency=1"])
            .args(files)
            .envs(env)
            .status()?
            .success(),
        "browser acceptance failed"
    );
    Ok(())
}
fn browser() -> Result<()> {
    let pods = kube(&[
        "get",
        "pods",
        "-l",
        "app.kubernetes.io/name=ayni-orchestrator-control-plane",
        "-o",
        "json",
    ])?;
    let pod = pods["items"]
        .as_array()
        .context("control-plane pods missing")?
        .iter()
        .find(|pod| {
            pod["status"]["containerStatuses"]
                .as_array()
                .is_some_and(|cs| {
                    cs.iter()
                        .any(|c| c["name"] == "control-plane" && c["ready"] == true)
                })
        })
        .and_then(|pod| pod["metadata"]["name"].as_str())
        .context("ready control plane missing")?;
    let env = kube(&[
        "exec",
        pod,
        "-c",
        "control-plane",
        "--",
        "/usr/local/bin/workflow-browser-fixture",
        "--canonical-local-acceptance",
    ])?;
    let env = serde_json::from_value(env)?;
    browser_suite(
        &[
            "apps/web/tests/connections.browser.mjs",
            "apps/web/tests/repositories.browser.mjs",
            "apps/web/tests/runtime-preparation.browser.mjs",
            "apps/web/tests/runtime-signing.browser.mjs",
        ],
        &env,
    )
}
pub fn rpc(method: &str, body: &Value) -> Result<Value> {
    let bytes = command::input(
        "curl",
        &[
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            "15",
            "-H",
            "content-type: application/json",
            "-H",
            "x-console-request: 1",
            "--data-binary",
            "@-",
            &format!("http://127.0.0.1:3000/api/{method}"),
        ],
        &serde_json::to_vec(body)?,
    )?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn sources() -> Result<Vec<Value>> {
    let mut all = Vec::new();
    let mut after = String::new();
    loop {
        let page = rpc("ListLogSources", &json!({"afterId":after}))?;
        if let Some(sources) = page["sources"].as_array() {
            all.extend(sources.iter().cloned());
        }
        after = page["nextAfterId"].as_str().unwrap_or_default().into();
        if after.is_empty() {
            return Ok(all);
        }
    }
}
fn eventually<T>(mut check: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(150);
    loop {
        if let Some(value) = check()? {
            return Ok(value);
        }
        ensure!(Instant::now() < deadline, "deployment acceptance timed out");
        thread::sleep(Duration::from_secs(1));
    }
}
struct Fixtures {
    image: String,
    names: Vec<String>,
}
impl Fixtures {
    fn new() -> Result<Self> {
        let deployment = kube(&["get", "deployment/control-plane", "-o", "json"])?;
        Ok(Self {
            image: deployment["spec"]["template"]["spec"]["containers"][0]["image"]
                .as_str()
                .context("control-plane image missing")?
                .into(),
            names: Vec::new(),
        })
    }
    fn create(
        &mut self,
        kind: &str,
        owned: bool,
        containers: &[(&str, &str)],
        init: &[(&str, &str)],
        restart: &str,
    ) -> Result<String> {
        let name = format!("dev-check-{kind}-{}", uuid::Uuid::new_v4().simple());
        let labels = if owned {
            json!({"app.kubernetes.io/name":"ayni-debug-qualification", "app.kubernetes.io/part-of":"ayni-orchestrator", "ayni.build/deployment":dev::NAMESPACE})
        } else {
            json!({})
        };
        let container = |(name, mode): &(&str, &str)| json!({"name":name,"image":self.image,"command":["/usr/local/bin/workflow-log-fixture",mode],"resources":{"limits":{"memory":"32Mi","cpu":"100m"}},"securityContext":{"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}}});
        let pod = json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"namespace":dev::NAMESPACE,"labels":labels},"spec":{"automountServiceAccountToken":false,"restartPolicy":restart,"activeDeadlineSeconds":600,"containers":containers.iter().map(container).collect::<Vec<_>>(),"initContainers":init.iter().map(container).collect::<Vec<_>>()}});
        command::input(
            "kubectl",
            &[
                "--context",
                "docker-desktop",
                "-n",
                dev::NAMESPACE,
                "create",
                "-f",
                "-",
            ],
            &serde_json::to_vec(&pod)?,
        )?;
        self.names.push(name.clone());
        Ok(name)
    }
    fn delete(name: &str, force: bool) -> Result<()> {
        let mut cmd = dev::kube();
        cmd.args(["delete", "pod", name, "--ignore-not-found"]);
        if force {
            cmd.args(["--grace-period=0", "--force", "--wait=false"]);
        }
        ensure!(cmd.status()?.success(), "fixture cleanup failed");
        Ok(())
    }
}
impl Drop for Fixtures {
    fn drop(&mut self) {
        for name in &self.names {
            let _ = Self::delete(name, false);
        }
    }
}
fn download(id: &str) -> Result<Vec<u8>> {
    let id: String = url::form_urlencoded::byte_serialize(id.as_bytes()).collect();
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            "15",
            &format!("http://127.0.0.1:3000/debug/logs/download?sourceId={id}"),
        ])
        .output()?;
    ensure!(output.status.success(), "raw download failed");
    Ok(output.stdout)
}
fn debug() -> Result<()> {
    let mut fixtures = Fixtures::new()?;
    let name = fixtures.create(
        "capture",
        true,
        &[("main", "bytes"), ("sidecar", "sidecar")],
        &[("init", "init")],
        "Never",
    )?;
    let unrelated = fixtures.create("unrelated", false, &[("main", "quiet")], &[], "Never")?;
    let failed = fixtures.create(
        "init",
        true,
        &[("never-started", "quiet")],
        &[("failing-init", "fail")],
        "Never",
    )?;
    let inventory = eventually(|| {
        let all = sources()?;
        let present = ["main", "sidecar", "init"].iter().all(|c| {
            all.iter()
                .any(|s| s["podName"] == name && s["container"] == *c)
        }) && all
            .iter()
            .any(|s| s["podName"] == failed && s["container"] == "never-started");
        Ok(present.then_some(all))
    })?;
    ensure!(
        !inventory.iter().any(|s| s["podName"] == unrelated),
        "unrelated pod was collected"
    );
    let id = inventory
        .iter()
        .find(|s| s["podName"] == name && s["container"] == "main")
        .and_then(|s| s["id"].as_str())
        .context("fixture source missing")?;
    let expected =
        b"same\nsame\n<img src=x onerror=globalThis.__rawExecuted=true>\x1b[2J\0\xfftail";
    let before = eventually(|| {
        let bytes = download(id)?;
        Ok(bytes
            .windows(expected.len())
            .any(|window| window == expected)
            .then_some(bytes))
    })?;
    Fixtures::delete(&name, false)?;
    ensure!(
        download(id)? == before,
        "retained raw bytes changed after pod deletion"
    );
    ensure!(
        Command::new("node")
            .arg("apps/web/tests/debug-deployed.acceptance.mjs")
            .env("AYNI_DEBUG_SOURCE_ID", id)
            .status()?
            .success(),
        "frontend debug acceptance failed"
    );
    println!(
        "Passed managed init/sidecar/failed-start inventory, unrelated exclusion, identical lines, binary output, retained download and browser rendering at {}",
        dev::URL
    );
    Ok(())
}
fn recovery() -> Result<()> {
    let mut fixtures = Fixtures::new()?;
    let crash = fixtures.create("crash", true, &[("main", "restart")], &[], "OnFailure")?;
    let oom = fixtures.create("oom", true, &[("main", "oom")], &[], "Never")?;
    let force = fixtures.create("force", true, &[("main", "idle")], &[], "Never")?;
    eventually(|| {
        let pod = kube(&["get", "pod", &crash, "-o", "json"])?;
        Ok(pod["status"]["containerStatuses"]
            .as_array()
            .is_some_and(|s| {
                s.iter()
                    .any(|c| c["restartCount"].as_u64().unwrap_or(0) >= 2)
            })
            .then_some(()))
    })?;
    eventually(|| {
        let pod = kube(&["get", "pod", &oom, "-o", "json"])?;
        Ok(pod["status"]["containerStatuses"]
            .as_array()
            .is_some_and(|s| {
                s.iter()
                    .any(|c| c["state"]["terminated"]["reason"] == "OOMKilled")
            })
            .then_some(()))
    })?;
    let before = eventually(|| {
        let all = sources()?;
        let ready = all
            .iter()
            .any(|s| s["podName"] == crash && s["restart"].as_u64().unwrap_or(0) >= 2)
            && [&oom, &force]
                .iter()
                .all(|name| all.iter().any(|s| s["podName"] == **name));
        Ok(ready.then_some(all))
    })?;
    Fixtures::delete(&force, true)?;
    eventually(|| {
        Ok(sources()?
            .iter()
            .any(|s| {
                s["podName"] == force
                    && s["captureStatus"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("deleted")
            })
            .then_some(()))
    })?;
    ensure!(
        dev::kube()
            .args(["delete", "pod", "log-collector-0", "--wait=true"])
            .status()?
            .success(),
        "collector restart failed"
    );
    ensure!(
        dev::kube()
            .args([
                "rollout",
                "status",
                "statefulset/log-collector",
                "--timeout=180s"
            ])
            .status()?
            .success(),
        "collector failed to recover"
    );
    let after = sources()?;
    for source in before.iter().filter(|s| {
        [&crash, &oom, &force]
            .iter()
            .any(|name| s["podName"] == **name)
    }) {
        ensure!(
            after.iter().any(|s| s["id"] == source["id"]),
            "source lost after collector restart"
        );
    }
    ensure!(
        after
            .iter()
            .filter(|s| s["podName"] == crash)
            .all(|s| s["captureStatus"] != "complete"),
        "API-only collection claimed completeness"
    );
    println!(
        "Passed crash restart inventory, OOM, forced deletion uncertainty and collector recovery"
    );
    Ok(())
}
