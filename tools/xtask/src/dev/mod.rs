mod credentials;
mod images;
mod inputs;
mod process;
mod reconcile;
mod serving;
mod state;

use anyhow::{Context as _, Result, ensure};
use state::Status;
use std::{
    env, fs,
    net::TcpListener,
    path::Path,
    process::Command,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

pub const NAMESPACE: &str = "ayni-orchestrator-live";
pub const URL: &str = "http://127.0.0.1:5174";
pub fn kube() -> Command {
    let mut command = Command::new("kubectl");
    command.args([
        "--context",
        "docker-desktop",
        "--namespace",
        NAMESPACE,
        "--request-timeout=20s",
    ]);
    command
}

pub fn run(args: &[String]) -> Result<()> {
    ensure!(
        args.len() <= 1,
        "usage: cargo xtask dev [status|stop|redeploy]"
    );
    let action = args.first().map_or("start", String::as_str);
    ensure!(
        ["start", "status", "stop", "redeploy"].contains(&action),
        "unknown dev command: {action}"
    );
    let directory = state::directory()?;
    let lock = state::lock(&directory)?;
    if lock.try_lock().is_err() {
        let owner =
            state::read(&directory).context("controller is starting; retry status shortly")?;
        if action == "stop" || action == "redeploy" {
            if action == "redeploy" {
                ensure!(
                    owner.checkout == env::current_dir()?,
                    "{} owns development; stop it before changing checkout",
                    owner.checkout.display()
                );
            }
            fs::write(directory.join(format!("{action}.request")), &owner.session)?;
            println!("Requested {action} from controller {} at {URL}", owner.pid);
            if action == "stop" {
                let deadline = Instant::now() + Duration::from_secs(45);
                while lock.try_lock().is_err() {
                    ensure!(
                        Instant::now() < deadline,
                        "controller is still stopping; inspect cargo xtask dev status"
                    );
                    thread::sleep(Duration::from_millis(100));
                }
            }
        } else {
            print_status(&owner, true)?;
        }
        return Ok(());
    }
    if action == "status" {
        if let Ok(status) = state::read(&directory) {
            print_status(&status, false)?;
        } else {
            println!("Development is stopped. Start with cargo xtask dev: {URL}");
        }
        return Ok(());
    }
    if action == "stop" {
        reconcile::stop_workloads()?;
        return Ok(());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    let _signals = runtime.spawn(async {
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
            process::STOP.store(true, Ordering::Relaxed);
        }
    });
    start(&directory)
}
fn print_status(status: &Status, running: bool) -> Result<()> {
    let mut value = serde_json::to_value(status)?;
    value["controller_running"] = running.into();
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
fn preflight() -> Result<()> {
    for name in [
        "AYNI_ORCHESTRATOR_CONTEXT",
        "AYNI_ORCHESTRATOR_NAMESPACE",
        "AYNI_ORCHESTRATOR_LIVE_UI_PORT",
        "AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT",
    ] {
        ensure!(
            env::var_os(name).is_none(),
            "remove {name}; development has one fixed context, namespace and URL"
        );
    }
    for port in [5174, 3000] {
        TcpListener::bind(("127.0.0.1", port)).with_context(|| format!("127.0.0.1:{port} is occupied; stop its previous owner (no alternate URL will be selected)"))?;
    }
    ensure!(
        process::output(Command::new("kubectl").args([
            "config",
            "get-contexts",
            "docker-desktop",
            "-o",
            "name"
        ]))? == "docker-desktop",
        "Docker Desktop Kubernetes context unavailable"
    );
    images::validate_node()?;
    credentials::ensure_local()?;
    ensure!(
        process::output(Command::new("helm").args(["version", "--template", "{{ .Version }}"]))?
            .starts_with("v4.1.1"),
        "Helm 4.1.1 required"
    );
    Ok(())
}
fn start(directory: &Path) -> Result<()> {
    preflight()?;
    let previous = state::read(directory).unwrap_or_default();
    let checkout = env::current_dir()?;
    let mut status = Status {
        pid: std::process::id(),
        session: uuid::Uuid::new_v4().to_string(),
        checkout: checkout.clone(),
        branch: process::output(Command::new("git").args(["branch", "--show-current"]))?,
        url: URL.into(),
        ..Status::default()
    };
    if previous.checkout == checkout {
        status.active = previous.active;
        status.images = previous.images;
    }
    status.phase(
        directory,
        "starting",
        "Preparing the singleton development session",
    )?;
    let initial = inputs::snapshot()?;
    process::run(Command::new("cargo").args([
        "build",
        "--locked",
        "-p",
        "ayni-orchestrator-control-plane",
        "--bin",
        "workflow-gateway",
    ]))?;
    process::run(Command::new("pnpm").args(["install", "--frozen-lockfile"]))?;
    let mut serving = serving::Serving::start(directory)?;
    let outcome = watch(directory, &mut status, &initial);
    process::STOP.store(true, Ordering::Relaxed);
    serving.stop();
    // Shutdown must remain possible after cancellation has interrupted a build.
    let shutdown = reconcile::stop_workloads();
    status.phase(
        directory,
        "stopped",
        "Host processes and application controllers stopped; storage retained",
    )?;
    outcome.and(shutdown)
}
fn requested(directory: &Path, action: &str, session: &str) -> bool {
    let path = directory.join(format!("{action}.request"));
    let valid = fs::read_to_string(&path).is_ok_and(|value| value == session);
    if valid {
        let _ = fs::remove_file(path);
    }
    valid
}
fn watch(
    directory: &Path,
    status: &mut Status,
    initial: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let mut attempted = None;
    let mut desired = initial.clone();
    let mut changed = Instant::now();
    loop {
        if process::STOP.load(Ordering::Relaxed) || requested(directory, "stop", &status.session) {
            return Ok(());
        }
        let current = inputs::snapshot()?;
        if current.get("controller") == initial.get("controller") {
            if current != desired {
                desired = current;
                changed = Instant::now();
            }
            let forced = requested(directory, "redeploy", &status.session);
            if forced
                || (attempted.as_ref() != Some(&desired)
                    && changed.elapsed() >= Duration::from_secs(1))
            {
                attempted = Some(desired.clone());
                if let Err(error) = reconcile::apply(directory, status, &desired, forced) {
                    if process::STOP.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    status.phase(
                        directory,
                        "failed",
                        format!(
                            "{error:#}. Fix the source or run cargo xtask dev redeploy to retry."
                        ),
                    )?;
                }
            }
        } else {
            status.phase(
                directory,
                "restart_required",
                "Development controller changed; run cargo xtask dev stop, then cargo xtask dev",
            )?;
        }
        status.gateway = fs::read_to_string(directory.join("gateway-status.txt"))
            .unwrap_or_else(|_| "starting".into());
        status.save(directory)?;
        thread::sleep(Duration::from_millis(500));
    }
}
