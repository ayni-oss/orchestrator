//! Complete frozen Git checkout acquisition, isolated from repository execution.
use crate::{Failure, command};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read as _,
    path::{Component, Path},
    time::Duration,
};

const MAX_FILES: usize = 10_000;
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;

/// Fetch only the selected immutable commit with one repository-read capability.
///
/// No repository hooks, filters, global configuration or credential helpers run.
/// The destination must be a new operation-owned directory.
/// # Errors
/// Reject unsupported paths/objects, changed identities, resource bounds or fetch failures.
pub async fn acquire(
    full_name: &str,
    commit: &str,
    tree: &str,
    token: &str,
    destination: &Path,
    seconds: u64,
) -> Result<String, Failure> {
    if !repository_name(full_name) || !object_id(commit) || !object_id(tree) || token.is_empty() {
        return Err(Failure::InvalidInput);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    tokio::time::timeout_at(deadline, github_tree_preflight(full_name, tree, token))
        .await
        .map_err(|_| Failure::ExecutionDeadline)??;
    fs::create_dir(destination).map_err(|_| Failure::ArtifactConflict)?;
    let empty = tempfile::tempdir().map_err(|_| Failure::ExecutionFailed)?;
    let mut environment = BTreeMap::from([
        ("HOME".into(), empty.path().to_string_lossy().into_owned()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        (
            "GIT_CONFIG_KEY_0".into(),
            "http.https://github.com/.extraHeader".into(),
        ),
        (
            "GIT_CONFIG_VALUE_0".into(),
            format!(
                "Authorization: Basic {}",
                STANDARD.encode(format!("x-access-token:{token}"))
            ),
        ),
    ]);
    git(
        destination,
        &[
            "init",
            "--quiet",
            "--template",
            &empty.path().to_string_lossy(),
        ],
        &environment,
        seconds,
    )
    .await?;
    git(
        destination,
        &[
            "remote",
            "add",
            "origin",
            &format!("https://github.com/{full_name}.git"),
        ],
        &environment,
        seconds,
    )
    .await?;
    fetch_commit(destination, commit, &environment, seconds, deadline).await?;
    // No provider capability is available to checkout or any subsequent stage.
    environment.remove("GIT_CONFIG_COUNT");
    environment.remove("GIT_CONFIG_KEY_0");
    environment.remove("GIT_CONFIG_VALUE_0");
    let observed = git(
        destination,
        &["rev-parse", "FETCH_HEAD^0", "FETCH_HEAD^0:"],
        &environment,
        seconds,
    )
    .await?;
    let expected = format!("{commit}\n{tree}\n");
    if observed != expected.as_bytes() {
        return Err(Failure::SourceIdentityMismatch);
    }
    let entries = git(
        destination,
        &["ls-tree", "-r", "-l", "-z", "FETCH_HEAD"],
        &environment,
        seconds,
    )
    .await?;
    validate_tree(&entries)?;
    git(
        destination,
        &["checkout", "--quiet", "--detach", "FETCH_HEAD"],
        &environment,
        seconds,
    )
    .await?;
    fs::remove_dir_all(destination.join(".git")).map_err(|_| Failure::ExecutionFailed)?;
    snapshot_digest(destination)
}

async fn github_tree_preflight(repository: &str, tree: &str, token: &str) -> Result<(), Failure> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| Failure::SourceTreeUnavailable)?;
    let url = format!("https://api.github.com/repos/{repository}/git/trees/{tree}?recursive=1");
    fetch_tree(&client, &url, tree, token).await
}

async fn fetch_commit(
    destination: &Path,
    commit: &str,
    environment: &BTreeMap<String, String>,
    seconds: u64,
    deadline: tokio::time::Instant,
) -> Result<(), Failure> {
    tokio::time::timeout_at(
        deadline,
        git(
            destination,
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                "--depth=1",
                "origin",
                commit,
            ],
            environment,
            seconds,
        ),
    )
    .await
    .map_err(|_| Failure::ExecutionDeadline)?
    .map_err(|reason| match reason {
        Failure::ExecutionFailed => Failure::SourceFetchUnavailable,
        other => other,
    })?;
    Ok(())
}

