//! Ayni Managed profile contracts and evidence admission.
//!
//! The profile consumes the shared container execution contract. Its Ayni
//! release, certificate, and assessment requirements do not apply to every
//! container runtime. The Actions evidence importer remains the current
//! implementation; these modules do not implement internal candidate assessment.

pub mod assessment;
pub mod ayni_release;
pub mod coding;
pub mod managed_runtime;
pub mod runtime_execution;
