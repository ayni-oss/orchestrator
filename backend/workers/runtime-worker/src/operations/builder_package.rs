//! Build the shipped tool recipe without source or certificate authority.
use crate::operations::runtime::command;
use crate::profiles::ayni::{
    build_environment, build_target, capture, image_id, success, wait_engine,
};
use ayni_orchestrator_profile_ayni::ayni_release;
use std::{env, fs, io, path::Path};

/// Bootstrap a reusable Ayni tool image through attempt-local Docker.
/// # Errors
/// Rejects non-native platforms or failed builds.
pub async fn build(workspace: &Path) -> io::Result<()> {
    if env::var("BUILDER_RECIPE_ID").map_err(io::Error::other)?
        != ayni_release::builder_recipe_identity()
    {
        return Err(io::Error::other("builder_recipe_changed"));
    }
    let platform = env::var("TEST_PLATFORM").map_err(io::Error::other)?;
    let native = match env::consts::ARCH {
        "aarch64" => "linux/arm64",
        "x86_64" => "linux/amd64",
        _ => "unsupported",
    };
    if platform != native {
        return Err(io::Error::other("runtime_platform_mismatch"));
    }
    fs::write(workspace.join("Dockerfile"), ayni_release::BUILDER_RECIPE)?;
    fs::write(workspace.join(".dockerignore"), "*\n!Dockerfile\n")?;
    wait_engine(workspace).await?;
    let image = build_target()?;
    let values = build_environment(false)?;
    let mut build = command("docker", workspace);
    build
        .envs(values.iter().map(|(k, v)| (*k, v)))
        .args([
            "build",
            "--platform",
            &platform,
            "--build-arg",
            &format!("AYNI_EXECUTOR_IMAGE={}", ayni_release::EXECUTOR_IMAGE),
            "--build-arg",
            &format!("DOCKER_CLIENT_IMAGE={}", ayni_release::DOCKER_CLIENT_IMAGE),
            "--tag",
            &image,
            ".",
        ])
        .stderr(std::process::Stdio::inherit());
    success(build).await?;
    let mut inspect = command("docker", workspace);
    inspect.envs(values.iter().map(|(k, v)| (*k, v))).args([
        "image",
        "inspect",
        "--format",
        "{{.Id}} {{.Os}}/{{.Architecture}}",
        &image,
    ]);
    let output = capture(inspect).await?;
    let (id, actual) = output
        .trim()
        .split_once(' ')
        .ok_or_else(|| io::Error::other("runtime_build_result_invalid"))?;
    if !image_id(id) || actual != platform {
        return Err(io::Error::other("runtime_platform_mismatch"));
    }
    fs::write("/dev/termination-log", id)
}
