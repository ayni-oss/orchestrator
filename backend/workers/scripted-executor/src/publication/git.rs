//! Git plumbing never checks out or executes repository source.
use super::{
    Result,
    config::{Configuration, sha},
};
use ayni_orchestrator_events::{task_execution::digest, task_publication::PublicationCommand};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const MAX_OUTPUT: u64 = 8 * 1024 * 1024;
pub struct Prepared {
    pub commit: String,
    pub tree: String,
    pub root: PathBuf,
}
fn command(root: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("git");
    command
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", "/tmp/publication-home")
        .env("LANG", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "protocol.allow=never",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "push.followTags=false",
            "-c",
            "credential.helper=",
            "-c",
            "http.followRedirects=false",
        ])
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}
async fn execute(mut command: tokio::process::Command, input: Option<&[u8]>) -> Result<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut child = command.spawn().map_err(|_| "publication Git unavailable")?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or("publication Git input unavailable")?;
        if let Some(input) = input {
            stdin
                .write_all(input)
                .await
                .map_err(|_| "publication Git input failed")?;
        }
        drop(stdin);
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or("publication Git output unavailable")?
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut output)
            .await
            .map_err(|_| "publication Git output failed")?;
        if output.len() as u64 > MAX_OUTPUT {
            return Err("publication Git output limit".into());
        }
        if !child
            .wait()
            .await
            .map_err(|_| "publication Git interrupted")?
            .success()
        {
            return Err("publication Git rejected operation".into());
        }
        Ok(output)
    })
    .await
    .map_err(|_| "publication Git timed out")?
}
async fn git(root: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut c = command(root);
    c.args(args);
    execute(c, input).await
}
fn object(bytes: Vec<u8>) -> Result<String> {
    let value = String::from_utf8(bytes)
        .map_err(|_| "publication Git identity invalid")?
        .trim()
        .to_owned();
    if !sha(&value) {
        return Err("publication Git identity invalid".into());
    }
    Ok(value)
}
pub async fn prepare(
    configuration: &Configuration,
    c: &PublicationCommand,
    pack: &[u8],
) -> Result<Prepared> {
    if pack.len() > 8 * 1024 * 1024
        || digest(pack) != c.source_pack_digest
        || digest(c.patch.as_bytes()) != c.patch_digest
    {
        return Err("publication source identity mismatch".into());
    }
    fs::create_dir_all(&configuration.scratch)?;
    let root = configuration.scratch.join(&c.publication_id);
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(&root)?;
    git(&root, &["init", "--initial-branch=publication"], None).await?;
    git(&root, &["unpack-objects", "-q"], Some(pack)).await?;
    let lock = git(
        &root,
        &["show", &format!("{}:.ayni.lock", c.source_commit)],
        None,
    )
    .await?;
    if digest(&lock) != c.lock_digest {
        return Err("publication lock identity mismatch".into());
    }
    git(&root, &["read-tree", &c.source_commit], None).await?;
    git(
        &root,
        &[
            "apply",
            "--cached",
            "--allow-empty",
            "--whitespace=nowarn",
            "-",
        ],
        Some(c.patch.as_bytes()),
    )
    .await?;
    let tree = object(git(&root, &["write-tree"], None).await?)?;
    let base_tree = object(
        git(
            &root,
            &[
                "rev-parse",
                "--verify",
                &format!("{}^{{tree}}", c.source_commit),
            ],
            None,
        )
        .await?,
    )?;
    if tree == base_tree {
        return Err("publication_no_changes".into());
    }
    let mut commit = command(&root);
    let date = format!("@{} +0000", c.commit_timestamp);
    commit
        .env("GIT_AUTHOR_NAME", &c.author_name)
        .env("GIT_AUTHOR_EMAIL", &c.author_email)
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_NAME", &c.author_name)
        .env("GIT_COMMITTER_EMAIL", &c.author_email)
        .env("GIT_COMMITTER_DATE", &date)
        .args(["commit-tree", &tree, "-p", &c.source_commit, "-F", "-"]);
    let commit = object(execute(commit, Some(c.commit_message.as_bytes())).await?)?;
    Ok(Prepared { commit, tree, root })
}
fn remote(root: &Path, c: &Configuration) -> tokio::process::Command {
    let mut command = command(root);
    if c.fixture {
        command.args(["-c", "protocol.git.allow=always"]);
    } else {
        // Credential bytes enter only the trusted Git process environment.
        let authorization = format!(
            "AUTHORIZATION: basic {}",
            STANDARD.encode(format!("x-access-token:{}", c.github_token))
        );
        command
            .args(["-c", "protocol.https.allow=always"])
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraHeader")
            .env("GIT_CONFIG_VALUE_0", authorization);
    }
    command
}
pub async fn head(prepared: &Prepared, c: &Configuration, branch: &str) -> Result<Option<String>> {
    let expected = format!("refs/heads/{branch}");
    let mut command = remote(&prepared.root, c);
    command.args(["ls-remote", "--refs", &c.remote, &expected]);
    let output =
        String::from_utf8(execute(command, None).await?).map_err(|_| "publication ref invalid")?;
    if output.is_empty() {
        return Ok(None);
    }
    let mut lines = output.lines();
    let line = lines.next().ok_or("publication ref missing")?;
    let (commit, name) = line.split_once('\t').ok_or("publication ref invalid")?;
    if !sha(commit) || name != expected || lines.next().is_some() {
        return Err("publication ref invalid".into());
    }
    Ok(Some(commit.into()))
}
pub async fn push(prepared: &Prepared, c: &Configuration, branch: &str) -> Result {
    let reference = format!("refs/heads/{branch}");
    let lease = format!("--force-with-lease={reference}:");
    let target = format!("{}:{reference}", prepared.commit);
    let mut command = remote(&prepared.root, c);
    command.args([
        "push",
        "--porcelain",
        "--no-verify",
        &lease,
        &c.remote,
        &target,
    ]);
    execute(command, None).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(directory: &Path) -> Result<(Configuration, PublicationCommand, Vec<u8>)> {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../dev/fixtures/task-quality");
        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.join("source.json"))?)?;
        let pack = fs::read(fixture.join("source.pack"))?;
        let command = PublicationCommand {
            publication_id: uuid::Uuid::now_v7().to_string(), source_commit: metadata["source_commit"].as_str().ok_or("fixture source")?.into(),
            source_pack_digest: digest(&pack), lock_digest: metadata["lock_digest"].as_str().ok_or("fixture lock")?.into(),
            patch: "diff --git a/publication-result.txt b/publication-result.txt\nnew file mode 100644\n--- /dev/null\n+++ b/publication-result.txt\n@@ -0,0 +1 @@\n+accepted\n".into(),
            author_name: "Ayni Orchestrator".into(), author_email:"orchestrator@ayni.build".into(),
            commit_message: "feat(task): retain accepted changes\n\nSigned-off-by: Ayni Orchestrator <orchestrator@ayni.build>\nai-generated: true\n".into(), commit_timestamp:1_700_000_000, ..Default::default()
        };
        let configuration = Configuration {
            authority: String::new(),
            authority_token: "authority-must-not-reach-git".into(),
            github_token: "github-capability".into(),
            workspace_id: String::new(),
            repository: "https://github.com/fixture/repo.git".into(),
            owner: "fixture".into(),
            name: "repo".into(),
            pod_uid: String::new(),
            api: String::new(),
            remote: String::new(),
            fixture: true,
            scratch: directory.join("scratch"),
        };
        Ok((configuration, command, pack))
    }
    #[tokio::test]
    async fn accepted_patch_produces_deterministic_commit_without_checkout_and_empty_patch_is_blocked()
    -> Result {
        let root = tempfile::tempdir()?;
        let (configuration, mut command, pack) = fixture(root.path())?;
        command.patch_digest = digest(command.patch.as_bytes());
        let first = prepare(&configuration, &command, &pack).await?;
        assert!(!first.root.join("Cargo.toml").exists());
        assert_eq!(
            git(
                &first.root,
                &["show", &format!("{}:publication-result.txt", first.commit)],
                None
            )
            .await?,
            b"accepted\n"
        );
        let second = prepare(&configuration, &command, &pack).await?;
        assert_eq!(first.commit, second.commit);
        assert_eq!(first.tree, second.tree);
        command.patch.clear();
        command.patch_digest = digest(b"");
        assert!(prepare(&configuration, &command, &pack).await.is_err());
        Ok(())
    }
    #[test]
    fn git_credentials_are_environment_only_and_global_configuration_is_disabled() -> Result {
        let root = tempfile::tempdir()?;
        let (mut configuration, _, _) = fixture(root.path())?;
        configuration.fixture = false;
        let command = remote(root.path(), &configuration);
        assert!(
            command
                .as_std()
                .get_args()
                .all(|v| !v.to_string_lossy().contains(&configuration.github_token))
        );
        let environment = command.as_std().get_envs().collect::<Vec<_>>();
        assert!(
            environment
                .iter()
                .any(|(k, v)| *k == "GIT_CONFIG_NOSYSTEM" && *v == Some(std::ffi::OsStr::new("1")))
        );
        assert!(environment.iter().all(|(_, v)| v.is_none_or(|value| {
            !value
                .to_string_lossy()
                .contains(&configuration.authority_token)
        })));
        Ok(())
    }
    async fn daemon(root: &Path) -> Result<(tokio::process::Child, u16)> {
        git(root, &["init", "--bare", "repo.git"], None).await?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let child = tokio::process::Command::new("git")
            .args([
                "daemon",
                "--reuseaddr",
                "--export-all",
                "--enable=receive-pack",
                "--listen=127.0.0.1",
                &format!("--port={port}"),
                &format!("--base-path={}", root.display()),
            ])
            .arg(root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        for _ in 0..40 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                return Ok((child, port));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err("fixture Git daemon unavailable".into())
    }
    #[tokio::test]
    #[ignore = "requires loopback networking; run scripts/ci/check-workflow-loopback.sh"]
    async fn create_only_push_recovers_same_commit_and_never_overwrites_another_head_requires_loopback()
    -> Result {
        let root = tempfile::tempdir()?;
        let remote_root = tempfile::tempdir()?;
        let (mut configuration, mut command, pack) = fixture(root.path())?;
        let (_daemon, port) = daemon(remote_root.path()).await?;
        configuration.remote = format!("git://127.0.0.1:{port}/repo.git");
        command.patch_digest = digest(command.patch.as_bytes());
        let first = prepare(&configuration, &command, &pack).await?;
        push(&first, &configuration, "task/owned").await?;
        assert_eq!(
            head(&first, &configuration, "task/owned").await?.as_deref(),
            Some(first.commit.as_str())
        );
        push(&first, &configuration, "task/owned").await?;
        command.patch = command.patch.replace("+accepted", "+different");
        command.patch_digest = digest(command.patch.as_bytes());
        let different = prepare(&configuration, &command, &pack).await?;
        assert!(
            push(&different, &configuration, "task/owned")
                .await
                .is_err()
        );
        assert_eq!(
            head(&different, &configuration, "task/owned")
                .await?
                .as_deref(),
            Some(first.commit.as_str())
        );
        Ok(())
    }
}