async fn fetch_tree(
    client: &reqwest::Client,
    url: &str,
    tree: &str,
    token: &str,
) -> Result<(), Failure> {
    let request = client
        .get(url)
        .header("User-Agent", "ayni-runtime-source")
        .header("Accept", "application/vnd.github+json")
        .bearer_auth(token);
    for delay in [250, 500] {
        let attempt = request.try_clone().ok_or(Failure::SourceTreeUnavailable)?;
        match tree_attempt(attempt, tree).await {
            Err(Failure::SourceTreeUnavailable) => {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            result => return result,
        }
    }
    tree_attempt(request, tree).await
}

async fn tree_attempt(request: reqwest::RequestBuilder, tree: &str) -> Result<(), Failure> {
    let mut response = request
        .send()
        .await
        .map_err(|_| Failure::SourceTreeUnavailable)?;
    match response.status().as_u16() {
        200 => {}
        403 if response
            .headers()
            .get("x-ratelimit-remaining")
            .is_some_and(|value| value == "0") =>
        {
            return Err(Failure::SourceRateLimited);
        }
        401 | 403 | 404 => return Err(Failure::SourceAccessDenied),
        429 => return Err(Failure::SourceRateLimited),
        500..=599 => return Err(Failure::SourceTreeUnavailable),
        _ => return Err(Failure::SourceUnavailable),
    }
    if response
        .content_length()
        .is_some_and(|size| size > 4 * 1024 * 1024)
    {
        return Err(Failure::SourceBoundsExceeded);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Failure::SourceTreeUnavailable)?
    {
        if body.len().saturating_add(chunk.len()) > 4 * 1024 * 1024 {
            return Err(Failure::SourceBoundsExceeded);
        }
        body.extend_from_slice(&chunk);
    }
    validate_github_tree(&body, tree)
}
fn validate_github_tree(bytes: &[u8], tree: &str) -> Result<(), Failure> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| Failure::SourceIdentityMismatch)?;
    if value["sha"] != tree || value["truncated"] != false {
        return Err(Failure::SourceIdentityMismatch);
    }
    let entries = value["tree"]
        .as_array()
        .ok_or(Failure::SourceIdentityMismatch)?;
    if entries.len() > MAX_FILES * 2 {
        return Err(Failure::SourceBoundsExceeded);
    }
    let mut files = 0usize;
    let mut directories = 0usize;
    let mut total = 0_u64;
    for entry in entries {
        let path = entry["path"]
            .as_str()
            .ok_or(Failure::SourcePathUnsupported)?;
        if !safe_path(path) || !entry["sha"].as_str().is_some_and(object_id) {
            return Err(Failure::SourcePathUnsupported);
        }
        match (entry["type"].as_str(), entry["mode"].as_str()) {
            (Some("tree"), Some("040000")) => {
                directories += 1;
                if directories >= MAX_FILES {
                    return Err(Failure::SourceBoundsExceeded);
                }
            }
            (Some("blob"), Some("100644" | "100755")) => {
                files += 1;
                let size = entry["size"]
                    .as_u64()
                    .ok_or(Failure::SourceIdentityMismatch)?;
                total = total
                    .checked_add(size)
                    .ok_or(Failure::SourceBoundsExceeded)?;
                if files > MAX_FILES || size > MAX_FILE_BYTES || total > MAX_SOURCE_BYTES {
                    return Err(Failure::SourceBoundsExceeded);
                }
            }
            _ => return Err(Failure::SourcePathUnsupported),
        }
    }
    Ok(())
}

async fn git(
    root: &Path,
    args: &[&str],
    environment: &BTreeMap<String, String>,
    seconds: u64,
) -> Result<Vec<u8>, Failure> {
    let mut arguments: Vec<String> = [
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "credential.helper=",
        "-c",
        "http.followRedirects=false",
        "-c",
        "protocol.file.allow=never",
        "-c",
        "submodule.recurse=false",
        "-c",
        "fetch.fsckObjects=true",
        "-c",
        "transfer.fsckObjects=true",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    arguments.extend(args.iter().map(|arg| (*arg).to_owned()));
    command::checked("/usr/bin/git", &arguments, root, environment, seconds).await
}

fn repository_name(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 100
                && *part != "."
                && *part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        })
}
fn object_id(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && !value.contains(['\\', '\0'])
        && value.split('/').count() <= 64
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(name) if name != ".git"))
}

