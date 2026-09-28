use ayni_orchestrator_control_plane::{
    AppState, Configuration, DevelopmentReconciler, KubernetesJobTransport, ModelCatalog,
    PreparationReconciler, PublisherState, Reconciler, router, serve_grpc,
};
use kube::Client;
use std::{env, fs, io, process::ExitCode};
use tokio::{net::TcpListener, signal, sync::watch};
use tracing::{Instrument, error, info, warn};

#[tokio::main]
async fn main() -> ExitCode {
    ayni_orchestrator_observability::init("control-plane");
    let process_span = ayni_orchestrator_observability::process_span("control-plane");
    match run().instrument(process_span).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(
                reason_code = "control_plane_failed",
                error = %ayni_orchestrator_observability::redact(&error.to_string()),
                "control plane stopped"
            );
            ExitCode::FAILURE
        }
    }
}

fn model_catalog_from_environment() -> Result<ModelCatalog, io::Error> {
    let openai_api_key_path = env::var("OPENAI_API_KEY_PATH")
        .map_err(|_| io::Error::other("OPENAI_API_KEY_PATH is not configured"))?;
    let openai_api_key = fs::read_to_string(openai_api_key_path)
        .map_err(|_| io::Error::other("could not read the OpenAI API key"))?;
    ModelCatalog::openai(&openai_api_key).map_err(io::Error::other)
}

async fn run() -> Result<(), io::Error> {
    let configuration = Configuration::from_environment().map_err(io::Error::other)?;
    let model_catalog = match model_catalog_from_environment() {
        Ok(catalog) => Some(catalog),
        Err(error) => {
            warn!(reason_code = "model_catalog_unavailable", %error, "model discovery disabled");
            None
        }
    };
    let mut state = AppState::open(configuration.database_path(), configuration.max_attempts())
        .map_err(io::Error::other)?
        .with_publisher(PublisherState::from_environment().map_err(io::Error::other)?);
    if let Some(catalog) = model_catalog {
        let warmup = catalog.clone();
        tokio::spawn(async move {
            if let Err(error) = warmup.available().await {
                warn!(reason_code = "model_catalog_refresh_failed", %error, "model discovery failed");
            }
        });
        state = state.with_model_catalog(catalog);
    }
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
            client.clone(),
            configuration.kubernetes_namespace().to_owned(),
            configuration.worker_image().to_owned(),
            publisher,
        )))
    });
    let preparation_task = state.publisher().map(|publisher| {
        tokio::spawn(run_preparation_reconciliation(PreparationReconciler::new(
            state.clone(),
            client,
            configuration.kubernetes_namespace().to_owned(),
            configuration.worker_image().to_owned(),
            publisher,
        )))
    });
    let listener = TcpListener::bind(configuration.bind_address()).await?;
    info!(address = %configuration.bind_address(), "HTTP control plane started");
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let signal_task = tokio::spawn(async move {
        shutdown_signal().await;
        let _ = shutdown_sender.send(true);
    });
    let http_shutdown = shutdown_receiver.clone();
    let http = axum::serve(listener, router(state.clone())).with_graceful_shutdown(async move {
        wait_for_shutdown(http_shutdown).await;
    });
    let grpc = serve_grpc(configuration.grpc_bind_address(), state, shutdown_receiver);
    tokio::try_join!(async { http.await.map_err(io::Error::other) }, grpc)?;
    signal_task.abort();
    reconciliation_task.abort();
    if let Some(task) = development_task {
        task.abort();
    }
    if let Some(task) = preparation_task {
        task.abort();
    }
    Ok(())
}

async fn run_preparation_reconciliation(reconciler: PreparationReconciler) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        ticker.tick().await;
        reconciler.reconcile_once().await;
    }
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

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    if !*shutdown.borrow() {
        let _ = shutdown.changed().await;
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
