//! Metadata-only observations accelerate guarded reconciliation; they never accept results.
use k8s_openapi::api::{
    batch::v1::Job,
    core::v1::{Pod, Secret},
};
use kube::{
    Api, Client, Resource,
    api::{ListParams, WatchEvent, WatchParams},
};
use serde::de::DeserializeOwned;
use std::{fmt::Debug, sync::Arc, time::Duration};
use tokio::sync::Notify;
use tokio_stream::StreamExt;

/// One retained permit per reconciliation lane bounds bursts independently of object count.
#[derive(Clone, Default)]
pub struct ObservationWakes {
    pub runtime: Arc<Notify>,
    pub coding: Arc<Notify>,
    pub connections: Arc<Notify>,
}

pub struct KubernetesNotifications {
    client: Client,
    namespace: String,
}
impl KubernetesNotifications {
    /// Uses deployed identity only; never selects a developer's current context.
    /// # Errors
    /// Returns a safe reason code when deployed Kubernetes identity is unavailable.
    pub fn new(namespace: String) -> Result<Self, &'static str> {
        let configuration =
            kube::Config::incluster().map_err(|_| "kubernetes_incluster_required")?;
        Ok(Self {
            client: Client::try_from(configuration).map_err(|_| "kubernetes_unavailable")?,
            namespace,
        })
    }

    /// Runs independent streams. Each disconnect relists from fresh resource version,
    /// waking repair even when the relist is empty (including missed deletions).
    pub async fn runtime(self, wakes: ObservationWakes) {
        tokio::join!(
            observe(
                Api::<Job>::namespaced(self.client.clone(), &self.namespace),
                "ayni.io/runtime-operation",
                wakes.runtime.clone()
            ),
            observe(
                Api::<Pod>::namespaced(self.client.clone(), &self.namespace),
                "ayni.io/runtime-operation",
                wakes.runtime.clone()
            ),
            observe(
                Api::<Job>::namespaced(self.client.clone(), &self.namespace),
                "ayni.io/attempt-id",
                wakes.coding.clone()
            ),
            observe(
                Api::<Pod>::namespaced(self.client.clone(), &self.namespace),
                "ayni.io/attempt-id",
                wakes.coding.clone()
            ),
            observe_engine(Api::<Pod>::namespaced(self.client, &self.namespace), wakes),
        );
    }

    /// Connector and certificate credentials share the same explicit UI ownership labels.
    /// Only metadata is requested; no secret payload is retained or logged.
    pub async fn connections(self, wakes: ObservationWakes) {
        tokio::join!(
            observe(
                Api::<Secret>::namespaced(self.client.clone(), &self.namespace),
                "ayni.dev/management-source=ui,ayni.dev/purpose=connection,ayni.dev/connection-id",
                wakes.connections.clone()
            ),
            observe(
                Api::<Job>::namespaced(self.client.clone(), &self.namespace),
                "ayni.io/connection-operation",
                wakes.connections.clone()
            ),
            observe(
                Api::<Pod>::namespaced(self.client, &self.namespace),
                "ayni.io/connection-operation",
                wakes.connections
            ),
        );
    }
}

async fn observe_engine(api: Api<Pod>, wakes: ObservationWakes) {
    // Engines participate in both preparation and coding. A bounded bridge wakes
    // both lanes, whose existing owner/UID checks perform authoritative readback.
    let notification = Arc::new(Notify::new());
    let forward = async {
        loop {
            notification.notified().await;
            wakes.runtime.notify_one();
            wakes.coding.notify_one();
        }
    };
    tokio::join!(
        observe(api, "ayni.io/build-engine", notification.clone()),
        forward
    );
}

