//! Canonical admission to terminal observation across real durability boundaries.
#[path = "support/fixture.rs"]
mod fixture;
#[path = "support/scenario.rs"]
mod scenario;
#[path = "support/transport.rs"]
mod transport;

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires scripts/ci/check-workflow-m1.sh disposable PostgreSQL and authenticated JetStream"]
async fn canonical_workflow_survives_every_retained_handoff_boundary() -> Result {
    Box::pin(scenario::run()).await
}
