//! Tracked reconciliation loops and bounded process shutdown.
use std::{future::Future, io, time::Duration};
use tokio::{sync::watch, task::JoinSet};

/// Owns background tasks so an unexpected exit cannot silently disable work.
#[derive(Default)]
pub struct BackgroundTasks {
    tasks: JoinSet<Option<&'static str>>,
}

impl BackgroundTasks {
    /// Track finite startup work; successful completion is expected.
    pub fn spawn_once(&mut self, operation: impl Future<Output = ()> + Send + 'static) {
        self.tasks.spawn(async move {
            operation.await;
            None
        });
    }

    /// Track a persistent dependency; its exit is unexpected until shutdown begins.
    pub fn spawn_required(
        &mut self,
        name: &'static str,
        mut shutdown: watch::Receiver<bool>,
        operation: impl Future<Output = ()> + Send + 'static,
    ) {
        self.tasks.spawn(async move {
            if !*shutdown.borrow() {
                tokio::select! {
                    () = operation => {},
                    _ = shutdown.changed() => {},
                }
            }
            Some(name)
        });
    }

    pub fn spawn_periodic<F, Fut>(
        &mut self,
        name: &'static str,
        period: Duration,
        mut shutdown: watch::Receiver<bool>,
        operation: F,
    ) where
        F: Fn() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        self.tasks.spawn(async move {
            let mut ticker = tokio::time::interval(period);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                if *shutdown.borrow() {
                    return Some(name);
                }
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => return Some(name),
                    _ = ticker.tick() => operation().await,
                }
            }
        });
    }

    /// Combine bounded observation hints with periodic repair, in a single serialized lane.
    pub fn spawn_observed<F, Fut>(
        &mut self,
        name: &'static str,
        period: Duration,
        mut shutdown: watch::Receiver<bool>,
        wake: std::sync::Arc<tokio::sync::Notify>,
        operation: F,
    ) where
        F: Fn() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        self.tasks.spawn(async move {
            let mut ticker = tokio::time::interval(period);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                if *shutdown.borrow() {
                    return Some(name);
                }
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => return Some(name),
                    _ = ticker.tick() => {},
                    () = wake.notified() => {},
                }
                operation().await;
                // Coalesce a hot stream without starving shutdown or periodic repair.
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => return Some(name),
                    () = tokio::time::sleep(Duration::from_secs(1)) => {},
                }
            }
        });
    }

    /// Wait for a loop to exit while the server should still be running.
    pub async fn unexpected_exit(&mut self, shutdown: watch::Receiver<bool>) -> io::Error {
        loop {
            let result = self.tasks.join_next().await;
            if *shutdown.borrow() {
                // A completed loop during coordinated shutdown is expected.
                return std::future::pending().await;
            }
            match result {
                Some(Ok(Some(name))) => {
                    return io::Error::other(format!("background task {name} stopped"));
                }
                Some(Err(_)) => return io::Error::other("background task failed"),
                Some(Ok(None)) => {}
                None => return std::future::pending().await,
            }
        }
    }

    /// Allow an in-flight pass to finish, then abort any tasks beyond the deadline.
    pub async fn shutdown(&mut self, grace: Duration) {
        let joined = tokio::time::timeout(grace, async {
            while let Some(result) = self.tasks.join_next().await {
                if let Err(error) = result {
                    tracing::warn!(
                        reason_code = "background_task_failed",
                        panicked = error.is_panic(),
                        "background task failed during shutdown"
                    );
                }
            }
        })
        .await;
        if joined.is_err() {
            tracing::warn!(
                reason_code = "background_shutdown_timeout",
                "background tasks exceeded the shutdown deadline"
            );
            self.tasks.abort_all();
            while self.tasks.join_next().await.is_some() {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BackgroundTasks;
    use std::{io, time::Duration};
    use tokio::sync::{mpsc, watch};

    #[tokio::test]
    async fn shutdown_finishes_the_active_pass_and_prevents_another() -> io::Result<()> {
        let (shutdown, receiver) = watch::channel(false);
        let (started, mut passes) = mpsc::unbounded_channel();
        let mut tasks = BackgroundTasks::default();
        tasks.spawn_periodic("test", Duration::from_secs(60), receiver, move || {
            let started = started.clone();
            async move {
                let _ = started.send(());
            }
        });
        passes
            .recv()
            .await
            .ok_or_else(|| io::Error::other("pass missing"))?;
        shutdown.send(true).map_err(io::Error::other)?;
        tasks.shutdown(Duration::from_secs(1)).await;
        assert!(passes.recv().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn observed_passes_coalesce_bursts_and_keep_periodic_repair() -> io::Result<()> {
        let (shutdown, receiver) = watch::channel(false);
        let (started, mut passes) = mpsc::unbounded_channel();
        let wake = std::sync::Arc::new(tokio::sync::Notify::new());
        let mut tasks = BackgroundTasks::default();
        tasks.spawn_observed(
            "observed",
            Duration::from_secs(2),
            receiver,
            wake.clone(),
            move || {
                let started = started.clone();
                async move {
                    let _ = started.send(());
                }
            },
        );
        passes
            .recv()
            .await
            .ok_or_else(|| io::Error::other("initial pass missing"))?;
        for _ in 0..10000 {
            wake.notify_one();
        }
        tokio::time::timeout(Duration::from_secs(2), passes.recv())
            .await
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("observation pass missing"))?;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), passes.recv())
                .await
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(3), passes.recv())
            .await
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("periodic repair missing"))?;
        shutdown.send(true).map_err(io::Error::other)?;
        tasks.shutdown(Duration::from_secs(1)).await;
        assert!(passes.recv().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn task_failure_is_observable() {
        let (_shutdown, receiver) = watch::channel(false);
        let mut tasks = BackgroundTasks::default();
        tasks.spawn_periodic(
            "test",
            Duration::from_secs(60),
            receiver.clone(),
            || async {
                // Simulate a task which panics independently of its supervisor.
                panic!("simulated loop failure");
            },
        );
        let error = tasks.unexpected_exit(receiver).await;
        assert!(error.to_string().contains("background task failed"));
    }
    #[tokio::test]
    async fn persistent_dependency_exit_stops_the_server() -> io::Result<()> {
        let (_shutdown, receiver) = watch::channel(false);
        let (finished, dependency) = tokio::sync::oneshot::channel();
        let mut tasks = BackgroundTasks::default();
        tasks.spawn_required("database", receiver.clone(), async move {
            let _ = dependency.await;
        });
        finished
            .send(())
            .map_err(|()| io::Error::other("dependency missing"))?;
        let error = tokio::time::timeout(Duration::from_secs(1), tasks.unexpected_exit(receiver))
            .await
            .map_err(io::Error::other)?;
        assert!(error.to_string().contains("database stopped"));
        Ok(())
    }
}