async fn observe<K>(api: Api<K>, selector: &'static str, wake: Arc<Notify>)
where
    K: Resource + Clone + Debug + DeserializeOwned + Send + 'static,
{
    loop {
        if observe_once(&api, selector, &wake).await.is_err() {
            tracing::warn!(
                reason_code = "kubernetes_observation_relist_required",
                selector,
                "Kubernetes metadata observation interrupted; periodic repair remains active"
            );
        }
        // Prevent tight reconnect loops on permission/transport failures.
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn observe_once<K>(api: &Api<K>, selector: &str, wake: &Notify) -> Result<(), kube::Error>
where
    K: Resource + Clone + Debug + DeserializeOwned + Send + 'static,
{
    // Pagination bounds metadata response memory. Do not retain an object cache:
    // the notification is a hint and authoritative reads stay in reconciliation.
    let mut params = ListParams::default().labels(selector).limit(100);
    let version = loop {
        let list = api.list_metadata(&params).await?;
        if let Some(token) = list.metadata.continue_.filter(|token| !token.is_empty()) {
            params = params.continue_token(&token);
        } else {
            break list.metadata.resource_version.unwrap_or_else(|| "0".into());
        }
    };
    wake.notify_one();
    let stream = api
        .watch_metadata(
            &WatchParams::default().labels(selector).timeout(60),
            &version,
        )
        .await?;
    tokio::pin!(stream);
    while let Some(event) = stream.next().await {
        notify_change(event?, wake)?;
    }
    Ok(())
}

fn notify_change<K>(
    event: WatchEvent<kube::core::PartialObjectMeta<K>>,
    wake: &Notify,
) -> Result<(), kube::Error> {
    match event {
        WatchEvent::Added(_) | WatchEvent::Modified(_) | WatchEvent::Deleted(_) => {
            wake.notify_one();
        }
        WatchEvent::Bookmark(_) => {}
        WatchEvent::Error(error) => return Err(kube::Error::Api(error)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn expired_resource_versions_request_relist_without_accepting_a_change()
    -> Result<(), Box<dyn std::error::Error>> {
        let wake = Notify::new();
        let error: kube::core::Status = serde_json::from_value(serde_json::json!({
            "status":"Failure", "message":"expired", "reason":"Expired", "code":410
        }))?;
        assert!(
            matches!(notify_change::<Job>(WatchEvent::Error(Box::new(error)), &wake), Err(kube::Error::Api(error)) if error.code == 410)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(10), wake.notified())
                .await
                .is_err()
        );
        let deleted = kube::core::PartialObjectMeta::<Job>::default();
        notify_change(WatchEvent::Deleted(deleted), &wake)?;
        tokio::time::timeout(Duration::from_millis(10), wake.notified()).await?;
        Ok(())
    }

    #[tokio::test]
    async fn bursts_coalesce_without_crossing_reconciliation_lanes() {
        let wakes = ObservationWakes::default();
        for _ in 0..10000 {
            wakes.runtime.notify_one();
        }
        wakes.runtime.notified().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(10), wakes.runtime.notified())
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(10), wakes.coding.notified())
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(10), wakes.connections.notified())
                .await
                .is_err()
        );
    }
}

#[cfg(test)]
mod cluster_tests {
    use super::*;
    use kube::api::{DeleteParams, Patch, PatchParams, PostParams};

