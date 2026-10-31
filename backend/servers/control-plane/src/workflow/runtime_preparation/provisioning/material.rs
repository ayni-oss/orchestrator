use ed25519_dalek::SigningKey;
use zeroize::Zeroizing;

pub(super) struct Material {
    pub(super) seed: Zeroizing<String>,
    pub(super) public_key: String,
}
impl Material {
    pub(super) fn imported(value: &str) -> Result<Self, &'static str> {
        let value = value.strip_suffix('\n').unwrap_or(value);
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("signing seed must contain 64 lower-case hexadecimal characters");
        }
        let mut bytes = Zeroizing::new([0u8; 32]);
        for (slot, pair) in bytes.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
            *slot = digit(pair[0]) * 16 + digit(pair[1]);
        }
        Ok(Self {
            seed: Zeroizing::new(value.to_owned()),
            public_key: public(&bytes),
        })
    }
    pub(super) fn generated() -> Result<Self, &'static str> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(bytes.as_mut()).map_err(|_| "signer_provisioning_unavailable")?;
        Ok(Self {
            seed: Zeroizing::new(hex(bytes.as_ref())),
            public_key: public(&bytes),
        })
    }
}
const fn digit(value: u8) -> u8 {
    if value.is_ascii_digit() {
        value - b'0'
    } else {
        value - b'a' + 10
    }
}
fn public(seed: &[u8; 32]) -> String {
    hex(SigningKey::from_bytes(seed).verifying_key().as_bytes())
}
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 15)]));
    }
    result
}
#[cfg(test)]
mod tests {
    use super::Material;
    #[test]
    fn canonical_seed_matches_official_ed25519_vector_and_rejects_noncanonical_uploads()
    -> Result<(), &'static str> {
        let seed = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
        let material = Material::imported(seed)?;
        assert_eq!(
            material.public_key,
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        assert_eq!(
            Material::imported(&format!("{seed}\n"))?.public_key,
            material.public_key
        );
        for invalid in [
            seed.to_uppercase(),
            format!(" {seed}"),
            format!("{seed}\n\n"),
            "a".repeat(65),
        ] {
            assert!(Material::imported(&invalid).is_err());
        }
        Ok(())
    }
    #[test]
    fn generation_uses_independent_full_entropy_and_same_public_derivation()
    -> Result<(), &'static str> {
        let first = Material::generated()?;
        let second = Material::generated()?;
        assert_ne!(first.public_key, second.public_key);
        assert_eq!(
            Material::imported(&first.seed)?.public_key,
            first.public_key
        );
        Ok(())
    }
}