/// Validate all Git modes and paths before checkout can create any repository file.
/// # Errors
/// Reject symlinks, submodules, unsafe paths, malformed object IDs and excess entries.
pub fn validate_tree(bytes: &[u8]) -> Result<(), Failure> {
    let mut count = 0;
    let mut total = 0_u64;
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        count += 1;
        if count > MAX_FILES {
            return Err(Failure::SourceBoundsExceeded);
        }
        let record = std::str::from_utf8(record).map_err(|_| Failure::SourcePathUnsupported)?;
        let (identity, path) = record
            .split_once('\t')
            .ok_or(Failure::SourcePathUnsupported)?;
        let identity: Vec<_> = identity.split_ascii_whitespace().collect();
        if identity.len() != 4
            || !matches!(identity[0], "100644" | "100755")
            || identity[1] != "blob"
            || !object_id(identity[2])
            || !safe_path(path)
        {
            return Err(Failure::SourcePathUnsupported);
        }
        let size = identity[3]
            .parse::<u64>()
            .map_err(|_| Failure::SourcePathUnsupported)?;
        total = total
            .checked_add(size)
            .ok_or(Failure::SourceBoundsExceeded)?;
        if size > MAX_FILE_BYTES || total > MAX_SOURCE_BYTES {
            return Err(Failure::SourceBoundsExceeded);
        }
    }
    Ok(())
}

