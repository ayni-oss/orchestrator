//! Credential-free result of an isolated registry lookup.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageDiscovery {
    Pending,
    Missing,
    Resolved { digest: String, image_id: String },
    Failed { reason: String },
}
