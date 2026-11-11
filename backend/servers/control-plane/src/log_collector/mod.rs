//! API-only collection. Kubernetes cannot prove complete log history after rotation.
mod tail;
use ayni_orchestrator_api::v1::{LogSource, TelemetryRecord};
use ayni_orchestrator_execution::{Spool, now_ms};
use futures_util::TryStreamExt as _;
use k8s_openapi::api::core::v1::{Event, Pod};
use kube::{
    Api, Client,
    api::{ListParams, WatchEvent, WatchParams},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, io,
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle};

fn observed(spool: &Spool, source: &LogSource, event: &str, data: Vec<u8>) -> io::Result<()> {
    spool.append(TelemetryRecord {
        source: Some(source.clone()),
        event: event.into(),
        data,
        observed_at_unix_ms: now_ms(),
        ..Default::default()
    })
}

fn owned(pod: &Pod, deployment: &str) -> bool {
    let Some(labels) = pod.metadata.labels.as_ref() else {
        return false;
    };
    if let Some(owner) = labels.get("ayni.build/deployment") {
        return owner == deployment;
    }
    labels
        .get("app.kubernetes.io/part-of")
        .is_some_and(|v| v == "ayni-orchestrator")
        || labels.get("app.kubernetes.io/managed-by").is_some_and(|v| {
            matches!(
                v.as_str(),
                "ayni-runtime-supervisor" | "ayni-conversation-supervisor" | "ayni-task-supervisor"
            )
        })
}

fn sources(pod: &Pod, deployment: &str, namespace: &str) -> Vec<LogSource> {
    let Some(spec) = &pod.spec else {
        return Vec::new();
    };
    let labels = pod.metadata.labels.clone().unwrap_or_default();
    let value = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| labels.get(*name))
            .cloned()
            .unwrap_or_default()
    };
    let mut result = Vec::new();
    let names = spec
        .containers
        .iter()
        .map(|c| c.name.clone())
        .chain(
            spec.init_containers
                .iter()
                .flatten()
                .map(|c| c.name.clone()),
        )
        .chain(
            spec.ephemeral_containers
                .iter()
                .flatten()
                .map(|c| c.name.clone()),
        );
    for container in names {
        let status = pod
            .status
            .iter()
            .flat_map(|s| {
                s.container_statuses
                    .iter()
                    .flatten()
                    .chain(s.init_container_statuses.iter().flatten())
                    .chain(s.ephemeral_container_statuses.iter().flatten())
            })
            .find(|s| s.name == container);
        let restart = status.map_or(0, |s| u32::try_from(s.restart_count).unwrap_or_default());
        let uid = pod.metadata.uid.clone().unwrap_or_default();
        let state = if pod.metadata.deletion_timestamp.is_some() {
            "deleted_or_retiring"
        } else if status.is_some_and(|s| s.state.as_ref().is_some_and(|s| s.waiting.is_some())) {
            "waiting"
        } else if status.is_some_and(|s| s.state.as_ref().is_some_and(|s| s.terminated.is_some())) {
            "terminated_completeness_unknown"
        } else {
            "completeness_unknown"
        };
        result.push(LogSource {
            id: format!("pod:{uid}:{container}:{restart}"),
            deployment: deployment.into(),
            namespace: namespace.into(),
            pod_name: pod.metadata.name.clone().unwrap_or_default(),
            pod_uid: uid,
            container,
            restart,
            component: value(&["app.kubernetes.io/name", "app.kubernetes.io/managed-by"]),
            execution_id: value(&[
                "ayni.build/operation",
                "ayni.build/attempt",
                "build.ayni/attempt",
                "build.ayni/attempt-id",
                "build.ayni/session-id",
            ]),
            generation: value(&["ayni.build/generation", "build.ayni/generation"])
                .parse()
                .unwrap_or_default(),
            stage: value(&["ayni.build/stage"]),
            repository_id: value(&["ayni.build/repository"]),
            project_id: value(&["ayni.build/project"]),
            provenance: "kubernetes_api".into(),
            capture_status: state.into(),
            observed_at_unix_ms: now_ms(),
        });
    }
    result
}

