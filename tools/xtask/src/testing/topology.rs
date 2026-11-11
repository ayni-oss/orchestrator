use super::deployment::kube;
use anyhow::{Context as _, Result, ensure};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    value[key]
        .as_array()
        .with_context(|| format!("missing {key}"))
}
fn named<'a>(values: &'a [Value], name: &str) -> Result<&'a Value> {
    values
        .iter()
        .find(|value| value["name"] == name)
        .with_context(|| format!("missing {name}"))
}
fn env(container: &Value) -> Result<BTreeMap<String, String>> {
    array(container, "env")?
        .iter()
        .map(|e| {
            Ok((
                e["name"]
                    .as_str()
                    .context("environment name missing")?
                    .into(),
                e["value"].as_str().unwrap_or_default().into(),
            ))
        })
        .collect()
}
fn secrets(pod: &Value, container: &Value) -> Result<BTreeSet<String>> {
    let volumes = array(&pod["spec"], "volumes")?;
    let mut secrets = BTreeSet::new();
    for mount in array(container, "volumeMounts")? {
        let volume = named(
            volumes,
            mount["name"].as_str().context("mount name missing")?,
        )?;
        if let Some(secret) = volume["secret"]["secretName"].as_str() {
            secrets.insert(secret.into());
        }
    }
    Ok(secrets)
}
fn isolated(pod: &Value) -> Result<()> {
    let spec = &pod["spec"];
    ensure!(
        spec["automountServiceAccountToken"] == false
            && spec["securityContext"]["runAsNonRoot"] == true,
        "workload isolation missing"
    );
    ensure!(
        array(spec, "volumes")?
            .iter()
            .all(|v| v.get("hostPath").is_none() && v.get("projected").is_none()),
        "host or service account volume exposed"
    );
    Ok(())
}
pub fn run() -> Result<()> {
    let sessions = format!("{}-sessions", crate::dev::NAMESPACE);
    let namespace = kube(&["get", "namespace", &sessions, "-o", "json"])?;
    ensure!(
        namespace["metadata"]["labels"]["build.ayni/parent-namespace"] == crate::dev::NAMESPACE
            && namespace["metadata"]["annotations"]["helm.sh/resource-policy"] == "keep",
        "session namespace ownership mismatch"
    );
    let parent = kube(&["get", "pods", "-o", "json"])?;
    let workers = kube(&["get", "pods", "-n", &sessions, "-o", "json"])?;
    sessions_check(array(&workers, "items")?)?;
    tasks_check(array(&parent, "items")?, array(&workers, "items")?)?;
    let identity = format!(
        "system:serviceaccount:{}:conversation-session-supervisor",
        crate::dev::NAMESPACE
    );
    for (ns, expected) in [(crate::dev::NAMESPACE, "no"), (sessions.as_str(), "yes")] {
        let output = crate::dev::kube()
            .args([
                "auth", "can-i", "get", "secrets", "--as", &identity, "-n", ns,
            ])
            .output()?;
        ensure!(
            String::from_utf8(output.stdout)?.trim() == expected,
            "supervisor Secret RBAC mismatch"
        );
    }
    println!(
        "Session/task/publisher isolation passed; Docker Desktop NetworkPolicy enforcement remains unqualified"
    );
    Ok(())
}
fn sessions_check(pods: &[Value]) -> Result<()> {
    let mut conversations = BTreeMap::new();
    let mut claims = BTreeSet::new();
    for pod in pods.iter().filter(|p| {
        p["metadata"]["labels"]["app.kubernetes.io/name"] == "ayni-conversation-session"
    }) {
        isolated(pod)?;
        let containers = array(&pod["spec"], "containers")?;
        ensure!(containers.len() == 1, "session must have one worker");
        let container = &containers[0];
        ensure!(
            container["args"] == serde_json::json!(["session-worker"]),
            "unexpected session executable"
        );
        hardened(container)?;
        let env = env(container)?;
        ensure!(
            env.keys()
                .all(|k| !k.contains("BROKER") && !k.contains("DATABASE")),
            "session has shared authority"
        );
        let scope = env
            .get("ORCHESTRATOR_SESSION_ID")
            .context("session scope missing")?;
        let generation = env
            .get("ORCHESTRATOR_SESSION_GENERATION")
            .context("generation missing")?;
        ensure!(
            pod["metadata"]["labels"]["build.ayni/session-id"] == *scope
                && pod["metadata"]["labels"]["build.ayni/session-generation"] == *generation,
            "session labels are not fenced"
        );
        let conversation = env
            .get("ORCHESTRATOR_SESSION_CONVERSATION_ID")
            .context("conversation missing")?;
        ensure!(
            conversations
                .insert(conversation.clone(), scope.clone())
                .is_none_or(|prior| prior == *scope),
            "conversation scope changed"
        );
        let volumes = array(&pod["spec"], "volumes")?;
        ensure!(
            (2..=3).contains(&volumes.len()),
            "unexpected session mounts"
        );
        let claim = volumes
            .iter()
            .find_map(|v| v["persistentVolumeClaim"]["claimName"].as_str())
            .context("session claim missing")?;
        ensure!(
            pod["metadata"]["name"] == claim
                && named(volumes, "token")?["secret"]["secretName"] == claim
                && claims.insert(claim),
            "session claims or capabilities overlap"
        );
        provider(container, volumes, &env)?;
    }
    ensure!(
        conversations.len() >= 2,
        "two retained conversation fixtures are required"
    );
    ensure!(
        conversations.values().collect::<BTreeSet<_>>().len() == conversations.len(),
        "conversation sessions overlap"
    );
    Ok(())
}
fn hardened(container: &Value) -> Result<()> {
    ensure!(
        container["securityContext"]["readOnlyRootFilesystem"] == true
            && container["securityContext"]["allowPrivilegeEscalation"] == false,
        "container lacks isolation"
    );
    ensure!(
        array(container, "env")?
            .iter()
            .all(|e| e["valueFrom"].get("secretKeyRef").is_none()),
        "credential environment exposed"
    );
    Ok(())
}
fn provider(container: &Value, volumes: &[Value], env: &BTreeMap<String, String>) -> Result<()> {
    if let Ok(provider) = named(volumes, "provider") {
        let workspace = env
            .get("ORCHESTRATOR_SESSION_WORKSPACE_ID")
            .context("workspace missing")?
            .replace('-', "");
        ensure!(
            provider["secret"]["secretName"] == format!("provider-openai-{workspace}")
                && provider["secret"]["optional"] == true
                && provider["secret"]["items"]
                    == serde_json::json!([{"key":"api-key","path":"api-key"}]),
            "provider credential scope mismatch"
        );
        ensure!(
            env.get("ORCHESTRATOR_OPENAI_API_KEY_PATH")
                .map(String::as_str)
                == Some("/var/run/provider/api-key"),
            "provider path mismatch"
        );
        let mount = named(array(container, "volumeMounts")?, "provider")?;
        ensure!(
            mount["mountPath"] == "/var/run/provider" && mount["readOnly"] == true,
            "provider mount writable"
        );
    } else {
        ensure!(
            !env.contains_key("ORCHESTRATOR_OPENAI_API_KEY_PATH"),
            "provider path without mount"
        );
    }
    Ok(())
}
fn tasks_check(parent: &[Value], workers: &[Value]) -> Result<()> {
    let publishers: Vec<_> = parent
        .iter()
        .filter(|p| p["metadata"]["labels"]["app.kubernetes.io/name"] == "ayni-task-publisher")
        .collect();
    ensure!(
        publishers.len() == 1,
        "one retained publisher fixture required"
    );
    let publisher = publishers[0];
    ensure!(
        publisher["spec"]["automountServiceAccountToken"] == false,
        "publisher service account exposed"
    );
    let volumes = array(&publisher["spec"], "volumes")?;
    ensure!(
        volumes.len() == 3 && named(volumes, "scratch")?.get("emptyDir").is_some(),
        "publisher mounts changed"
    );
    let github = named(volumes, "github")?["secret"]["secretName"]
        .as_str()
        .context("publisher GitHub secret missing")?;
    ensure!(
        named(volumes, "authority")?["secret"]["secretName"] != github,
        "publisher credentials overlap"
    );
    let mut claims = BTreeSet::new();
    for pod in workers
        .iter()
        .filter(|p| p["metadata"]["labels"]["app.kubernetes.io/name"] == "ayni-task-worker")
    {
        task(pod, &mut claims)?;
    }
    ensure!(
        claims.len() >= 2,
        "passing and failing retained managed task fixtures required"
    );
    for pod in parent.iter().chain(workers).filter(|p| *p != publisher) {
        for container in containers(pod) {
            ensure!(
                !secrets(pod, container)?.contains(github),
                "GitHub credential escaped publisher"
            );
        }
    }
    Ok(())
}
fn containers(pod: &Value) -> impl Iterator<Item = &Value> {
    pod["spec"]["initContainers"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(pod["spec"]["containers"].as_array().into_iter().flatten())
}
fn task(pod: &Value, claims: &mut BTreeSet<String>) -> Result<()> {
    isolated(pod)?;
    ensure!(
        pod["metadata"]["annotations"]["build.ayni/command-digest"]
            .as_str()
            .is_some_and(|d| d.starts_with("sha256:")),
        "task command fence missing"
    );
    let volumes = array(&pod["spec"], "volumes")?;
    let claim = named(volumes, "work")?["persistentVolumeClaim"]["claimName"]
        .as_str()
        .context("task claim missing")?;
    ensure!(
        pod["metadata"]["name"] == claim
            && named(volumes, "capability")?["secret"]["secretName"] == claim
            && claims.insert(claim.into()),
        "task claim or capability overlap"
    );
    let stages: Vec<_> = containers(pod).cloned().collect();
    ensure!(
        stages
            .iter()
            .filter_map(|c| c["name"].as_str())
            .collect::<BTreeSet<_>>()
            == BTreeSet::from([
                "initialize",
                "coding",
                "freeze",
                "prepare",
                "seal",
                "quality",
                "collector"
            ]),
        "task stages changed"
    );
    for stage in &stages {
        hardened(stage)?;
        if stage["name"] != "coding" {
            ensure!(
                array(stage, "volumeMounts")?
                    .iter()
                    .all(|m| m["name"] != "provider"),
                "provider exposed outside coding"
            );
        }
    }
    for name in ["prepare", "quality"] {
        let stage = named(&stages, name)?;
        ensure!(
            secrets(pod, stage)?.is_empty()
                && env(stage)?
                    .keys()
                    .all(|e| !e.starts_with("ORCHESTRATOR_") && !e.starts_with("OPENAI_")),
            "isolated verification has credentials"
        );
        ensure!(
            array(&stage["securityContext"]["capabilities"], "add")?
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
                == BTreeSet::from(["SETUID", "SETGID"]),
            "verification capabilities changed"
        );
    }
    let quality = named(&stages, "quality")?;
    ensure!(
        quality["image"]
            .as_str()
            .is_some_and(|i| i.contains("@sha256:")),
        "quality runtime is not pinned"
    );
    let mounts = array(quality, "volumeMounts")?;
    ensure!(
        mounts.iter().any(|m| m["mountPath"] == "/workspace"
            && m["readOnly"] == true
            && m["subPath"] == "quality/source")
            && mounts.iter().any(
                |m| m["mountPath"] == "/workspace/.ayni" && m["subPath"] == "quality/artifacts"
            ),
        "quality source/output separation lost"
    );
    ensure!(
        array(named(&stages, "prepare")?, "volumeMounts")?
            .iter()
            .any(|m| m["mountPath"] == "/workspace/.git" && m["readOnly"] == true),
        "prepare can mutate frozen Git source"
    );
    Ok(())
}
