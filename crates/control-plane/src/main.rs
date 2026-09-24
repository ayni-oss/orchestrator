use ayni_orchestrator_control_plane::{
    AppState, Configuration, DevelopmentReconciler, KubernetesJobTransport, PublisherState,
    Reconciler, router,
};
use kube::Client;
use std::io;
use tokio::{net::TcpListener, signal};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), io::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let configuration = Configuration::from_environment().map_err(io::Error::other)?;
    let state = AppState::open(configuration.database_path(), configuration.max_attempts())
        .map_err(io::Error::other)?
        .with_publisher(PublisherState::from_environment().map_err(io::Error::other)?);
    let client = Client::try_default().await.map_err(io::Error::other)?;
    let reconciler = Reconciler::new(
        state.store(),
        KubernetesJobTransport::new(
            client.clone(),
            configuration.kubernetes_namespace().to_owned(),
            configuration.worker_image().to_owned(),
        ),
    );
    let reconciliation_task = tokio::spawn(run_reconciliation(reconciler));
    let development_task = state.publisher().map(|publisher| {
        tokio::spawn(run_development_reconciliation(DevelopmentReconciler::new(
            state.clone(),
            client,
            configuration.kubernetes_namespace().to_owned(),
            configuration.worker_image().to_owned(),
            publisher,
        )))
    });
    let listener = TcpListener::bind(configuration.bind_address()).await?;
    info!(address = %configuration.bind_address(), "control plane started");

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    reconciliation_task.abort();
    if let Some(task) = development_task {
        task.abort();
    }
    Ok(())
}

async fn run_development_reconciliation(reconciler: DevelopmentReconciler) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        ticker.tick().await;
        reconciler.reconcile_once().await;
    }
}

async fn run_reconciliation(reconciler: Reconciler<KubernetesJobTransport>) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        ticker.tick().await;
        reconciler.reconcile_once().await;
    }
}

async fn shutdown_signal() {
    let interrupt = async {
        if let Err(error) = signal::ctrl_c().await {
            error!(%error, "failed to listen for interrupt signal");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => error!(%error, "failed to listen for terminate signal"),
        }
    };

    #[cfg(unix)]
    tokio::select! {
        () = interrupt => {},
        () = terminate => {},
    }

    #[cfg(not(unix))]
    interrupt.await;

    info!("control plane received shutdown signal");
}
