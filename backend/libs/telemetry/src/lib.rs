//! Canonical v1 telemetry types shared by producers, persistence and API adapters.
#[allow(clippy::all, clippy::pedantic, clippy::nursery)]
pub mod v1 {
    tonic::include_proto!("ayni.orchestrator.v1");
    include!(concat!(env!("OUT_DIR"), "/ayni.orchestrator.v1.serde.rs"));
}
include!(concat!(env!("OUT_DIR"), "/messages.rs"));
