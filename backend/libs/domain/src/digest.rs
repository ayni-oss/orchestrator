//! Bounded hexadecimal identity and content digests.
use sha2::{Digest, Sha256};

#[must_use]
pub fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[must_use]
pub fn digest(value: &str) -> bool {
    crate::oci::Sha256Digest::parse(value).is_ok()
}

#[must_use]
pub fn hex(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}