    #[tokio::test]
    #[ignore = "requires explicit Docker Desktop Kubernetes and a disposable namespace"]
    async fn docker_desktop_watch_excludes_unowned_secret_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        let namespace = std::env::var("ORCHESTRATOR_WATCH_TEST_NAMESPACE")?;
        let config = kube::Config::from_kubeconfig(&kube::config::KubeConfigOptions {
            context: Some("docker-desktop".into()),
            ..Default::default()
        })
        .await?;
        let api = Api::<Secret>::namespaced(Client::try_from(config)?, &namespace);
        let wake = Arc::new(Notify::new());
        let observed = api.clone();
        let signal = wake.clone();
        let stream = tokio::spawn(async move {
            observe_once(
                &observed,
                "ayni.dev/management-source=ui,ayni.dev/purpose=connection,ayni.dev/connection-id",
                &signal,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(10), wake.notified()).await?;
        // These empty Secrets contain no credentials. Only the owned metadata can wake the lane.
        let unowned: Secret =
            serde_json::from_value(serde_json::json!({"metadata":{"name":"watch-unowned"}}))?;
        api.create(&PostParams::default(), &unowned).await?;
        assert!(
            tokio::time::timeout(Duration::from_millis(200), wake.notified())
                .await
                .is_err()
        );
        let owned: Secret = serde_json::from_value(
            serde_json::json!({"metadata":{"name":"watch-owned", "labels":{
                "ayni.dev/management-source":"ui", "ayni.dev/purpose":"connection", "ayni.dev/connection-id":"fixture"
            }}}),
        )?;
        api.create(&PostParams::default(), &owned).await?;
        tokio::time::timeout(Duration::from_secs(10), wake.notified()).await?;
        api.delete("watch-owned", &DeleteParams::default()).await?;
        tokio::time::timeout(Duration::from_secs(10), wake.notified()).await?;
        stream.abort();
        let _ = stream.await;
        api.delete("watch-unowned", &DeleteParams::default())
            .await?;
        Ok(())
    }

    /// Exercise real metadata list/watch/relist without executing a workload.
    /// Run with `ORCHESTRATOR_WATCH_TEST_NAMESPACE=<disposable namespace>` and
    /// `cargo test -p ayni-orchestrator-kubernetes docker_desktop_watch -- --ignored`.
    #[tokio::test]
    #[ignore = "requires explicit Docker Desktop Kubernetes and a disposable namespace"]
    async fn docker_desktop_watch_observes_deletion_and_relist_after_disconnect()
    -> Result<(), Box<dyn std::error::Error>> {
        let namespace = std::env::var("ORCHESTRATOR_WATCH_TEST_NAMESPACE")?;
        let config = kube::Config::from_kubeconfig(&kube::config::KubeConfigOptions {
            context: Some("docker-desktop".into()),
            ..Default::default()
        })
        .await?;
        let client = Client::try_from(config)?;
        let api = Api::<Job>::namespaced(client, &namespace);
        let name = format!("watch-fixture-{}", std::process::id());
        let job: Job = serde_json::from_value(serde_json::json!({
            "apiVersion":"batch/v1", "kind":"Job",
            "metadata":{"name":name,"labels":{"ayni.io/runtime-operation":name,"ayni.build/watch-fixture":"true"}},
            "spec":{"suspend":true,"template":{"spec":{"restartPolicy":"Never","containers":[{"name":"fixture","image":"busybox:1.37"}]}}}
        }))?;
        api.create(&PostParams::default(), &job).await?;
        let result = async {
            let wake = Arc::new(Notify::new());
            let observed = api.clone();
            let signal = wake.clone();
            let stream = tokio::spawn(async move {
                observe_once(
                    &observed,
                    "ayni.io/runtime-operation,ayni.build/watch-fixture=true",
                    &signal,
                )
                .await
            });
            tokio::time::timeout(Duration::from_secs(10), wake.notified()).await?;
            // Drain the initial Added notification before the mutation assertion.
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = tokio::time::timeout(Duration::from_millis(100), wake.notified()).await;
            api.patch(
                &name,
                &PatchParams::default(),
                &Patch::Merge(serde_json::json!({"metadata":{"annotations":{"test":"changed"}}})),
            )
            .await?;
            tokio::time::timeout(Duration::from_secs(10), wake.notified()).await?;
            api.delete(&name, &DeleteParams::default()).await?;
            tokio::time::timeout(Duration::from_secs(10), wake.notified()).await?;
            stream.abort();
            let _ = stream.await;
            // A fresh empty relist still wakes repair: missed deletions cannot disappear.
            let observed = api.clone();
            let signal = wake.clone();
            let resumed = tokio::spawn(async move {
                observe_once(
                    &observed,
                    "ayni.io/runtime-operation,ayni.build/watch-fixture=true",
                    &signal,
                )
                .await
            });
            tokio::time::timeout(Duration::from_secs(10), wake.notified()).await?;
            resumed.abort();
            let _ = resumed.await;
            Ok::<(), Box<dyn std::error::Error>>(())
        }
        .await;
        let _ = api.delete(&name, &DeleteParams::default()).await;
        result
    }
}
