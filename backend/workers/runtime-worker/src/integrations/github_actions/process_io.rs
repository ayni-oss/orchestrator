use super::*;
pub(super) async fn checkout(
    repository: &str,
    revision: &str,
    destination: &Path,
) -> Result<(), &'static str> {
    if !assessment::hex(revision, 40) {
        return Err("revision_invalid");
    }
    fs::create_dir(destination).map_err(|_| "checkout_unavailable")?;
    command("git", &["init", "--quiet"], Some(destination), 4096).await?;
    // The token is an ephemeral environment header for fetch only. It is not
    // persisted in .git, argv, the candidate container, or evidence.
    let token = variable("GH_TOKEN")?;
    let mut fetch = base_command("git", Some(destination));
    fetch
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "fetch",
            "--quiet",
            "--depth=1",
            "--no-tags",
            "--",
            &format!("https://github.com/{repository}.git"),
            revision,
        ])
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!(
                "AUTHORIZATION: basic {}",
                STANDARD.encode(format!("x-access-token:{token}"))
            ),
        );
    let (exit, _) = collect(fetch, 4096).await?;
    if exit != 0 {
        return Err("checkout_failed");
    }
    command(
        "git",
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "checkout",
            "--quiet",
            "--detach",
            "FETCH_HEAD",
        ],
        Some(destination),
        4096,
    )
    .await?;
    let actual = command("git", &["rev-parse", "HEAD"], Some(destination), 128).await?;
    if actual != format!("{revision}\n").as_bytes() {
        return Err("checkout_mismatch");
    }
    Ok(())
}

pub(super) async fn tracked_paths(root: &Path) -> Result<Vec<String>, &'static str> {
    let bytes = command("git", &["ls-files", "-z"], Some(root), 1024 * 1024).await?;
    let paths: Vec<String> = bytes
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let path = std::str::from_utf8(p).map_err(|_| "source_path_invalid")?;
            if path.len() > 512
                || path.starts_with('/')
                || path
                    .split('/')
                    .any(|p| p == ".." || p == "." || p.is_empty())
                || path
                    .bytes()
                    .any(|b| b.is_ascii_control() || b == b',' || b == b':')
                || !fs::symlink_metadata(root.join(path))
                    .map_err(|_| "source_missing")?
                    .is_file()
            {
                return Err("source_path_unsupported");
            }
            Ok(path.to_owned())
        })
        .collect::<Result<_, _>>()?;
    if paths.is_empty()
        || paths.len() > 2048
        || paths
            .iter()
            .any(|p| p == ".ayni" || p.starts_with(".ayni/"))
    {
        return Err("source_bounds_exceeded");
    }
    command(
        "git",
        &["check-ignore", "--quiet", ".ayni/last/signals.json"],
        Some(root),
        128,
    )
    .await?;
    Ok(paths)
}

pub(super) fn source_digest(root: &Path, paths: &[String]) -> Result<String, &'static str> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut total = 0_usize;
    for path in paths {
        let bytes =
            read_bounded(&root.join(path), 16 * 1024 * 1024).map_err(|_| "source_unavailable")?;
        total += bytes.len();
        if total > 128 * 1024 * 1024 {
            return Err("source_oversized");
        }
        hash.update((path.len() as u64).to_be_bytes());
        hash.update(path.as_bytes());
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}

pub(super) async fn api(endpoint: &str, body: Option<&Value>) -> Result<Value, &'static str> {
    let mut command = base_command("gh", None);
    command
        .env("GH_TOKEN", variable("GH_TOKEN")?)
        .args(["api", endpoint]);
    if let Some(body) = body {
        command
            .args([
                "--method",
                if endpoint.contains("/issues/comments/") {
                    "PATCH"
                } else {
                    "POST"
                },
                "--input",
                "-",
            ])
            .stdin(Stdio::piped());
        let input = serde_json::to_vec(body).map_err(|_| "provider_request_invalid")?;
        command.stdout(Stdio::piped());
        let mut child = command.spawn().map_err(|_| "provider_unavailable")?;
        child
            .stdin
            .take()
            .ok_or("provider_unavailable")?
            .write_all(&input)
            .await
            .map_err(|_| "provider_unavailable")?;
        let mut bytes = Vec::new();
        let mut stdout = child
            .stdout
            .take()
            .ok_or("provider_unavailable")?
            .take(2_097_153);
        tokio::time::timeout(Duration::from_secs(60), stdout.read_to_end(&mut bytes))
            .await
            .map_err(|_| "provider_timeout")?
            .map_err(|_| "provider_unavailable")?;
        if bytes.len() > 2_097_152
            || !child
                .wait()
                .await
                .map_err(|_| "provider_unavailable")?
                .success()
        {
            return Err("provider_unavailable");
        }
        return serde_json::from_slice(&bytes).map_err(|_| "provider_response_invalid");
    }
    let (exit, bytes) = collect(command, 2_097_152).await?;
    if exit != 0 {
        return Err("provider_unavailable");
    }
    serde_json::from_slice(&bytes).map_err(|_| "provider_response_invalid")
}

pub(super) fn base_command(program: &str, directory: Option<&Path>) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", env::var("PATH").unwrap_or_default())
        .env("HOME", env::var("HOME").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    command
}
pub(super) async fn command(
    program: &str,
    args: &[&str],
    directory: Option<&Path>,
    limit: usize,
) -> Result<Vec<u8>, &'static str> {
    let (exit, bytes) = capture(program, args, directory, limit).await?;
    if exit != 0 {
        return Err("launcher_command_failed");
    }
    Ok(bytes)
}
pub(super) async fn capture(
    program: &str,
    args: &[&str],
    directory: Option<&Path>,
    limit: usize,
) -> Result<(i32, Vec<u8>), &'static str> {
    let mut command = base_command(program, directory);
    command.args(args);
    collect(command, limit).await
}
pub(super) async fn collect(
    mut command: Command,
    limit: usize,
) -> Result<(i32, Vec<u8>), &'static str> {
    let mut child = command.spawn().map_err(|_| "launcher_unavailable")?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("launcher_unavailable")?
        .take(limit as u64 + 1);
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(1500), async {
        stdout
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "launcher_failed")?;
        if bytes.len() > limit {
            return Err("launcher_output_oversized");
        }
        let status = child.wait().await.map_err(|_| "launcher_failed")?;
        Ok((status.code().unwrap_or(-1), bytes))
    })
    .await
    .map_err(|_| "launcher_timeout")?
}
