//! Shared Linux process confinement for trusted task and compatibility launchers.
//! Call before constructing an async runtime and pass no inherited authority handles.
pub mod filesystem;
pub mod identity;
pub mod network;
