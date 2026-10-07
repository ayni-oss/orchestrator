//! Internal Kubernetes execution adapter and deterministic Job manifests.
//!
//! Public application contracts and product lifecycle decisions stay outside
//! this crate. Configuration is operator-owned; credentials cross the boundary
//! only as scoped request inputs or Kubernetes secret references.

mod coding;
pub mod configuration;
pub mod execution;
pub mod job;
pub mod registry_probe;
mod workload;

mod execution_port;

pub mod connections;
