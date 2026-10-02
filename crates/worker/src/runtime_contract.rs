//! Shared, provider-neutral admission contract for a coding runtime.
//!
//! The controller resolves a registry discovery reference before constructing this
//! value. Kubernetes receives the resulting `repository@sha256:<digest>` only;
//! a tag is never a runnable identity.

use std::fmt;
use thiserror::Error;

const SHA256_HEX_LENGTH: usize = 64;
const MAX_COMMAND_ARGUMENTS: usize = 32;
const MAX_COMMAND_ARGUMENT_BYTES: usize = 4_096;

/// An immutable OCI image reference suitable for a coding Job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeImage(String);

impl RuntimeImage {
    /// Parses an OCI repository reference pinned to a lowercase SHA-256 digest.
    ///
    /// # Errors
    ///
    /// Returns an error when the reference is malformed or is not digest-pinned.
    pub fn parse(value: &str) -> Result<Self, RuntimeContractError> {
        let Some((repository, digest)) = value.split_once("@sha256:") else {
            return Err(RuntimeContractError::MutableImage(value.to_owned()));
        };
        if value.matches("@sha256:").count() != 1
            || !valid_repository(repository)
            || digest.len() != SHA256_HEX_LENGTH
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(RuntimeContractError::InvalidImage(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the exact image reference passed to Kubernetes.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RuntimeImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The supported platform of one runtime artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimePlatform {
    /// Linux on `x86_64`.
    LinuxAmd64,
    /// Linux on ARM64.
    LinuxArm64,
}

impl RuntimePlatform {
    /// Parses the canonical OCI platform value.
    ///
    /// # Errors
    ///
    /// Returns an error when the platform is outside the v1 allow-list.
    pub fn parse(value: &str) -> Result<Self, RuntimeContractError> {
        match value {
            "linux/amd64" => Ok(Self::LinuxAmd64),
            "linux/arm64" => Ok(Self::LinuxArm64),
            _ => Err(RuntimeContractError::UnsupportedPlatform(value.to_owned())),
        }
    }

    /// Returns the canonical OCI platform value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LinuxAmd64 => "linux/amd64",
            Self::LinuxArm64 => "linux/arm64",
        }
    }
}

/// Bounded arguments handed to an image's existing entrypoint.
///
/// Kubernetes `args` replace the image `CMD`, but leave its `ENTRYPOINT` intact.
/// That distinction is intentional: an admitted runtime is responsible for any
/// preparation it needs before it invokes this launcher argv.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LauncherArguments(Vec<String>);

impl LauncherArguments {
    /// Validates a non-empty launcher argv supplied by trusted task construction.
    ///
    /// # Errors
    ///
    /// Returns an error when an argument is empty, oversized, or contains NUL.
    pub fn new(arguments: Vec<String>) -> Result<Self, RuntimeContractError> {
        if arguments.is_empty()
            || arguments.len() > MAX_COMMAND_ARGUMENTS
            || arguments.iter().any(|argument| {
                argument.is_empty()
                    || argument.len() > MAX_COMMAND_ARGUMENT_BYTES
                    || argument.bytes().any(|byte| byte == 0)
            })
        {
            return Err(RuntimeContractError::InvalidLauncherArguments);
        }
        Ok(Self(arguments))
    }

    /// Returns the exact argv passed to the runtime launcher.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.0
    }
}

/// The effective non-root identity verified for the runtime artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeUser {
    uid: i64,
    gid: i64,
}

impl RuntimeUser {
    /// Creates the identity used for the launcher and writable volumes.
    ///
    /// # Errors
    ///
    /// Returns an error for root or otherwise invalid Unix identities.
    pub const fn new(uid: i64, gid: i64) -> Result<Self, RuntimeContractError> {
        if uid <= 0 || gid <= 0 {
            return Err(RuntimeContractError::InvalidRuntimeUser);
        }
        Ok(Self { uid, gid })
    }

    /// Returns the Unix user ID.
    #[must_use]
    pub const fn uid(self) -> i64 {
        self.uid
    }

    /// Returns the Unix group ID.
    #[must_use]
    pub const fn gid(self) -> i64 {
        self.gid
    }
}