async fn reconcile(
    client: &Client,
    namespace: &str,
    deployment: &str,
    spool: &Arc<Spool>,
    tails: &mut BTreeMap<String, JoinHandle<()>>,
    known: &mut BTreeMap<String, LogSource>,
) -> io::Result<()> {
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pods = api
        .list(&ListParams::default())
        .await
        .map_err(io::Error::other)?;
    let mut active = BTreeSet::new();
    for pod in pods.items.iter().filter(|pod| owned(pod, deployment)) {
        for source in sources(pod, deployment, namespace) {
            let id = source.id.clone();
            active.insert(id.clone());
            if known
                .get(&id)
                .is_none_or(|old| old.capture_status != source.capture_status)
            {
                observed(spool, &source, "container_observed", Vec::new())?;
            }
            known.insert(id.clone(), source.clone());
            if tails.get(&id).is_none_or(JoinHandle::is_finished)
                && spool.checkpoint(&format!("finished:{id}"))?.is_none()
                && tails.values().filter(|t| !t.is_finished()).count() < 128
            {
                let api = api.clone();
                let spool = spool.clone();
                let source = source.clone();
                tails.insert(
                    id.clone(),
                    tokio::spawn(async move {
                        let _ = tail::collect(api, spool, source, false).await;
                    }),
                );
            }
            if source.restart > 0 {
                let mut previous = source.clone();
                previous.restart -= 1;
                previous.id = format!(
                    "pod:{}:{}:{}",
                    previous.pod_uid, previous.container, previous.restart
                );
                active.insert(previous.id.clone());
                if spool
                    .checkpoint(&format!("previous:{}", previous.id))?
                    .is_none()
                    && tails.get(&previous.id).is_none_or(JoinHandle::is_finished)
                    && tails.values().filter(|t| !t.is_finished()).count() < 128
                {
                    let api = api.clone();
                    let spool = spool.clone();
                    let id = previous.id.clone();
                    tails.insert(
                        id,
                        tokio::spawn(async move {
                            let _ = tail::collect(api, spool, previous, true).await;
                        }),
                    );
                }
            }
        }
    }
    for (id, source) in known.iter_mut().filter(|(_, s)| s.namespace == namespace) {
        if !active.contains(id) && source.capture_status != "deleted" {
            source.capture_status = "deleted".into();
            source.observed_at_unix_ms = now_ms();
            observed(spool, source, "pod_deleted_capture_uncertain", Vec::new())?;
            if let Some(task) = tails.remove(id) {
                task.abort();
            }
        }
    }
    collect_events(client, namespace, spool, known).await
}

async fn collect_events(
    client: &Client,
    namespace: &str,
    spool: &Arc<Spool>,
    known: &BTreeMap<String, LogSource>,
) -> io::Result<()> {
    let events: Api<Event> = Api::namespaced(client.clone(), namespace);
    if let Ok(events) = events.list(&ListParams::default()).await {
        for event in events.items {
            let Some(uid) = event.involved_object.uid else {
                continue;
            };
            let Some(source) = known.values().find(|s| s.pod_uid == uid) else {
                continue;
            };
            let key = format!("event:{}", event.metadata.uid.unwrap_or_default());
            let version = event.metadata.resource_version.unwrap_or_default();
            if spool.checkpoint(&key)?.as_deref() != Some(&version) {
                let text = format!(
                    "{}: {}",
                    event.reason.unwrap_or_default(),
                    event.message.unwrap_or_default()
                );
                for bytes in text.as_bytes().chunks(8192) {
                    observed(spool, source, "kubernetes_event", bytes.into())?;
                }
                spool.set_checkpoint(&key, &version)?;
            }
        }
    }
    Ok(())
}

