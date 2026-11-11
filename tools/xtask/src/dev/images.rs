use super::{kube, process};
use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::Read as _,
    path::Path,
    process::{Command, Stdio},
};

pub fn validate_node() -> Result<()> {
    ensure!(process::output(Command::new("docker").args(["inspect", "desktop-control-plane", "--format", "{{index .Config.Labels \"io.x-k8s.kind.cluster\"}} {{index .Config.Labels \"io.x-k8s.kind.role\"}} "]))? == "desktop control-plane", "Docker Desktop node identity mismatch");
    ensure!(
        process::output(kube().args([
            "get",
            "nodes",
            "-o",
            "jsonpath={.items[*].status.nodeInfo.architecture}"
        ]))? == "arm64",
        "local runtime builds require the native arm64 Docker Desktop node"
    );
    Ok(())
}

pub fn capacity() -> Result<()> {
    for path in [
        std::env::current_dir()?,
        std::env::var_os("HOME").context("HOME missing")?.into(),
    ] {
        let output = process::output(Command::new("df").arg("-Pk").arg(path))?;
        let free: u64 = output
            .lines()
            .nth(1)
            .and_then(|line| line.split_whitespace().nth(3))
            .context("cannot read free disk space")?
            .parse()?;
        ensure!(
            free >= 10 * 1024 * 1024,
            "build paused: less than 10 GiB free; reclaim unused build cache or expand storage (data is never pruned automatically)"
        );
    }
    let node = process::output(Command::new("docker").args([
        "exec",
        "desktop-control-plane",
        "df",
        "-Pk",
        "/var/lib/containerd",
    ]))?;
    let free: u64 = node
        .lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .context("cannot read node disk capacity")?
        .parse()?;
    ensure!(
        free >= 10 * 1024 * 1024,
        "build paused: Docker Desktop node has less than 10 GiB free; storage is never pruned automatically"
    );
    Ok(())
}

pub fn build(component: &str, identity: &str, source: &Path) -> Result<String> {
    capacity()?;
    let repository = format!("local/ayni-live-{component}");
    let temporary = format!("{repository}:build-{}", uuid::Uuid::new_v4());
    process::run(Command::new("docker").current_dir(source).args([
        "build",
        "--file",
        "dev/live.Dockerfile",
        "--build-arg",
        &format!("COMPONENT={component}"),
        "--target",
        component,
        "--label",
        &format!("build.ayni.dev.source={identity}"),
        "--tag",
        &temporary,
        ".",
    ]))?;
    let digest = process::output(
        Command::new("docker").args(["image", "inspect", &temporary, "--format", "{{.Id}} "]),
    )?;
    valid_digest(&digest)?;
    let reference = format!("{repository}:{}-{}", &identity[..12], &digest[7..19]);
    process::run(Command::new("docker").args(["image", "tag", &temporary, &reference]))?;
    process::run(Command::new("docker").args(["image", "remove", &temporary]))?;
    load(&reference)?;
    Ok(reference)
}

pub fn load(reference: &str) -> Result<String> {
    validate_node()?;
    let mut save = process::Owned::spawn(
        Command::new("docker")
            .args(["save", "--platform", "linux/arm64", reference])
            .stdout(Stdio::piped()),
    )?;
    let pipe = save.0.stdout.take().context("docker save stdout missing")?;
    process::run(
        Command::new("docker")
            .args([
                "exec",
                "-i",
                "desktop-control-plane",
                "ctr",
                "--namespace",
                "k8s.io",
                "images",
                "import",
                "--platform",
                "linux/arm64",
                "-",
            ])
            .stdin(Stdio::from(pipe)),
    )?;
    ensure!(save.0.wait()?.success(), "docker save failed");
    let source = format!("docker.io/{reference}");
    let listing = process::output(Command::new("docker").args([
        "exec",
        "desktop-control-plane",
        "ctr",
        "--namespace",
        "k8s.io",
        "images",
        "list",
    ]))?;
    let manifest = listing
        .lines()
        .find_map(|line| {
            let parts: Vec<_> = line.split_whitespace().collect();
            (parts.first() == Some(&source.as_str()))
                .then(|| parts.get(2).copied())
                .flatten()
        })
        .context("imported manifest absent")?;
    valid_digest(manifest)?;
    let immutable = format!(
        "{}@{manifest}",
        source.rsplit_once(':').context("image tag missing")?.0
    );
    if !listing
        .lines()
        .any(|line| line.split_whitespace().next() == Some(&immutable))
    {
        process::run(Command::new("docker").args([
            "exec",
            "desktop-control-plane",
            "ctr",
            "--namespace",
            "k8s.io",
            "images",
            "tag",
            &source,
            &immutable,
        ]))?;
    }
    Ok(immutable)
}

