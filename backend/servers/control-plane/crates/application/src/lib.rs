//! Control-plane use cases independent of transports and persistence codecs.
pub mod tasks;

pub mod execution;
pub mod execution_settings;

pub mod readiness;

pub mod publication;

pub mod assessment;

pub mod reconciliation;

pub mod identity;

pub mod runtime_activation;

pub mod runtime_selection;

pub mod runtime_reconciliation;

pub mod automation;