/// Hash every actual checkout file under the stage's time/memory/storage limits.
/// Symlinks and special files fail closed. This is source provenance, not Ayni policy evaluation.
/// # Errors
/// Reject inaccessible/unsupported/oversized or concurrently changed source.
pub fn snapshot_digest(root: &Path) -> Result<String, Failure> {
    if !fs::symlink_metadata(root)
        .map_err(|_| Failure::SourceUnavailable)?
        .is_dir()
    {
        return Err(Failure::SourcePathUnsupported);
    }
    let mut directories = 1usize;
    let mut pending = vec![root.to_path_buf()];
    let mut files = BTreeMap::new();
    let mut total = 0_u64;
    while let Some(directory) = pending.pop() {
        for item in fs::read_dir(directory).map_err(|_| Failure::SourceUnavailable)? {
            let item = item.map_err(|_| Failure::SourceUnavailable)?;
            let path = item.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| Failure::SourcePathUnsupported)?;
            let relative = relative.to_str().ok_or(Failure::SourcePathUnsupported)?;
            if !safe_path(relative) {
                return Err(Failure::SourcePathUnsupported);
            }
            let metadata = fs::symlink_metadata(&path).map_err(|_| Failure::SourceUnavailable)?;
            if metadata.is_dir() {
                directories = directories
                    .checked_add(1)
                    .ok_or(Failure::SourceBoundsExceeded)?;
                if directories > MAX_FILES {
                    return Err(Failure::SourceBoundsExceeded);
                }
                pending.push(path);
            } else if metadata.is_file() {
                total = total
                    .checked_add(metadata.len())
                    .ok_or(Failure::SourceBoundsExceeded)?;
                if files.len() >= MAX_FILES
                    || metadata.len() > MAX_FILE_BYTES
                    || total > MAX_SOURCE_BYTES
                {
                    return Err(Failure::SourceBoundsExceeded);
                }
                let digest = hash_file(&path, metadata.len())?;
                files.insert(
                    relative.to_owned(),
                    (metadata.len(), file_mode(&metadata), digest),
                );
            } else {
                return Err(Failure::SourcePathUnsupported);
            }
        }
    }
    let mut digest = Sha256::new();
    digest.update(b"ayni.runtime.source.v1\0");
    for (path, (length, mode, content)) in files {
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
        digest.update(length.to_be_bytes());
        digest.update(mode.to_be_bytes());
        digest.update(content);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn hash_file(path: &Path, expected_bytes: u64) -> Result<[u8; 32], Failure> {
    let before = fs::symlink_metadata(path).map_err(|_| Failure::SourceUnavailable)?;
    let file = fs::File::open(path).map_err(|_| Failure::SourceUnavailable)?;
    if !before.is_file()
        || !same_file(
            &before,
            &file.metadata().map_err(|_| Failure::SourceUnavailable)?,
        )
    {
        return Err(Failure::SourceIdentityMismatch);
    }
    let mut source = file.take(expected_bytes + 1);
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 65_536].into_boxed_slice();
    let mut total = 0_u64;
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|_| Failure::SourceUnavailable)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > expected_bytes {
            return Err(Failure::SourceIdentityMismatch);
        }
        hash.update(&buffer[..count]);
    }
    if total != expected_bytes {
        return Err(Failure::SourceIdentityMismatch);
    }
    Ok(hash.finalize().into())
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o777
}
#[cfg(not(unix))]
fn file_mode(_metadata: &fs::Metadata) -> u32 {
    0
}
#[cfg(unix)]
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    a.dev() == b.dev() && a.ino() == b.ino() && a.len() == b.len() && file_mode(a) == file_mode(b)
}
#[cfg(not(unix))]
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.len() == b.len()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acquisition_rejects_git_paths_that_could_run_or_escape_the_snapshot() {
        let object = "a".repeat(40);
        for (mode, path) in [
            ("120000", "link"),
            ("160000", "module"),
            ("100644", "../escape"),
            ("100644", ".git/config"),
            ("100644", "/absolute"),
        ] {
            assert!(validate_tree(format!("{mode} blob {object} 1\t{path}\0").as_bytes()).is_err());
        }
        assert!(
            validate_tree(
                format!(
                    "100644 blob {object} 1\t.ayni.toml\0100755 blob {object} 1\tscripts/check\0"
                )
                .as_bytes()
            )
            .is_ok()
        );
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
    #[test]
    fn declared_git_sizes_are_rejected_before_checkout() {
        let object = "a".repeat(40);
        assert!(matches!(
            validate_tree(
                format!("100644 blob {object} {}\tlarge\0", MAX_FILE_BYTES + 1).as_bytes()
            ),
            Err(Failure::SourceBoundsExceeded)
        ));
    }
    #[cfg(unix)]
    #[test]
    fn executable_mode_is_part_of_source_identity_and_symlinks_fail() -> TestResult {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir()?;
        let file = root.path().join("program");
        fs::write(&file, b"source")?;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644))?;
        let original = snapshot_digest(root.path())?;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755))?;
        assert_ne!(snapshot_digest(root.path())?, original);
        std::os::unix::fs::symlink("program", root.path().join("link"))?;
        assert!(matches!(
            snapshot_digest(root.path()),
            Err(Failure::SourcePathUnsupported)
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tree_metadata_tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
    #[tokio::test]
    async fn tree_preflight_retries_transient_responses_and_stops_on_denial() -> TestResult {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/tree", listener.local_addr()?);
        let tree = "b".repeat(40);
        let body = serde_json::json!({"sha":tree,"truncated":false,"tree":[]}).to_string();
        let server = tokio::spawn(async move {
            for (status, body) in [
                ("503 Service Unavailable", ""),
                ("200 OK", &body),
                ("403 Forbidden", ""),
            ] {
                let (mut stream, _) = listener.accept().await?;
                let request_size = stream.read(&mut [0; 4096]).await?;
                assert!(request_size > 0);
                let response = format!(
                    "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await?;
            }
            Ok::<_, std::io::Error>(())
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;
        fetch_tree(&client, &url, &tree, "fixture").await?;
        assert_eq!(
            fetch_tree(&client, &url, &tree, "fixture").await,
            Err(Failure::SourceAccessDenied)
        );
        server.await??;
        Ok(())
    }
    #[test]
    fn provider_tree_must_be_complete_exact_and_bounded_before_git_fetch() -> TestResult {
        let tree = "b".repeat(40);
        let object = "a".repeat(40);
        let valid = serde_json::json!({"sha":tree,"truncated":false,"tree":[{"path":".ayni.toml","mode":"100644","type":"blob","sha":object,"size":1}]});
        assert!(validate_github_tree(&serde_json::to_vec(&valid)?, &tree).is_ok());
        for (field, value) in [
            ("truncated", serde_json::json!(true)),
            ("sha", serde_json::json!("c".repeat(40))),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(validate_github_tree(&serde_json::to_vec(&invalid)?, &tree).is_err());
        }
        for (field, value) in [
            ("path", serde_json::json!("../private")),
            ("mode", serde_json::json!("120000")),
            ("size", serde_json::json!(MAX_FILE_BYTES + 1)),
        ] {
            let mut invalid = valid.clone();
            invalid["tree"][0][field] = value;
            assert!(validate_github_tree(&serde_json::to_vec(&invalid)?, &tree).is_err());
        }
        Ok(())
    }
}
