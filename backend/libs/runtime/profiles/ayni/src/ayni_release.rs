//! Release policy for the Ayni integration, not a platform-wide tool requirement.
//! Update `../ayni-release.json`; build tooling and workflow rendering share it.

/// Ayni CLI release tested with this adapter.
pub const CLI_VERSION: &str = env!("AYNI_CLI_VERSION");
/// Checksum of the released Linux/amd64 CLI archive.
pub const LINUX_AMD64_ARCHIVE_SHA256: &str = env!("AYNI_ARCHIVE_SHA256");
/// Immutable released executor used to build Ayni environments.
pub const EXECUTOR_IMAGE: &str = env!("AYNI_EXECUTOR_IMAGE");
/// Signals contract understood by the Ayni evidence adapter.
pub const SIGNALS_SCHEMA_VERSION: &str = env!("AYNI_SIGNALS_SCHEMA_VERSION");

/// Pinned Docker CLI used to bootstrap builder installation.
pub const DOCKER_CLIENT_IMAGE: &str = env!("AYNI_DOCKER_CLIENT_IMAGE");
/// Trusted packaging recipe shipped with the release contract.
pub const BUILDER_RECIPE: &str =
    include_str!("../../../../../../deploy/images/runtime-builder/Dockerfile");

/// Identity of the shipped recipe and its pinned release inputs.
#[must_use]
pub fn builder_recipe_identity() -> String {
    use sha2::{Digest, Sha256};
    format!(
        "sha256:{:x}",
        Sha256::digest(format!(
            "{CLI_VERSION}\n{EXECUTOR_IMAGE}\n{DOCKER_CLIENT_IMAGE}\n{BUILDER_RECIPE}"
        ))
    )
}
