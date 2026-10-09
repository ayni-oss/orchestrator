//! Provider-independent OCI identity parsing. Provider admission stays with callers.

/// A complete lowercase SHA-256 content identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sha256Digest<'a>(&'a str);

impl<'a> Sha256Digest<'a> {
    /// # Errors
    /// Reject incomplete or noncanonical digests.
    pub fn parse(value: &'a str) -> Result<Self, &'static str> {
        value
            .strip_prefix("sha256:")
            .is_some_and(|hex| {
                hex.len() == 64
                    && hex
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            })
            .then_some(Self(value))
            .ok_or("invalid_sha256_digest")
    }
    #[must_use]
    pub const fn as_str(self) -> &'a str {
        self.0
    }
    #[must_use]
    pub fn hex(self) -> &'a str {
        &self.0[7..]
    }
}

/// An OCI repository path without a tag or digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageRepository<'a>(&'a str);

impl<'a> ImageRepository<'a> {
    /// # Errors
    /// Reject invalid path components, noncanonical characters and tag suffixes.
    pub fn parse(value: &'a str) -> Result<Self, &'static str> {
        if value.is_empty() || value.len() > 512 {
            return Err("invalid_image_repository");
        }
        let mut parts = value.split('/');
        let first = parts.next().ok_or("invalid_image_repository")?;
        let remaining: Vec<_> = parts.collect();
        let component = |part: &str| {
            !part.is_empty()
                && part.len() <= 255
                && part
                    .bytes()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                && part
                    .bytes()
                    .last()
                    .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                && part.bytes().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-')
                })
        };
        let first_valid = if let Some((host, port)) = first.split_once(':') {
            !remaining.is_empty()
                && component(host)
                && !host.contains('_')
                && !port.is_empty()
                && port.bytes().all(|c| c.is_ascii_digit())
        } else {
            component(first) && (remaining.is_empty() || !first.contains('_'))
        };
        if value.len() > 512 || !first_valid || !remaining.iter().all(|part| component(part)) {
            return Err("invalid_image_repository");
        }
        Ok(Self(value))
    }
    #[must_use]
    pub const fn as_str(self) -> &'a str {
        self.0
    }
}

/// A bounded OCI tag used for discovery, never execution admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageTag<'a>(&'a str);
impl<'a> ImageTag<'a> {
    /// # Errors
    /// Reject tags outside the OCI tag grammar.
    pub fn parse(value: &'a str) -> Result<Self, &'static str> {
        if value.is_empty()
            || value.len() > 128
            || value.starts_with(['.', '-'])
            || !value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
        {
            return Err("invalid_image_tag");
        }
        Ok(Self(value))
    }
    #[must_use]
    pub const fn as_str(self) -> &'a str {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identities_cannot_hide_tags_paths_or_authorities() {
        for value in [
            "ghcr.io/team/runtime",
            "localhost:5000/team/runtime",
            "runtime",
        ] {
            assert!(ImageRepository::parse(value).is_ok(), "{value}");
        }
        for value in [
            "",
            "ghcr.io//runtime",
            "ghcr.io/../runtime",
            "ghcr.io/runtime:tag",
            "https://ghcr.io/runtime",
            "ghcr.io/team/",
            "host:port/runtime",
        ] {
            assert!(ImageRepository::parse(value).is_err(), "{value}");
        }
        for value in ["v0.15.0", "_test", "sha-abc"] {
            assert!(ImageTag::parse(value).is_ok());
        }
        for value in ["", ".tag", "-tag", "tag/name", "tag@digest"] {
            assert!(ImageTag::parse(value).is_err());
        }
        assert!(ImageRepository::parse(&"a/".repeat(1024)).is_err());
        assert!(Sha256Digest::parse(&format!("sha256:{}", "a".repeat(64))).is_ok());
        assert!(Sha256Digest::parse(&format!("sha256:{}", "A".repeat(64))).is_err());
    }
}
