use super::{
    kube,
    process::{self, Owned},
};
use anyhow::{Context as _, Result};
use serde_json::Value;
use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    sync::atomic::Ordering,
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct Serving {
    forwarding: Option<JoinHandle<()>>,
}
impl Serving {
    pub fn start(directory: &Path) -> Result<Self> {
        let port = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
        let gateway = start_gateway(directory, port)?;
        let vite = start_vite()?;
        let directory = directory.to_path_buf();
        let forwarding = thread::spawn(move || forward(&directory, port, gateway, vite));
        Ok(Self {
            forwarding: Some(forwarding),
        })
    }
    pub fn stop(&mut self) {
        if let Some(handle) = self.forwarding.take() {
            let _ = handle.join();
        }
    }
}
fn start_gateway(directory: &Path, port: u16) -> Result<Owned> {
    Owned::spawn(
        Command::new("target/debug/workflow-gateway")
            .args([
                "127.0.0.1:3000",
                &format!("127.0.0.1:{port}"),
                "--dev-status",
            ])
            .arg(directory.join("status.json")),
    )
}
fn start_vite() -> Result<Owned> {
    Owned::spawn(
        Command::new("pnpm")
            .args([
                "--filter",
                "@ayni-orchestrator/web",
                "dev",
                "--host",
                "127.0.0.1",
                "--port",
                "5174",
                "--strictPort",
            ])
            .env_remove("AYNI_ORCHESTRATOR_LIVE_UI_PORT")
            .env_remove("AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT"),
    )
}
fn ready_pod() -> Result<Option<String>> {
    let pods: Value = serde_json::from_str(&process::output(kube().args([
        "get",
        "pods",
        "-l",
        "app.kubernetes.io/name=ayni-orchestrator-control-plane",
        "-o",
        "json",
    ]))?)?;
    Ok(pods["items"]
        .as_array()
        .context("invalid pod inventory")?
        .iter()
        .filter(|pod| {
            pod["metadata"]["deletionTimestamp"].is_null()
                && pod["status"]["phase"] == "Running"
                && pod["status"]["containerStatuses"]
                    .as_array()
                    .is_some_and(|statuses| {
                        statuses
                            .iter()
                            .any(|item| item["name"] == "control-plane" && item["ready"] == true)
                    })
        })
        .max_by_key(|pod| pod["metadata"]["creationTimestamp"].as_str())
        .and_then(|pod| pod["metadata"]["name"].as_str().map(str::to_owned)))
}
fn forward(directory: &Path, port: u16, mut gateway: Owned, mut vite: Owned) {
    let mut child: Option<Owned> = None;
    let mut selected = None;
    let mut binary = fs::metadata("target/debug/workflow-gateway")
        .and_then(|value| value.modified())
        .ok();
    while !process::STOP.load(Ordering::Relaxed) {
        if let (Ok(request), Ok(status)) = (
            fs::read_to_string(directory.join("stop.request")),
            super::state::read(directory),
        ) && request == status.session
        {
            process::STOP.store(true, Ordering::Relaxed);
            break;
        }
        let result = (|| -> Result<()> {
            let modified = fs::metadata("target/debug/workflow-gateway")?.modified()?;
            if binary != Some(modified) || gateway.0.try_wait()?.is_some() {
                gateway.stop();
                gateway = start_gateway(directory, port)?;
                binary = Some(modified);
            }
            if vite.0.try_wait()?.is_some() {
                vite.stop();
                vite = start_vite()?;
            }
            refresh(&mut child, &mut selected, port)
        })();
        let message = match result {
            Ok(())
                if TcpStream::connect_timeout(
                    &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                    Duration::from_millis(200),
                )
                .is_ok() =>
            {
                selected.as_ref().map_or_else(
                    || "Waiting for a ready control plane".into(),
                    |pod| format!("Forwarding {pod}"),
                )
            }
            Ok(()) => "Waiting for a ready control plane".into(),
            Err(error) => format!("Disconnected: {error}"),
        };
        let _ = fs::write(directory.join("gateway-status.txt"), message);
        thread::sleep(Duration::from_secs(1));
    }
}
fn refresh(child: &mut Option<Owned>, selected: &mut Option<String>, port: u16) -> Result<()> {
    let next = ready_pod()?;
    let exited = child
        .as_mut()
        .is_some_and(|child| child.0.try_wait().ok().flatten().is_some());
    if next != *selected || exited {
        *child = None;
        *selected = None;
        if let Some(pod) = next {
            *child = Some(Owned::spawn(kube().args([
                "port-forward",
                "--address",
                "127.0.0.1",
                &format!("pod/{pod}"),
                &format!("{port}:50051"),
            ]))?);
            *selected = Some(pod);
        }
    }
    Ok(())
}
