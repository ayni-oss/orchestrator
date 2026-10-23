//! Dedicated repository-scoped publisher. Repository code is never executed.
#[path = "../publication/config.rs"]
mod config;
#[path = "../publication/git.rs"]
mod git;
#[path = "../publication/github.rs"]
mod github;
#[path = "../publication/workflow.rs"]
mod workflow;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() -> Result {
    let configuration = config::Configuration::load()?;
    let http = github::client()?;
    loop {
        let operation = workflow::poll(&http, &configuration);
        tokio::select! {
            result = operation => if result.is_err() { eprintln!("task publication awaits reconciliation; inspect retained publication state"); },
            _ = tokio::signal::ctrl_c() => return Ok(())
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
