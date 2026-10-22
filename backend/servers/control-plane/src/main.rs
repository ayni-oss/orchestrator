use ayni_orchestrator_control_plane::{
    Configuration, WorkflowReadiness, broker, maintenance, router, serve_grpc,
};
use ayni_orchestrator_workflow_postgres::WorkflowStore;
use std::{io, process::ExitCode};
use tokio::{net::TcpListener, signal, sync::watch};
use tracing::{Instrument as _, error, info};

fn main() -> ExitCode {
    let telemetry = ayni_orchestrator_observability::init("control-plane");
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_or(ExitCode::FAILURE, |runtime| {
            runtime.block_on(Box::pin(control_plane_main()))
        });
    telemetry.shutdown();
    result
}

async fn control_plane_main() -> ExitCode {
    let span = ayni_orchestrator_observability::process_span("control-plane");
    match Box::pin(run()).instrument(span).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(
                reason_code = "control_plane_failed",
                error = %ayni_orchestrator_observability::redact(&error.to_string()),
                "replacement workflow control plane stopped"
            );
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), io::Error> {
    let configuration = Configuration::from_environment().map_err(io::Error::other)?;
    let runtime_database_url = configuration.runtime_database_url()?;
    let runtime_database_role = configuration.runtime_database_role().to_owned();
    ayni_orchestrator_workflow_postgres::validate_for_runtime(
        &runtime_database_url,
        &runtime_database_role,
    )
    .await
    .map_err(io::Error::other)?;

    let http_listener = TcpListener::bind(configuration.http_bind_address()).await?;
    let store = WorkflowStore::new(runtime_database_url.clone());
    let http = router(WorkflowReadiness::new(
        runtime_database_url,
        runtime_database_role,
    ))
    .merge(ayni_orchestrator_control_plane::session_authority::router(
        store.clone(),
    )?);
    let http = http.merge(ayni_orchestrator_control_plane::task_authority::router(
        store.clone(),
    )?);
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let signal_sender = shutdown_sender.clone();
    let signal_task = tokio::spawn(async move {
        shutdown_signal().await;
        let _ = signal_sender.send(true);
    });
    info!(
        http_address = %configuration.http_bind_address(),
        grpc_address = %configuration.grpc_bind_address(),
        "replacement workflow control plane ready"
    );
    let mut http_shutdown = shutdown_receiver.clone();
    let http_server = axum::serve(http_listener, http).with_graceful_shutdown(async move {
        if !*http_shutdown.borrow() {
            let _ = http_shutdown.changed().await;
        }
    });
    let grpc_server = serve_grpc(
        configuration.grpc_bind_address(),
        store.clone(),
        shutdown_receiver.clone(),
    );
    let cleanup = maintenance::run(store.clone(), shutdown_receiver.clone());
    let publisher = broker::run_publisher(&configuration, store, shutdown_receiver);
    let result = tokio::try_join!(
        async { http_server.await.map_err(io::Error::other) },
        grpc_server,
        publisher,
        async {
            cleanup.await;
            Ok(())
        },
    )
    .map(|_| ());
    let _ = shutdown_sender.send(true);
    signal_task.abort();
    result
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
}
