//! Provider boundary shared by research and coding workers.
use rig_core::providers::openai;

/// Construct the only enabled completion provider. Credentials never appear in errors.
///
/// # Errors
/// Rejects unsupported providers, blank credentials, or client initialization failure.
pub fn openai_client(provider: &str, api_key: &str) -> Result<openai::Client, &'static str> {
    if provider != "openai" {
        return Err("provider_not_available");
    }
    let key = api_key.trim();
    if key.is_empty() || key.len() > 4096 || key.chars().any(char::is_control) {
        return Err("provider_credentials_invalid");
    }
    openai::Client::new(key).map_err(|_| "provider_initialization_failed")
}

#[cfg(test)]
mod tests {
    #[test]
    fn unavailable_providers_and_invalid_keys_are_rejected() {
        assert!(super::openai_client("anthropic", "key").is_err());
        assert!(super::openai_client("bedrock", "key").is_err());
        assert!(super::openai_client("openai", " ").is_err());
        assert!(super::openai_client("openai", "key\nother").is_err());
        assert!(super::openai_client("openai", "key").is_ok());
    }
}
