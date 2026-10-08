//! Trusted synthetic repository exercises the same Ayni build and certificate contracts.
use super::{capture, check, command, success};
use ayni_orchestrator_profile_ayni::{
    ayni_release, managed_runtime::RuntimeExpectation, runtime_execution::ExecutionRequest,
};
use serde_json::json;
use std::{env, fs, io, path::Path};

fn fixture(workspace: &Path) -> io::Result<()> {
    let key = env::var("CERTIFICATE_KEY_ID").map_err(io::Error::other)?;
    let public = env::var("CERTIFICATE_PUBLIC_KEY").map_err(io::Error::other)?;
    if !key
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        || key.is_empty()
        || public.len() != 64
        || !public
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(io::Error::other("test_certificate_invalid"));
    }
    fs::create_dir_all(workspace.join("src"))?;
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"builder-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.94\"\n",
    )?;
    fs::write(
        workspace.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"builder-fixture\"\nversion = \"0.1.0\"\n",
    )?;
    fs::write(
        workspace.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.94.0\"\nprofile = \"minimal\"\n",
    )?;
    fs::write(
        workspace.join("src/lib.rs"),
        "pub fn answer() -> u32 { 42 }\n#[test]\nfn computes_answer() { assert_eq!(answer(), 42); }\n",
    )?;
    fs::write(
        workspace.join(".ayni.toml"),
        format!(
            "[languages]\nenabled = [\"rust\"]\n[checks]\ntest = true\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n[environment.certificate.trusted_keys]\n{key} = \"{public}\"\n"
        ),
    )?;
    Ok(())
}
async fn commit(workspace: &Path) -> io::Result<String> {
    for arguments in [
        vec!["init", "--quiet"],
        vec![
            "add",
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "src/lib.rs",
            ".ayni.toml",
            ".ayni.lock",
        ],
        vec![
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "test: trusted builder fixture",
        ],
    ] {
        let mut git = command("git", workspace);
        git.args(arguments)
            .env("GIT_AUTHOR_NAME", "Builder fixture")
            .env("GIT_AUTHOR_EMAIL", "builder@localhost")
            .env("GIT_COMMITTER_NAME", "Builder fixture")
            .env("GIT_COMMITTER_EMAIL", "builder@localhost")
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
        success(git).await?;
    }
    let mut git = command("git", workspace);
    git.args(["rev-parse", "HEAD"]);
    Ok(capture(git).await?.trim().into())
}
/// Build and certify the trusted synthetic repository.
/// # Errors
/// Reports unavailable tools, invalid public identities, or build failures.
pub async fn build_fixture(workspace: &Path) -> io::Result<()> {
    fixture(workspace)?;
    let mut version = command("/usr/local/bin/ayni", workspace);
    version.arg("--version");
    if capture(version).await?.trim() != format!("ayni {}", ayni_release::CLI_VERSION) {
        return Err(io::Error::other("runtime_version_unsupported"));
    }
    super::wait_engine(workspace).await?;
    let base = env::var("PROVISIONING_BASE").map_err(io::Error::other)?;
    let mut lock = command("/usr/local/bin/ayni", workspace);
    lock.args(["env", "lock", "--base", &base]);
    success(lock).await?;
    let lock = fs::read_to_string(workspace.join(".ayni.lock"))?;
    let parsed: serde_json::Value = serde_json::from_str(&lock)?;
    let revision = commit(workspace).await?;
    let request = ExecutionRequest {
        repository_url: "https://github.com/ayni-oss/orchestrator.git".into(),
        revision,
        expected: RuntimeExpectation {
            lock_fingerprint: parsed["fingerprint"]
                .as_str()
                .ok_or_else(|| io::Error::other("test_lock_invalid"))?
                .into(),
            platform: env::var("TEST_PLATFORM").map_err(io::Error::other)?,
            ayni_version: ayni_release::CLI_VERSION.into(),
        },
    };
    super::build_inner(&request, workspace, true).await?;
    println!(
        "AYNI_BUILDER_TEST_RESULT={}",
        json!({"request":request,"lock":lock})
    );
    Ok(())
}
/// Verify the fixture in the independently pulled certified image.
/// # Errors
/// Rejects changed inputs, certificate failures, or failed quality checks.
pub async fn verify_fixture(request: &ExecutionRequest, workspace: &Path) -> io::Result<()> {
    fixture(workspace)?;
    fs::copy("/request/fixture.lock", workspace.join(".ayni.lock"))?;
    if commit(workspace).await? != request.revision {
        return Err(io::Error::other("test_source_mismatch"));
    }
    let report = check(request, workspace).await?;
    if report.check_exit_code != 0 {
        return Err(io::Error::other("runtime_verification_failed"));
    }
    fs::write("/dev/termination-log", "verified")
}

/// Exercise the injected launcher and packaged tools under the build Job contract.
/// # Errors
/// Rejects incorrect UID, missing tools, or unwritable scratch storage.
pub async fn probe(workspace: &Path) -> io::Result<()> {
    let mut identity = command("id", workspace);
    identity.arg("-u");
    if capture(identity).await?.trim() != "10001" {
        return Err(io::Error::other("builder_user_invalid"));
    }
    let mut version = command("/usr/local/bin/ayni", workspace);
    version.arg("--version");
    if capture(version).await?.trim() != format!("ayni {}", ayni_release::CLI_VERSION) {
        return Err(io::Error::other("runtime_version_unsupported"));
    }
    fs::write("/scratch/launcher-probe", "verified")?;
    for (tool, args) in [
        ("/usr/local/bin/ayni", vec!["--version"]),
        ("docker", vec!["--version"]),
        ("docker", vec!["buildx", "version"]),
        ("mise", vec!["--version"]),
        ("git", vec!["--version"]),
    ] {
        let mut probe = command(tool, workspace);
        probe.args(args);
        success(probe).await?;
    }
    Ok(())
}
