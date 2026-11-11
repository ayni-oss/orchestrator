use ayni_orchestrator_control_plane::gateway::{Gateway, development_router, router};
use std::{env, error::Error, net::SocketAddr};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let address: SocketAddr = arguments
        .next()
        .unwrap_or_else(|| "127.0.0.1:3000".into())
        .parse()?;
    let endpoint: SocketAddr = arguments
        .next()
        .unwrap_or_else(|| "127.0.0.1:50051".into())
        .parse()?;
    let status = match arguments.next().as_deref() {
        None => None,
        Some("--dev-status") => Some(std::path::PathBuf::from(
            arguments.next().ok_or("missing dev status file")?,
        )),
        Some(_) => {
            return Err("usage: workflow-gateway [LISTEN] [UPSTREAM] [--dev-status FILE]".into());
        }
    };
    if arguments.next().is_some() {
        return Err("unexpected gateway argument".into());
    }
    let gateway = Gateway::new(address, endpoint)?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("Local API gateway: http://{address}");
    let app = status.map_or_else(
        || router(gateway.clone()),
        |path| development_router(gateway.clone(), path),
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
