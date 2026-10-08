//! Safe inputs for trusted image operations.
use serde::{Deserialize, Serialize};
/// Publication needs a platform, never checkout or signing authority.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ImagePublicationRequest {
    pub platform: String,
}