fn valid_digest(value: &str) -> Result<()> {
    ensure!(
        value.len() == 71
            && value.starts_with("sha256:")
            && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid image digest"
    );
    Ok(())
}
fn download(url: &str, path: &Path, expected: &str) -> Result<()> {
    process::run(
        Command::new("curl")
            .args([
                "--fail",
                "--location",
                "--retry",
                "3",
                "--proto",
                "=https",
                "--tlsv1.2",
                url,
                "--output",
            ])
            .arg(path),
    )?;
    ensure!(
        format!("{:x}", Sha256::digest(fs::read(path)?)) == expected,
        "download checksum mismatch"
    );
    Ok(())
}

pub fn runtime(identity: &str, directory: &Path, source: &Path) -> Result<serde_json::Value> {
    let cache = directory.join(format!("runtime-{identity}.json"));
    if let Some(value) = cached_runtime(&cache)? {
        return Ok(value);
    }
    let value = build_runtime(identity, source)?;
    fs::write(cache, serde_json::to_vec_pretty(&value)?)?;
    Ok(value)
}

fn cached_runtime(cache: &Path) -> Result<Option<serde_json::Value>> {
    if let Ok(bytes) = fs::read(cache) {
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let images = process::output(Command::new("docker").args([
            "exec",
            "desktop-control-plane",
            "ctr",
            "--namespace",
            "k8s.io",
            "images",
            "list",
            "--quiet",
        ]))?;
        if ["worker_image", "verifier_image"].iter().all(|key| {
            value["ayni-orchestrator"]["runtimePreparation"]["profile"][key]
                .as_str()
                .is_some_and(|reference| images.lines().any(|line| line == reference))
        }) {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn build_runtime(identity: &str, source: &Path) -> Result<serde_json::Value> {
    capacity()?;
    let pins: serde_json::Value =
        serde_json::from_slice(&fs::read(source.join("tools/xtask/ayni-tool.json"))?)?;
    let pin = |key: &str| -> Result<&str> {
        pins[key].as_str().with_context(|| format!("missing {key}"))
    };
    let context = tempfile::tempdir()?;
    let archive_path = context.path().join("ayni.tar.gz");
    let name = format!(
        "ayni-ayni-v{}-aarch64-unknown-linux-gnu",
        pin("cli_version")?
    );
    download(
        &format!(
            "https://github.com/ayni-oss/ayni/releases/download/ayni-v{}/{name}.tar.gz",
            pin("cli_version")?
        ),
        &archive_path,
        pin("linux_arm64_archive_sha256")?,
    )?;
    let mut archive =
        tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(archive_path)?));
    let mut cli = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new(&format!("{name}/ayni"))
            && entry.header().entry_type().is_file()
        {
            entry.read_to_end(&mut cli)?;
        }
    }
    ensure!(
        format!("{:x}", Sha256::digest(&cli)) == pin("linux_arm64_cli_sha256")?,
        "CLI checksum mismatch"
    );
    fs::write(context.path().join("ayni"), cli)?;
    download(
        "https://github.com/opencontainers/umoci/releases/download/v0.6.0/umoci.linux.arm64",
        &context.path().join("umoci.linux.arm64"),
        "5cfd17f2e7a4bcf9ed67ea1b955ca893d200349b9ce6a3d3707dba415f458a1f",
    )?;
    process::run(Command::new("docker").current_dir(source).args([
        "buildx",
        "build",
        "--platform",
        "linux/arm64",
        "--file",
        "deploy/images/runtime-preparation/Dockerfile.binaries",
        "--build-arg",
        "KEEP_TARGET_CACHE=true",
        "--target",
        "binaries",
        "--output",
        &format!("type=local,dest={}", context.path().display()),
        ".",
    ]))?;
    let mut references = Vec::new();
    for (kind, dockerfile) in [("worker", "Dockerfile"), ("probes", "Dockerfile.probes")] {
        capacity()?;
        let reference = format!("local/ayni-runtime-{kind}:dev-{}", &identity[..16]);
        process::run(
            Command::new("docker")
                .current_dir(source)
                .args([
                    "build",
                    "--platform",
                    "linux/arm64",
                    "--file",
                    &format!("deploy/images/runtime-preparation/{dockerfile}"),
                    "--build-arg",
                    &format!("AYNI_CLI_SHA256={}", pin("linux_arm64_cli_sha256")?),
                    "--tag",
                    &reference,
                ])
                .arg(context.path()),
        )?;
        references.push(load(&reference)?);
    }
    let values = serde_json::json!({"ayni-orchestrator":{"runtimePreparation":{"enabled":true,"localDevelopment":true,"publicEgress":true,"profile":{"profile_id":"rootless-podman-vfs-userns-local-development-v1","qualified":false,"platform":"linux/arm64","worker_image":references[0],"verifier_image":references[1],"executor_image":pin("linux_arm64_executor_image")?,"trusted_cli_sha256":format!("sha256:{}",pin("linux_arm64_cli_sha256")?)}}}});
    Ok(values)
}
