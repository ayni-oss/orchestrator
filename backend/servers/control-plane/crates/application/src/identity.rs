//! Validated identities used by control-plane decisions and provider ports.
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRevision(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("Git revision must be a full hexadecimal commit identity")]
pub struct InvalidGitRevision;

impl GitRevision {
    /// Preserve the exact frozen representation while validating its syntax.
    ///
    /// # Errors
    /// Rejects branch names, abbreviated hashes and unsafe argument text.
    pub fn parse(value: &str) -> Result<Self, InvalidGitRevision> {
        if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(InvalidGitRevision);
        }
        Ok(Self(value.into()))
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_representation_is_preserved_and_unbound_references_are_rejected()
    -> Result<(), InvalidGitRevision> {
        let value = "A".repeat(40);
        assert_eq!(GitRevision::parse(&value)?.as_str(), value);
        for value in [
            "main",
            "aabbcc",
            "--upload-pack=other",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        ] {
            assert!(GitRevision::parse(value).is_err());
        }
        Ok(())
    }
}
