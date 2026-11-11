use crate::command;
use anyhow::{Result, ensure};
use std::{collections::BTreeMap, process::Command, time::Duration};

const IMAGE: &str =
    "postgres@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3";
const PASSWORD: &str = "feasibility-only";
const DATABASE: &str = "workflow_gate";
const ROLE: &str = "ayni_gate_runtime";
const RUNTIME_PASSWORD: &str = "runtime-feasibility-only";

struct Fixture(String);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "--force", &self.0])
            .output();
    }
}

pub fn run(suite: Option<&str>) -> Result<()> {
    let name = format!("ayni-postgres-test-{}", uuid::Uuid::new_v4().simple());
    command::run(
        "docker",
        &[
            "run",
            "--detach",
            "--rm",
            "--name",
            &name,
            "--env",
            &format!("POSTGRES_PASSWORD={PASSWORD}"),
            "--env",
            &format!("POSTGRES_DB={DATABASE}"),
            "--publish",
            "127.0.0.1::5432",
            IMAGE,
        ],
    )?;
    let _fixture = Fixture(name.clone());
    let mut ready = false;
    for _ in 0..60 {
        if Command::new("docker")
            .args([
                "exec",
                &name,
                "pg_isready",
                "--host",
                "127.0.0.1",
                "--username",
                "postgres",
                "--dbname",
                DATABASE,
            ])
            .output()?
            .status
            .success()
        {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    ensure!(ready, "PostgreSQL fixture readiness failed");
    let address = command::output("docker", &["port", &name, "5432/tcp"])?;
    let address = address.trim();
    ensure!(
        address.starts_with("127.0.0.1:") && address.parse::<std::net::SocketAddr>().is_ok(),
        "PostgreSQL fixture must bind only loopback"
    );
    let sql = format!(
        "CREATE ROLE {ROLE} LOGIN PASSWORD '{RUNTIME_PASSWORD}'; REVOKE ALL ON DATABASE {DATABASE} FROM PUBLIC; GRANT CONNECT ON DATABASE {DATABASE} TO {ROLE}"
    );
    command::input(
        "docker",
        &[
            "exec",
            "-i",
            &name,
            "psql",
            "--username",
            "postgres",
            "--dbname",
            DATABASE,
            "-v",
            "ON_ERROR_STOP=1",
        ],
        sql.as_bytes(),
    )?;
    let env = BTreeMap::from([
        ("M1_FIXTURE_POSTGRES_CONTAINER", name),
        (
            "AYNI_POSTGRES_FEASIBILITY_URL",
            format!("postgresql://postgres:{PASSWORD}@{address}/{DATABASE}"),
        ),
        (
            "AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL",
            format!("postgresql://{ROLE}:{RUNTIME_PASSWORD}@{address}/{DATABASE}"),
        ),
        ("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE", ROLE.into()),
    ]);
    run_suites(env, suite)
}

fn run_suites(env: BTreeMap<&str, String>, suite: Option<&str>) -> Result<()> {
    let suites = [
        "feasibility",
        "logs",
        "plans",
        "task_runs",
        "publications",
        "task_execution",
        "plan_approval",
        "plan_proposals",
        "project_continuity",
        "connections",
        "repositories",
        "repository_inspections",
        "runtime_signers",
        "runtime_signer_provisioning",
        "runtime_preparations",
        "configuration",
        "sessions",
        "provider_calls",
        "baseline_rejection",
        "transport_retention",
        "execution_timeline",
        "watch_feasibility",
    ];
    if let Some(suite) = suite {
        ensure!(
            suites.contains(&suite),
            "unknown PostgreSQL test suite: {suite}"
        );
        cargo(&env, "ayni-orchestrator-workflow-postgres", suite, true)?;
    } else {
        for suite in suites {
            cargo(&env, "ayni-orchestrator-workflow-postgres", suite, true)?;
        }
        let mut runtime = env;
        runtime.insert(
            "AYNI_POSTGRES_FEASIBILITY_URL",
            runtime["AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL"].clone(),
        );
        cargo(
            &runtime,
            "ayni-orchestrator-control-plane",
            "workflow_api",
            false,
        )?;
        cargo(
            &runtime,
            "ayni-orchestrator-control-plane",
            "workflow_api",
            true,
        )?;
    }
    Ok(())
}

fn cargo(env: &BTreeMap<&str, String>, package: &str, suite: &str, ignored: bool) -> Result<()> {
    let mut command = Command::new("cargo");
    command
        .args([
            "test",
            "--locked",
            "-p",
            package,
            "--test",
            suite,
            "--",
            "--nocapture",
            "--test-threads=1",
        ])
        .envs(env);
    if ignored {
        command.arg("--ignored");
    }
    ensure!(
        command.status()?.success(),
        "PostgreSQL test suite failed: {suite}"
    );
    Ok(())
}
