//! Trusted launch operations. Build and coding commands run in separate Jobs
//! with disjoint authority; coding results are never assessment evidence.

pub use ayni_orchestrator_profile_ayni::managed_runtime;
pub mod assessment_actions;
pub mod coding;
pub mod model_usage;
pub mod runtime_execution;

pub mod activity;

mod coding_checkpoint;
