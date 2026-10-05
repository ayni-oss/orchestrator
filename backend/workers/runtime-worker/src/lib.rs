//! Trusted launch operations. Build and coding commands run in separate Jobs
//! with disjoint authority; coding results are never assessment evidence.

#[path = "integrations/github-actions/mod.rs"]
pub mod assessment_actions;
pub mod coding;
#[path = "coding/model_usage.rs"]
pub mod model_usage;
#[path = "operations/runtime.rs"]
pub mod runtime_execution;

#[path = "coding/activity.rs"]
pub mod activity;

#[path = "coding/checkpoint.rs"]
mod coding_checkpoint;
