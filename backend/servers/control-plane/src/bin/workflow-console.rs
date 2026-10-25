use ayni_orchestrator_control_plane::console::{Console, router};
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
    let web_directory = arguments.next();
    if arguments.next().is_some() {
        return Err(
            "usage: workflow-console [LOOPBACK_LISTEN_ADDRESS] [LOOPBACK_GRPC_ADDRESS] [WEB_BUILD_DIRECTORY]".into(),
        );
    }
    let mut console = Console::new(address, endpoint)?;
    if let Some(directory) = web_directory {
        console = console.with_web_assets(std::path::Path::new(&directory))?;
        println!("Project workspace: http://{address}/workspace/");
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("Contributor console: http://{address}");
    axum::serve(listener, router(console))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
