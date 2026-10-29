//! Private GHCR protocol primitives; callers retain operation-specific authority.
mod discovery;
pub use discovery::{Candidate, Discovery, DiscoveryError, GhcrClient, Platform};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

/// Parsed Docker credentials. Deliberately has no Debug or serialization implementation.
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// Decode credentials for the fixed GHCR authority only.
/// # Errors
/// Reject malformed, oversized, missing, or empty credentials.
pub fn decode_credentials(data: &[u8]) -> Result<Credentials, &'static str> {
    if data.len() > 65_536 {
        return Err("registry_credentials_invalid");
    }
    let value: serde_json::Value =
        serde_json::from_slice(data).map_err(|_| "registry_credentials_invalid")?;
    let entry = value
        .get("auths")
        .and_then(|auths| auths.get("ghcr.io"))
        .ok_or("registry_credentials_invalid")?;
    let decoded;
    let (username, password) = if let Some(auth) = entry.get("auth").and_then(|v| v.as_str()) {
        decoded = STANDARD
            .decode(auth)
            .map_err(|_| "registry_credentials_invalid")?;
        let pair = std::str::from_utf8(&decoded).map_err(|_| "registry_credentials_invalid")?;
        pair.split_once(':').ok_or("registry_credentials_invalid")?
    } else {
        (
            entry
                .get("username")
                .and_then(|v| v.as_str())
                .ok_or("registry_credentials_invalid")?,
            entry
                .get("password")
                .and_then(|v| v.as_str())
                .ok_or("registry_credentials_invalid")?,
        )
    };
    if username.is_empty() || password.is_empty() {
        return Err("registry_credentials_invalid");
    }
    Ok(Credentials {
        username: username.into(),
        password: password.into(),
    })
}

/// Read a bounded trusted credential mount, including Kubernetes projected symlinks.
/// # Errors
/// Reject missing, non-file, or oversized credential input.
pub fn read_credentials(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 65_536 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid or oversized registry credentials",
        ));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(65_537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65_536 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oversized registry credentials",
        ));
    }
    Ok(bytes)
}

/// Read a provider response without buffering beyond its operation-specific limit.
/// # Errors
/// Reject oversized or interrupted responses without retaining provider diagnostics.
pub async fn bounded_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, &'static str> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err("registry_response_oversized");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "registry_response_unavailable")?
    {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err("registry_response_oversized");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Construct a token request at the fixed GHCR authority.
/// The caller selects its operation's exact action scope and handles response policy.
pub fn token_request(
    client: &reqwest::Client,
    repository: &str,
    credentials: Option<&Credentials>,
    actions: &str,
) -> reqwest::RequestBuilder {
    let mut request = client.get("https://ghcr.io/token").query(&[
        ("service", "ghcr.io"),
        (
            "scope",
            format!("repository:{repository}:{actions}").as_str(),
        ),
    ]);
    if let Some(credentials) = credentials {
        request = request.basic_auth(&credentials.username, Some(&credentials.password));
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_require_the_exact_registry_and_nonempty_pair() -> Result<(), &'static str> {
        let credentials =
            decode_credentials(br#"{"auths":{"ghcr.io":{"auth":"dXNlcjpwYXNzOndvcmQ="}}}"#)?;
        assert_eq!(credentials.username, "user");
        assert_eq!(credentials.password, "pass:word");
        let credentials = decode_credentials(
            br#"{"auths":{"ghcr.io":{"username":"user","password":"password"}}}"#,
        )?;
        assert_eq!(credentials.username, "user");
        for invalid in [
            br#"{"auths":{"evil.test":{"username":"user","password":"password"}}}"#.as_slice(),
            br#"{"auths":{"ghcr.io":{"auth":"Og=="}}}"#,
            br#"{"auths":{"ghcr.io":{"username":"","password":"password"}}}"#,
            br#"{"auths":{"ghcr.io":{"auth":"not base64","username":"fallback","password":"password"}}}"#,
        ] { assert!(decode_credentials(invalid).is_err()); }
        assert!(decode_credentials(&vec![b' '; 65_537]).is_err());
        Ok(())
    }

    #[test]
    fn token_scope_cannot_change_credential_destination() -> Result<(), Box<dyn std::error::Error>>
    {
        let credentials = Credentials {
            username: "fixture".into(),
            password: "fixture".into(),
        };
        let request = token_request(
            &reqwest::Client::new(),
            "team/runtime?redirect=evil.test",
            Some(&credentials),
            "pull",
        )
        .build()?;
        assert_eq!(request.url().host_str(), Some("ghcr.io"));
        assert_eq!(request.url().path(), "/token");
        assert_eq!(
            request
                .url()
                .query_pairs()
                .find(|(key, _)| key == "scope")
                .map(|(_, value)| value.into_owned()),
            Some("repository:team/runtime?redirect=evil.test:pull".into())
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires loopback networking; run scripts/ci/check-workflow-loopback.sh"]
    async fn streaming_body_rejects_oversized_chunked_responses_requires_loopback()
    -> Result<(), Box<dyn std::error::Error>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0; 4096];
                let _ = stream.read(&mut request).await;
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\n1234\r\n4\r\n5678\r\n0\r\n\r\n").await;
            }
        });
        let response = reqwest::Client::new()
            .get(format!("http://{address}"))
            .send()
            .await?;
        assert_eq!(
            bounded_body(response, 6).await,
            Err("registry_response_oversized")
        );
        Ok(())
    }
}
