//! Ayni Managed profile contracts and evidence admission.
//!
//! The profile consumes the shared container execution contract. Its Ayni
//! release and certificate requirements do not apply to every
//! container runtime. The Actions evidence importer remains the current
//! implementation; these modules expose managed admission expectations.

pub mod coding;
pub mod compatibility;
pub mod managed_runtime;
pub mod runtime_execution;
