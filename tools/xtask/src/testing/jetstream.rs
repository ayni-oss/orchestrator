use crate::command;
use anyhow::{Context as _, Result, ensure};
use regex::Regex;
use serde_json::Value;
use std::fmt::Write as _;
use std::{collections::BTreeMap, fs, path::Path, process::Command, thread, time::Duration};
const IMAGE: &str =
    "nats:2.15.0-alpine@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4";
const INSTANCE: &str = "018f0f7b-6a4d-7a11-8d77-010203040506";
const LOSER: &str = "018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6";
struct Container(String);
impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "--force", &self.0])
            .output();
    }
}

pub fn run(integrated: bool) -> Result<()> {
    let directory = tempfile::tempdir()?;
    configuration(directory.path())?;
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))?
        .local_addr()?
        .port();
    let name = format!("ayni-m1-jetstream-test-{}", uuid::Uuid::new_v4().simple());
    command::run(
        "docker",
        &[
            "run",
            "--detach",
            "--rm",
            "--name",
            &name,
            "--mount",
            &format!(
                "type=bind,source={},target=/fixture,readonly",
                directory.path().display()
            ),
            "--publish",
            &format!("127.0.0.1:{port}:4222"),
            IMAGE,
            "--config",
            "/fixture/nats.conf",
        ],
    )?;
    let _container = Container(name.clone());
    ready(&name)?;
    let address = command::output("docker", &["port", &name, "4222/tcp"])?;
    ensure!(
        address.trim().starts_with("127.0.0.1:")
            && address.trim().parse::<std::net::SocketAddr>().is_ok(),
        "broker fixture must bind loopback"
    );
    let mut env = BTreeMap::from([
        ("M1_FIXTURE_NATS_CONTAINER", name),
        (
            "M1_TRANSPORT_FIXTURE_DIRECTORY",
            directory.path().display().to_string(),
        ),
    ]);
    for (identity, user, password) in [
        ("ADMIN", "workflow-admin", "fixture-admin"),
        ("APPLICATION", "workflow-application", "fixture-application"),
        ("WINNER", "workflow-executor", "fixture-executor"),
        ("LOSER", "fixture-loser", "fixture-loser"),
    ] {
        // Keys are owned below to avoid mutating the process environment used by other tests.
        for (suffix, value) in [
            ("URL", format!("nats://{}", address.trim())),
            ("USER", user.into()),
            ("PASSWORD", password.into()),
        ] {
            let key = match (identity, suffix) {
                ("ADMIN", "URL") => "M1_ADMIN_NATS_URL",
                ("ADMIN", "USER") => "M1_ADMIN_NATS_USER",
                ("ADMIN", _) => "M1_ADMIN_NATS_PASSWORD",
                ("APPLICATION", "URL") => "M1_APPLICATION_NATS_URL",
                ("APPLICATION", "USER") => "M1_APPLICATION_NATS_USER",
                ("APPLICATION", _) => "M1_APPLICATION_NATS_PASSWORD",
                ("WINNER", "URL") => "M1_WINNER_NATS_URL",
                ("WINNER", "USER") => "M1_WINNER_NATS_USER",
                ("WINNER", _) => "M1_WINNER_NATS_PASSWORD",
                (_, "URL") => "M1_LOSER_NATS_URL",
                (_, "USER") => "M1_LOSER_NATS_USER",
                _ => "M1_LOSER_NATS_PASSWORD",
            };
            env.insert(key, value);
        }
    }
    transport(&env, "disposable_profile_setup")?;
    transport(&env, "diagnostic_stream_replays")?;
    if integrated {
        super::postgres::with_fixture(|postgres| {
            let mut combined = env.clone();
            combined.extend(postgres);
            cargo(
                &combined,
                &[
                    "-p",
                    "ayni-orchestrator-workflow-acceptance",
                    "--test",
                    "durable_handoff",
                ],
            )
        })?;
        transport(&env, "disposable_profile_setup")?;
    }
    cargo(
        &env,
        &[
            "-p",
            "ayni-orchestrator-conversation-supervisor",
            "authenticated_jetstream_redelivery_preserves_ledger_ownership",
        ],
    )?;
    transport(&env, "profile_authorization_capacity_and_payload")?;
    Ok(())
}
fn cargo(env: &BTreeMap<&str, String>, args: &[&str]) -> Result<()> {
    ensure!(
        Command::new("cargo")
            .args(["test", "--locked"])
            .args(args)
            .args(["--", "--ignored", "--nocapture", "--test-threads=1"])
            .envs(env)
            .status()?
            .success(),
        "broker acceptance failed"
    );
    Ok(())
}
fn transport(env: &BTreeMap<&str, String>, test: &str) -> Result<()> {
    cargo(
        env,
        &[
            "-p",
            "ayni-orchestrator-control-plane",
            "--test",
            "workflow_transport",
            test,
        ],
    )
}
fn ready(name: &str) -> Result<()> {
    for _ in 0..60 {
        if Command::new("docker")
            .args([
                "exec",
                name,
                "wget",
                "-q",
                "-O",
                "/dev/null",
                "http://127.0.0.1:8222/healthz",
            ])
            .output()?
            .status
            .success()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
    anyhow::bail!("broker fixture readiness failed")
}
fn configuration(root: &Path) -> Result<()> {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../dev/charts/ayni-orchestrator-local/templates/jetstream.yaml"),
    )?;
    let header = Regex::new(r"^  ([a-z-]+\.(?:json|conf)): \|$")?;
    let mut blocks = BTreeMap::<String, String>::new();
    let mut current = None;
    for line in source.lines() {
        if let Some(capture) = header.captures(line) {
            current = Some(capture[1].to_owned());
        } else if let Some(body) = line.strip_prefix("    ") {
            if let Some(name) = &current {
                writeln!(blocks.entry(name.clone()).or_default(), "{body}")?;
            }
        } else if !line.is_empty() {
            current = None;
        }
    }
    ensure!(
        blocks.contains_key("nats.conf") && blocks.contains_key("executor-decision-consumer.json"),
        "broker configuration missing"
    );
    for (name, body) in blocks {
        let mut body = body
            .replace("{{ int64 .Values.jetstream.maxFileBytes }}", "1073741824")
            .replace("{{ .Values.jetstream.executorInstanceId }}", INSTANCE);
        if name == "nats.conf" {
            for (role, password) in [
                ("ADMIN", "admin"),
                ("APPLICATION", "application"),
                ("EXECUTOR", "executor"),
                ("ACCEPTANCE", "acceptance"),
            ] {
                body = body.replace(
                    &format!("$WORKFLOW_{role}_PASSWORD"),
                    &format!("fixture-{password}"),
                );
            }
            let start = body
                .find("    {\n      user: \"workflow-executor\"")
                .context("executor ACL missing")?;
            let end = body
                .find("    {\n      user: \"workflow-acceptance\"")
                .context("acceptance ACL missing")?;
            let loser = body[start..end]
                .replace("workflow-executor", "fixture-loser")
                .replace("fixture-executor", "fixture-loser")
                .replace(INSTANCE, LOSER);
            body.insert_str(end, &loser);
        } else {
            let mut value: Value = serde_json::from_str(&body)?;
            if ["command.json", "report.json", "decision.json"].contains(&name.as_str()) {
                value["max_age"] = 2_000_000_000_u64.into();
                value["duplicate_window"] = 1_000_000_000_u64.into();
            } else if name.contains("consumer") {
                value["ack_wait"] = 1_000_000_000_u64.into();
            }
            body = serde_json::to_string(&value)?;
            if name == "executor-decision-consumer.json" {
                fs::write(
                    root.join("loser-consumer.json"),
                    body.replace(INSTANCE, LOSER),
                )?;
            }
        }
        fs::write(root.join(name), body)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_broker_acl_and_caps_are_retained() -> Result<()> {
        let root = tempfile::tempdir()?;
        configuration(root.path())?;
        let config = fs::read_to_string(root.path().join("nats.conf"))?;
        assert!(config.contains(LOSER) && config.contains(INSTANCE));
        let stream: Value = serde_json::from_slice(&fs::read(root.path().join("command.json"))?)?;
        assert_eq!(stream["storage"], "file");
        assert_eq!(stream["max_age"], 2_000_000_000_u64);
        assert!(stream["max_bytes"].as_i64().is_some_and(|v| v > 0));
        Ok(())
    }
}
