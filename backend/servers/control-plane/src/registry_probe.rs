//! GHCR-only probe executed in an isolated Job, never inside the API process.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, time::Duration};

const MANIFEST_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const CONFIG_TYPE: &str = "application/vnd.oci.image.config.v1+json";

/// Safe test evidence. All errors are bounded codes, never provider response text.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent probe outcomes are observations, not a lifecycle state"
)]
pub struct RegistryTestReport {
    pub authentication: bool,
    pub push: bool,
    pub pull: bool,
    pub cleanup: bool,
    pub failure_reason: String,
    pub cleanup_failure_reason: String,
    pub artifact_repository: String,
    pub artifact_digest: String,
}

struct Credentials {
    username: String,
    password: String,
}

fn credentials(path: &str) -> Result<Credentials, &'static str> {
    let data = fs::read(path).map_err(|_| "registry_credentials_unavailable")?;
    if data.len() > 65_536 {
        return Err("registry_credentials_invalid");
    }
    let value: serde_json::Value =
        serde_json::from_slice(&data).map_err(|_| "registry_credentials_invalid")?;
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

/// Validates a GHCR namespace without permitting hostnames or credential destinations.
#[must_use]
pub fn valid_namespace(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with(['-', '.', '_'])
                && !part.ends_with(['-', '.', '_'])
                && part.bytes().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_' | b'.')
                })
        })
}

fn artifact_repository(namespace: &str, operation: &str) -> Result<String, &'static str> {
    if !valid_namespace(namespace)
        || operation.is_empty()
        || operation.len() > 64
        || !operation
            .bytes()
            .all(|v| v.is_ascii_lowercase() || v.is_ascii_digit() || v == b'-')
    {
        return Err("registry_test_identity_invalid");
    }
    Ok(format!("{namespace}/ayni-connection-test-{operation}"))
}

fn allowed_location(location: &str) -> Result<Url, &'static str> {
    let base = Url::parse("https://ghcr.io").map_err(|_| "registry_upload_location_invalid")?;
    let url = base
        .join(location)
        .map_err(|_| "registry_upload_location_invalid")?;
    if url.scheme() != "https"
        || url.host_str() != Some("ghcr.io")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().starts_with("/v2/")
    {
        return Err("registry_upload_location_invalid");
    }
    Ok(url)
}

async fn auth(
    client: &Client,
    repository: &str,
    credentials: Option<&Credentials>,
    actions: &str,
) -> Result<String, &'static str> {
    let scope = format!("repository:{repository}:{actions}");
    let mut request = client
        .get("https://ghcr.io/token")
        .query(&[("service", "ghcr.io"), ("scope", scope.as_str())]);
    if let Some(credentials) = credentials {
        request = request.basic_auth(&credentials.username, Some(&credentials.password));
    }
    let response = request
        .send()
        .await
        .map_err(|_| "registry_authentication_unavailable")?;
    if !response.status().is_success() {
        return Err("registry_authentication_failed");
    }
    let response: serde_json::Value = serde_json::from_slice(&bounded_body(response).await?)
        .map_err(|_| "registry_authentication_invalid")?;
    response
        .get("token")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty() && v.len() < 65_536)
        .map(str::to_owned)
        .ok_or("registry_authentication_invalid")
}

fn pull_only_scopes(scopes: &str) -> bool {
    let scopes: Vec<_> = scopes.split(',').map(str::trim).collect();
    scopes.contains(&"read:packages")
        && !scopes
            .iter()
            .any(|scope| matches!(*scope, "write:packages" | "delete:packages"))
}

async fn verify_pull_only(client: &Client, credentials: &Credentials) -> Result<(), &'static str> {
    let response = client
        .get("https://api.github.com/user")
        .bearer_auth(&credentials.password)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", "ayni-orchestrator")
        .header("x-github-api-version", "2022-11-28")
        .send()
        .await
        .map_err(|_| "registry_pull_authority_unavailable")?;
    if !response.status().is_success() {
        return Err("registry_pull_authority_invalid");
    }
    let scopes = response
        .headers()
        .get("x-oauth-scopes")
        .and_then(|v| v.to_str().ok())
        .ok_or("registry_pull_authority_unverifiable")?;
    if !pull_only_scopes(scopes) {
        return Err("registry_pull_only_authority_required");
    }
    Ok(())
}

async fn upload_blob(
    client: &Client,
    repository: &str,
    token: &str,
    blob: &[u8],
) -> Result<String, &'static str> {
    let digest = format!("sha256:{:x}", Sha256::digest(blob));
    let response = client
        .post(format!("https://ghcr.io/v2/{repository}/blobs/uploads/"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "registry_push_unavailable")?;
    if response.status() != StatusCode::ACCEPTED {
        return Err("registry_push_failed");
    }
    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or("registry_upload_location_invalid")?;
    let mut url = allowed_location(location)?;
    url.query_pairs_mut().append_pair("digest", &digest);
    let response = client
        .put(url)
        .bearer_auth(token)
        .header("content-type", "application/octet-stream")
        .body(blob.to_vec())
        .send()
        .await
        .map_err(|_| "registry_push_unavailable")?;
    if response.status() != StatusCode::CREATED {
        return Err("registry_push_failed");
    }
    Ok(digest)
}

