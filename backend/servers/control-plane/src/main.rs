use ayni_orchestrator_control_plane::{
    AppState, BackgroundTasks, Configuration, ModelCatalog, RuntimeReconciler, router,
    serve_custom_grpc, serve_grpc,
};
use std::{env, fs, io, process::ExitCode, time::Duration};
use tokio::{net::TcpListener, signal, sync::watch};
use tracing::{Instrument, debug, error, info, warn};

#[tokio::main]
async fn main() -> ExitCode {
    ayni_orchestrator_observability::init("control-plane");
    let process_span = ayni_orchestrator_observability::process_span("control-plane");
    match Box::pin(run().instrument(process_span)).await {
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
    let custom_grpc = configuration
        .custom_grpc()
        .map(|custom| {
            custom
                .load_access()
                .map(|access| (custom.bind_address(), access))
        })
        .transpose()
        .map_err(io::Error::other)?;
    let mut background = BackgroundTasks::default();
    let model_catalog = match model_catalog_from_environment() {
        Ok(catalog) => Some(catalog),
        Err(error) => {
            warn!(reason_code = "model_catalog_unavailable", %error, "model discovery disabled");
            None
        }
    };
    let mut state = AppState::open(configuration.database_path(), configuration.max_attempts())
        .map_err(io::Error::other)?;
    if let Some(catalog) = model_catalog {
        let warmup = catalog.clone();
        background.spawn_once(async move {
            if let Err(error) = warmup.available().await {
                warn!(reason_code = "model_catalog_refresh_failed", %error, "model discovery failed");
            }
        });
        state = state.with_model_catalog(catalog);
    }

    state = state.with_github_onboarding();
    let runtimes = RuntimeReconciler::from_environment(state.clone()).map_err(io::Error::other)?;
    let listener = TcpListener::bind(configuration.bind_address()).await?;
    debug!(address = %configuration.bind_address(), "HTTP control plane started");
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let signal_sender = shutdown_sender.clone();
    let signal_task = tokio::spawn(async move {
        shutdown_signal().await;
        let _ = signal_sender.send(true);
    });
    let database_state = state.clone();
    let database_monitor = state.clone();
    background.spawn_required("database", shutdown_receiver.clone(), async move {
        database_monitor.wait_for_database_exit().await;
    });
    spawn_background_loops(&mut background, state.clone(), runtimes, &shutdown_receiver);
    let http_shutdown = shutdown_receiver.clone();
    let http = axum::serve(listener, router(state.clone())).with_graceful_shutdown(async move {
        wait_for_shutdown(http_shutdown).await;
    });
    let grpc = serve_grpc(
        configuration.grpc_bind_address(),
        state.clone(),
        shutdown_receiver.clone(),
    );
    let custom_shutdown = shutdown_receiver.clone();
    let custom = async move {
        if let Some((address, access)) = custom_grpc {
            serve_custom_grpc(address, state, custom_shutdown, access).await
        } else {
            wait_for_shutdown(custom_shutdown).await;
            Ok(())
        }
    };
    let serving = async {
        tokio::try_join!(async { http.await.map_err(io::Error::other) }, grpc, custom).map(|_| ())
    };
    tokio::pin!(serving);
    let result = tokio::select! {
        result = &mut serving => result,
        error = background.unexpected_exit(shutdown_receiver.clone()) => {
            let _ = shutdown_sender.send(true);
            let _ = tokio::time::timeout(Duration::from_secs(10), &mut serving).await;
            Err(error)
        }
    };
    let _ = shutdown_sender.send(true);
    background.shutdown(Duration::from_secs(10)).await;
    signal_task.abort();
    let database_shutdown =
        tokio::time::timeout(Duration::from_secs(10), database_state.shutdown_database())
            .await
            .map_err(|_| io::Error::other("database shutdown deadline exceeded"))?
            .map_err(io::Error::other);
    result.and(database_shutdown)
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

fn spawn_runtime_loops(
    background: &mut BackgroundTasks,
    runtimes: Option<std::sync::Arc<RuntimeReconciler>>,
    shutdown: &watch::Receiver<bool>,
) {
    let Some(runtimes) = runtimes else {
        return;
    };
    for operation in 0..3 {
        let runtimes = runtimes.clone();
        let name = match operation {
            0 => "runtime-verification",
            1 => "coding",
            _ => "publication",
        };
        background.spawn_periodic(name, Duration::from_secs(5), shutdown.clone(), move || {
            let runtimes = runtimes.clone();
            async move {
                match operation {
                    0 => runtimes.reconcile().await,
                    1 => runtimes.reconcile_coding().await,
                    _ => runtimes.reconcile_publication().await,
                }
            }
        });
    }
}

fn spawn_background_loops(
    background: &mut BackgroundTasks,
    state: AppState,
    runtimes: Option<std::sync::Arc<RuntimeReconciler>>,
    shutdown: &watch::Receiver<bool>,
) {
    let onboarding_state = state.clone();
    background.spawn_periodic(
        "onboarding",
        Duration::from_secs(15),
        shutdown.clone(),
        move || {
            let state = onboarding_state.clone();
            async move {
                state.reconcile_onboarding().await;
            }
        },
    );
    let reconciliation_state = state;
    background.spawn_periodic(
        "retention",
        Duration::from_secs(5),
        shutdown.clone(),
        move || {
            let state = reconciliation_state.clone();
            async move {
                if let Err(error) = state.reconcile_once().await {
                    warn!(reason_code = "clean_reconciliation_failed", %error,
                    "clean-v1 reconciliation pass failed");
                }
            }
        },
    );
    spawn_runtime_loops(background, runtimes, shutdown);
}
