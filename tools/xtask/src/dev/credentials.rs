//! Provision immutable local role sets without reading existing secret payloads.
use super::{NAMESPACE, kube, process};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, process::Command};

fn random() -> Result<String> {
    process::output(Command::new("openssl").args(["rand", "-hex", "32"]))
}
fn exists(name: &str) -> Result<bool> {
    Ok(
        !process::output(kube().args(["get", "secret", name, "--ignore-not-found", "-o", "name"]))?
            .is_empty(),
    )
}
fn complete(names: &[&str]) -> Result<bool> {
    let mut present = 0;
    for name in names {
        present += usize::from(exists(name)?);
    }
    ensure!(
        present == 0 || present == names.len(),
        "immutable credential set is incomplete; restore its missing Secrets before development (no data reset is performed)"
    );
    Ok(present == names.len())
}
fn create(name: &str, data: Value) -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let path = scratch.path().join("secret.json");
    let mut value = json!({"apiVersion":"v1","kind":"Secret","metadata":{"name":name,"namespace":NAMESPACE},"immutable":true,"type":"Opaque"});
    value["stringData"] = data;
    fs::write(&path, serde_json::to_vec(&value)?)?;
    process::run(kube().args(["create", "-f"]).arg(path))
}
pub fn ensure_local() -> Result<()> {
    // The canonical retained namespace must already exist. Startup never discovers,
    // deletes, or adopts another environment.
    let namespace: Value = serde_json::from_str(&process::output(kube().args([
        "get",
        "namespace",
        NAMESPACE,
        "-o",
        "json",
    ]))?)?;
    ensure!(
        namespace["metadata"]["labels"]["orchestrator.ayni.dev/environment"] == "local",
        "canonical namespace lacks the expected local ownership label"
    );
    if !complete(&[
        "workflow-postgres-admin",
        "workflow-postgres-migrator",
        "workflow-postgres-runtime",
    ])? {
        // Missing credentials on a retained database require recovery, never new passwords.
        let volumes: Value =
            serde_json::from_str(&process::output(kube().args(["get", "pvc", "-o", "json"]))?)?;
        ensure!(
            volumes["items"].as_array().is_some_and(Vec::is_empty),
            "database credentials missing while retained storage exists; restore the original Secrets"
        );
        create(
            "workflow-postgres-admin",
            json!({"username":"ayni_admin","password":random()?,"database":"ayni_workflow"}),
        )?;
        for (name, username) in [
            ("workflow-postgres-migrator", "ayni_migrator"),
            ("workflow-postgres-runtime", "ayni_runtime"),
        ] {
            let password = random()?;
            create(
                name,
                json!({"username":username,"password":password,"url":format!("postgresql://{username}:{password}@postgres.{NAMESPACE}.svc.cluster.local:5432/ayni_workflow")}),
            )?;
        }
    }
    if !complete(&[
        "workflow-jetstream-admin",
        "workflow-jetstream-application",
        "workflow-jetstream-executor",
        "workflow-jetstream-acceptance",
    ])? {
        for name in [
            "workflow-jetstream-admin",
            "workflow-jetstream-application",
            "workflow-jetstream-acceptance",
        ] {
            create(name, json!({"password":random()?}))?;
        }
        create(
            "workflow-jetstream-executor",
            json!({"password":random()?,"instance-id":"018f0f7b-6a4d-7a11-8d77-010203040506","signing-seed-hex":random()?}),
        )?;
    }
    for name in [
        "workflow-session-authority",
        "workflow-publication-authority",
        "workflow-runtime-authority",
        "workflow-telemetry-authority",
    ] {
        if !exists(name)? {
            create(name, json!({"token":random()?}))?;
        }
    }
    Ok(())
}