async fn push(client: &Client, repository: &str, token: &str) -> Result<String, &'static str> {
    let config = br#"{"architecture":"arm64","os":"linux","rootfs":{"type":"layers","diff_ids":[]},"config":{}}"#;
    let digest = upload_blob(client, repository, token, config).await?;
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schemaVersion":2,"mediaType":MANIFEST_TYPE,
        "config":{"mediaType":CONFIG_TYPE,"digest":digest,"size":config.len()},"layers":[]
    }))
    .map_err(|_| "registry_manifest_invalid")?;
    let digest = format!("sha256:{:x}", Sha256::digest(&manifest));
    let response = client
        .put(format!("https://ghcr.io/v2/{repository}/manifests/probe"))
        .bearer_auth(token)
        .header("content-type", MANIFEST_TYPE)
        .body(manifest)
        .send()
        .await
        .map_err(|_| "registry_push_unavailable")?;
    if response.status() != StatusCode::CREATED {
        return Err("registry_push_failed");
    }
    Ok(digest)
}

async fn bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, &'static str> {
    if response.content_length().is_some_and(|size| size > 65_536) {
        return Err("registry_response_oversized");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "registry_response_unavailable")?
    {
        if bytes.len() + chunk.len() > 65_536 {
            return Err("registry_response_oversized");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn cleanup(
    client: &Client,
    repository: &str,
    digest: &str,
    credentials: &Credentials,
) -> Result<(), &'static str> {
    let (owner, package) = repository
        .split_once('/')
        .ok_or("registry_cleanup_identity_invalid")?;
    // GHCR deletion is a GitHub package-version operation, not OCI DELETE.
    // Only the uniquely probe-owned package and exact digest can be deleted.
    let mut versions = None;
    for owner_kind in ["orgs", "users"] {
        let mut url = Url::parse("https://api.github.com")
            .map_err(|_| "registry_cleanup_identity_invalid")?;
        url.path_segments_mut()
            .map_err(|()| "registry_cleanup_identity_invalid")?
            .extend([
                owner_kind,
                owner,
                "packages",
                "container",
                package,
                "versions",
            ]);
        url.query_pairs_mut().append_pair("per_page", "100");
        let response = client
            .get(url.clone())
            .bearer_auth(&credentials.password)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "ayni-orchestrator")
            .header("x-github-api-version", "2022-11-28")
            .send()
            .await
            .map_err(|_| "registry_cleanup_unavailable")?;
        if response.status() == StatusCode::NOT_FOUND {
            continue;
        }
        if !response.status().is_success() {
            return Err("registry_cleanup_permission_required");
        }
        let body = bounded_body(response).await?;
        let entries: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| "registry_cleanup_result_invalid")?;
        let entries = entries
            .as_array()
            .ok_or("registry_cleanup_result_invalid")?;
        let Some(entry) = entries
            .iter()
            .find(|entry| entry["name"].as_str() == Some(digest))
        else {
            return Ok(()); // Exact version is already absent from a confirmed package.
        };
        let id = entry["id"]
            .as_u64()
            .ok_or("registry_cleanup_result_invalid")?;
        versions = Some((url, id));
        break;
    }
    let (mut url, id) = versions.ok_or("registry_cleanup_permission_required")?;
    url.set_query(None);
    url.path_segments_mut()
        .map_err(|()| "registry_cleanup_identity_invalid")?
        .push(&id.to_string());
    let response = client
        .delete(url)
        .bearer_auth(&credentials.password)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", "ayni-orchestrator")
        .header("x-github-api-version", "2022-11-28")
        .send()
        .await
        .map_err(|_| "registry_cleanup_unavailable")?;
    if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
        Ok(())
    } else {
        Err("registry_cleanup_permission_required")
    }
}

fn probe_client() -> Result<Client, &'static str> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| "registry_test_client_unavailable")
}

fn checkpoint(report: &RegistryTestReport) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec(report).map_err(|_| "registry_test_evidence_invalid")?;
    fs::write("/dev/termination-log", bytes).map_err(|_| "registry_test_evidence_unavailable")
}