/// Immutable inputs required to start an isolated coding runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeContract {
    image: RuntimeImage,
    platform: RuntimePlatform,
    launcher_arguments: LauncherArguments,
    user: RuntimeUser,
}

impl RuntimeContract {
    /// Creates a contract after registry resolution and platform admission.
    #[must_use]
    pub const fn new(
        image: RuntimeImage,
        platform: RuntimePlatform,
        launcher_arguments: LauncherArguments,
        user: RuntimeUser,
    ) -> Self {
        Self {
            image,
            platform,
            launcher_arguments,
            user,
        }
    }

    /// Returns the digest-pinned image identity.
    #[must_use]
    pub const fn image(&self) -> &RuntimeImage {
        &self.image
    }

    /// Returns the admitted platform.
    #[must_use]
    pub const fn platform(&self) -> RuntimePlatform {
        self.platform
    }

    /// Returns arguments passed to the runtime image's preserved entrypoint.
    #[must_use]
    pub const fn launcher_arguments(&self) -> &LauncherArguments {
        &self.launcher_arguments
    }

    /// Returns the effective identity verified for writable runtime paths.
    #[must_use]
    pub const fn user(&self) -> RuntimeUser {
        self.user
    }
}

fn valid_repository(value: &str) -> bool {
    let Some(last) = value.rsplit('/').next() else {
        return false;
    };
    !value.is_empty()
        && value.len() <= 512
        && !last.contains(':')
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'/' | b'-' | b'_' | b':')
        })
}

/// A runtime artifact cannot safely start with the supplied admission data.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RuntimeContractError {
    /// A tag or unqualified image was supplied instead of a digest identity.
    #[error("runtime image must be pinned as repository@sha256:<64 lowercase hex>, got {0:?}")]
    MutableImage(String),
    /// The repository or digest component is malformed.
    #[error("runtime image is malformed: {0:?}")]
    InvalidImage(String),
    /// The runtime platform is outside the v1 allow-list.
    #[error("runtime platform is unsupported: {0:?}")]
    UnsupportedPlatform(String),
    /// The override is not a bounded non-shell argv.
    #[error("runtime launcher arguments must be a bounded non-empty argv")]
    InvalidLauncherArguments,
    /// The runtime's effective identity would violate the non-root boundary.
    #[error("runtime user and group must be positive non-root Unix IDs")]
    InvalidRuntimeUser,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{LauncherArguments, RuntimeContract, RuntimeImage, RuntimePlatform, RuntimeUser};

    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn accepts_only_digest_pinned_images() {
        let image = format!("ghcr.io/acme/runtime@sha256:{DIGEST}");
        assert_eq!(RuntimeImage::parse(&image).unwrap().as_str(), image);
        assert!(RuntimeImage::parse("ghcr.io/acme/runtime:latest").is_err());
        assert!(RuntimeImage::parse("ghcr.io/acme/runtime@sha256:ABC").is_err());
        assert!(RuntimeImage::parse("ghcr.io/acme/runtime:tag@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_err());
    }

    #[test]
    fn rejects_unsupported_platform_and_shell_like_command_contracts() {
        assert!(RuntimePlatform::parse("darwin/arm64").is_err());
        assert!(LauncherArguments::new(Vec::new()).is_err());
        assert!(LauncherArguments::new(vec!["run\0this".into()]).is_err());
        let contract = RuntimeContract::new(
            RuntimeImage::parse(&format!(
                "registry.example:5000/team/runtime@sha256:{DIGEST}"
            ))
            .unwrap(),
            RuntimePlatform::parse("linux/arm64").unwrap(),
            LauncherArguments::new(vec![
                "/orchestrator/launcher".into(),
                "--workspace".into(),
                "/workspace".into(),
            ])
            .unwrap(),
            RuntimeUser::new(10_002, 10_001).unwrap(),
        );
        assert_eq!(contract.platform().as_str(), "linux/arm64");
        assert_eq!(contract.launcher_arguments().arguments().len(), 3);
        assert_eq!(contract.user().uid(), 10_002);
        assert!(RuntimeUser::new(0, 10_001).is_err());
    }
}
