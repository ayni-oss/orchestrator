//! Provider boundary shared by research and coding workers.
use rig_core::providers::openai;

pub mod coding;
mod plan_proposals;
pub mod responses;

/// Construct the only enabled completion provider. Credentials never appear in errors.
///
/// # Errors
/// Rejects unsupported providers, blank credentials, invalid operator endpoints,
/// or client initialization failure. The API key always comes from the caller.
pub fn openai_client(provider: &str, api_key: &str) -> Result<openai::Client, &'static str> {
    let endpoint = match std::env::var("OPENAI_BASE_URL") {
        Ok(endpoint) => Some(endpoint),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => return Err("provider_endpoint_invalid"),
    };
    configured_openai_client(provider, api_key, endpoint.as_deref())
}

fn configured_openai_client(
    provider: &str,
    api_key: &str,
    endpoint: Option<&str>,
) -> Result<openai::Client, &'static str> {
    if provider != "openai" {
        return Err("provider_not_available");
    }
    let key = api_key.trim();
    if key.is_empty() || key.len() > 4096 || key.chars().any(char::is_control) {
        return Err("provider_credentials_invalid");
    }
    let mut builder = openai::Client::builder().api_key(key);
    if let Some(endpoint) = endpoint {
        let parsed = url::Url::parse(endpoint).map_err(|_| "provider_endpoint_invalid")?;
        let authority = endpoint
            .split_once("://")
            .map(|(_, rest)| rest.split('/').next().unwrap_or_default())
            .ok_or("provider_endpoint_invalid")?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none_or(str::is_empty)
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || authority.contains('@')
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err("provider_endpoint_invalid");
        }
        builder = builder.base_url(parsed.as_str());
    }
    builder
        .build()
        .map_err(|_| "provider_initialization_failed")
}

#[cfg(test)]
mod tests {
    #[test]
    fn unavailable_providers_and_invalid_keys_are_rejected() {
        assert!(super::configured_openai_client("anthropic", "key", None).is_err());
        assert!(super::configured_openai_client("bedrock", "key", None).is_err());
        assert!(super::configured_openai_client("openai", " ", None).is_err());
        assert!(super::configured_openai_client("openai", "key\nother", None).is_err());
        assert!(super::configured_openai_client("openai", "key", None).is_ok());
    }

    #[test]
    fn operator_endpoint_changes_only_the_provider_base_url() -> Result<(), &'static str> {
        let default = super::configured_openai_client("openai", "mounted-key", None)?;
        assert_eq!(default.base_url(), "https://api.openai.com/v1");
        for endpoint in ["http://planner-stub:8080/v1", "https://gateway.example/v1"] {
            let configured =
                super::configured_openai_client("openai", "mounted-key", Some(endpoint))?;
            assert_eq!(configured.base_url(), endpoint);
            assert_eq!(
                configured
                    .headers()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer mounted-key")
            );
        }
        Ok(())
    }

    #[test]
    fn malformed_or_credential_bearing_operator_endpoints_are_rejected() {
        for endpoint in [
            "",
            "not a URL",
            "file:///tmp/provider",
            "ftp://host/v1",
            "https:///",
            "https://user@host/v1",
            "https://user:password@host/v1",
            "https://@host/v1",
            "https:@host/v1",
            "https://host/v1?key=value",
            "https://host/v1#fragment",
        ] {
            assert!(
                super::configured_openai_client("openai", "mounted-key", Some(endpoint)).is_err()
            );
        }
    }
}