/// Runs independent authentication/push/pull checks; always attempts cleanup after push.
///
/// # Errors
/// Returns a bounded code for invalid local inputs before a probe can run.
pub async fn run(
    namespace: &str,
    operation: &str,
    push_path: &str,
    pull_path: Option<&str>,
    cleanup_digest: Option<&str>,
) -> Result<RegistryTestReport, &'static str> {
    let repository = artifact_repository(namespace, operation)?;
    let push_credentials = credentials(push_path)?;
    let pull_credentials = pull_path.map(credentials).transpose()?;
    let client = probe_client()?;
    let mut report = RegistryTestReport {
        artifact_repository: repository.clone(),
        failure_reason: "registry_test_interrupted".into(),
        ..RegistryTestReport::default()
    };
    checkpoint(&report)?;
    let token = match auth(&client, &repository, Some(&push_credentials), "pull,push").await {
        Ok(token) => {
            report.authentication = true;
            token
        }
        Err(reason) => {
            report.failure_reason = reason.into();
            return Ok(report);
        }
    };
    if let Some(digest) = cleanup_digest {
        if digest.len() != 71
            || !digest.starts_with("sha256:")
            || !digest[7..].bytes().all(|v| v.is_ascii_hexdigit())
        {
            return Err("registry_cleanup_digest_invalid");
        }
        report.artifact_digest = digest.into();
        report.cleanup_failure_reason = "registry_cleanup_pending".into();
        report.failure_reason.clear();
    } else {
        match push(&client, &repository, &token).await {
            Ok(digest) => {
                report.push = true;
                report.artifact_digest = digest;
                report.cleanup_failure_reason = "registry_cleanup_pending".into();
                checkpoint(&report)?;
            }
            Err(reason) => {
                report.failure_reason = reason.into();
                return Ok(report);
            }
        }
        let pull = async {
            if let Some(credentials) = &pull_credentials {
                verify_pull_only(&client, credentials).await?;
            }
            let pull_token = auth(&client, &repository, pull_credentials.as_ref(), "pull").await?;
            let response = client
                .get(format!(
                    "https://ghcr.io/v2/{repository}/manifests/{}",
                    report.artifact_digest
                ))
                .bearer_auth(pull_token)
                .header("accept", MANIFEST_TYPE)
                .send()
                .await
                .map_err(|_| "registry_pull_unavailable")?;
            if !response.status().is_success() {
                return Err("registry_pull_failed");
            }
            let bytes = bounded_body(response).await?;
            if format!("sha256:{:x}", Sha256::digest(&bytes)) != report.artifact_digest {
                return Err("registry_pull_digest_mismatch");
            }
            Ok(())
        }
        .await;
        match pull {
            Ok(()) => {
                report.pull = true;
                report.failure_reason.clear();
            }
            Err(reason) => report.failure_reason = reason.into(),
        }
    }
    checkpoint(&report)?;
    match cleanup(
        &client,
        &repository,
        &report.artifact_digest,
        &push_credentials,
    )
    .await
    {
        Ok(()) => {
            report.cleanup = true;
            report.cleanup_failure_reason.clear();
        }
        Err(reason) => report.cleanup_failure_reason = reason.into(),
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespaces_cannot_change_credential_destinations() {
        assert!(valid_namespace("acme/runtimes"));
        for value in [
            "",
            "https://evil.test",
            "acme/../other",
            "acme//x",
            "ACME",
            "evil.test:443",
            "-acme",
        ] {
            assert!(!valid_namespace(value), "{value}");
        }
    }
    #[test]
    fn upload_locations_cannot_forward_tokens_to_another_authority() {
        assert!(allowed_location("/v2/acme/probe/blobs/uploads/id?x=y").is_ok());
        for value in [
            "https://evil.test/v2/upload",
            "http://ghcr.io/v2/upload",
            "https://user@ghcr.io/v2/upload",
            "https://ghcr.io:444/v2/upload",
            "//evil.test/v2/upload",
            "/token",
        ] {
            assert!(allowed_location(value).is_err(), "{value}");
        }
    }
    async fn fixture(response: &'static [u8]) -> Result<String, Box<dyn std::error::Error>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = [0; 4096];
                let _ = stream.read(&mut buffer).await;
                let _ = stream.write_all(response).await;
            }
        });
        Ok(format!("http://{address}"))
    }
    #[tokio::test]
    async fn provider_responses_are_bounded_before_parsing()
    -> Result<(), Box<dyn std::error::Error>> {
        let url =
            fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\n")
                .await?;
        let response = Client::new().get(url).send().await?;
        assert_eq!(
            bounded_body(response).await,
            Err("registry_response_oversized")
        );
        let url =
            fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await?;
        let response = Client::new().get(url).send().await?;
        assert_eq!(bounded_body(response).await, Ok(b"{}".to_vec()));
        Ok(())
    }
    #[tokio::test]
    async fn probe_client_never_follows_authentication_redirects()
    -> Result<(), Box<dyn std::error::Error>> {
        let url=fixture(b"HTTP/1.1 302 Found\r\nLocation: https://evil.test/token\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
        let response = probe_client()?
            .get(url)
            .basic_auth("fixture-user", Some("fixture-password"))
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::FOUND);
        Ok(())
    }
    #[test]
    fn candidate_tokens_require_read_only_package_scopes() {
        assert!(pull_only_scopes("read:packages"));
        assert!(pull_only_scopes("repo, read:packages"));
        for scopes in [
            "",
            "repo",
            "read:packages, write:packages",
            "read:packages,delete:packages",
            "write:packages",
        ] {
            assert!(!pull_only_scopes(scopes), "{scopes}");
        }
    }
}
