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