fn watch_namespaces(
    client: &Client,
    namespaces: &[String],
    deployment: &str,
    spool: &Arc<Spool>,
    wake: &tokio::sync::mpsc::Sender<()>,
) -> Vec<JoinHandle<()>> {
    let mut watchers = Vec::new();
    for namespace in namespaces {
        let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
        let wake = wake.clone();
        let spool = spool.clone();
        let namespace = namespace.clone();
        let deployment = deployment.to_owned();
        watchers.push(tokio::spawn(async move {
            loop {
                if let Ok(events) = api.watch(&WatchParams::default().timeout(30), "0").await {
                    tokio::pin!(events);
                    while let Ok(Some(event)) = events.try_next().await {
                        match event {
                            WatchEvent::Added(pod)
                            | WatchEvent::Modified(pod)
                            | WatchEvent::Deleted(pod)
                                if owned(&pod, &deployment) =>
                            {
                                // Register the watch object immediately: a short-lived pod
                                // can disappear before the next namespace list completes.
                                let key = format!(
                                    "pod-version:{}",
                                    pod.metadata.uid.as_deref().unwrap_or_default()
                                );
                                let version =
                                    pod.metadata.resource_version.as_deref().unwrap_or_default();
                                if spool.checkpoint(&key).ok().flatten().as_deref() != Some(version)
                                {
                                    let status =
                                        serde_json::to_vec(&pod.status).unwrap_or_default();
                                    for source in sources(&pod, &deployment, &namespace) {
                                        for chunk in status.chunks(8192) {
                                            let _ = observed(
                                                &spool,
                                                &source,
                                                "container_status",
                                                chunk.to_vec(),
                                            );
                                        }
                                    }
                                    let _ = spool.set_checkpoint(&key, version);
                                }
                                let _ = wake.try_send(());
                            }
                            _ => {}
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }));
    }
    watchers
}

/// # Errors
/// Return missing configuration, journal or Kubernetes client errors.
pub async fn run() -> io::Result<()> {
    let deployment = env::var("ORCHESTRATOR_DEPLOYMENT").map_err(io::Error::other)?;
    let namespaces: Vec<_> = env::var("ORCHESTRATOR_LOG_NAMESPACES")
        .map_err(io::Error::other)?
        .split(',')
        .map(str::to_owned)
        .collect();
    let path = env::var("ORCHESTRATOR_TELEMETRY_SPOOL").map_err(io::Error::other)?;
    let spool = Arc::new(Spool::open(
        &Path::new(&path).join("collector.sqlite"),
        env::var("ORCHESTRATOR_TELEMETRY_SPOOL_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1024 * 1024 * 1024),
    )?);
    let endpoint = env::var("ORCHESTRATOR_TELEMETRY_ENDPOINT").map_err(io::Error::other)?;
    let token = std::fs::read_to_string(
        env::var("ORCHESTRATOR_TELEMETRY_TOKEN_PATH").map_err(io::Error::other)?,
    )?;
    let client = Client::try_default().await.map_err(io::Error::other)?;
    let (stop, receiver) = watch::channel(false);
    let journal = spool.clone();
    let delivery = tokio::spawn(async move {
        let _ = ayni_orchestrator_execution::transport::deliver(
            journal,
            &endpoint,
            token.trim(),
            receiver,
        )
        .await;
    });
    let (wake, mut changes) = tokio::sync::mpsc::channel(32);
    let watchers = watch_namespaces(&client, &namespaces, &deployment, &spool, &wake);
    let mut tails = BTreeMap::new();
    let mut known = BTreeMap::new();
    let health = LogSource {
        id: format!(
            "collector:{}",
            env::var("HOSTNAME").unwrap_or_else(|_| "collector".into())
        ),
        deployment: deployment.clone(),
        component: "log-collector".into(),
        provenance: "kubernetes_api".into(),
        capture_status: "connected".into(),
        ..Default::default()
    };
    let mut heartbeat = tokio::time::Instant::now() - Duration::from_secs(5);
    loop {
        if heartbeat.elapsed() >= Duration::from_secs(5) {
            heartbeat = tokio::time::Instant::now();
            let _ = spool.append(TelemetryRecord {
                source: Some(health.clone()),
                event: "collector_heartbeat".into(),
                pending_bytes: spool.pending_bytes().unwrap_or_default(),
                observed_at_unix_ms: now_ms(),
                ..Default::default()
            });
        }
        for namespace in &namespaces {
            if reconcile(
                &client,
                namespace,
                &deployment,
                &spool,
                &mut tails,
                &mut known,
            )
            .await
            .is_err()
            {
                eprintln!("log collection reconciliation unavailable; retained output will retry");
            }
        }
        tokio::select! {
            ()=tokio::time::sleep(Duration::from_secs(5))=>{},
            _=changes.recv()=>{},
            ()=ayni_orchestrator_execution::process::termination()=>break,
        }
    }
    let _ = stop.send(true);
    delivery.abort();
    for (_, task) in tails {
        task.abort();
    }
    for task in watchers {
        task.abort();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unrelated_pods_are_not_collected_and_init_containers_are_inventoried()
    -> Result<(), serde_json::Error> {
        let mut pod: Pod = serde_json::from_value(
            serde_json::json!({"metadata":{"name":"test","uid":"uid"},"spec":{"containers":[{"name":"main"}],"initContainers":[{"name":"init"}]}}),
        )?;
        assert!(!owned(&pod, "deployment"));
        pod.metadata.labels = Some(BTreeMap::from([(
            "ayni.build/deployment".into(),
            "deployment".into(),
        )]));
        assert!(owned(&pod, "deployment"));
        assert_eq!(sources(&pod, "deployment", "namespace").len(), 2);
        Ok(())
    }
}
