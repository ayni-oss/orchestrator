use ayni_orchestrator_control_plane::gateway::{Gateway, router};
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
    if arguments.next().is_some() {
        return Err(
            "usage: workflow-gateway [LOOPBACK_LISTEN_ADDRESS] [LOOPBACK_GRPC_ADDRESS]".into(),
        );
    }
    let gateway = Gateway::new(address, endpoint)?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("Local API gateway: http://{address}");
    axum::serve(listener, router(gateway))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
