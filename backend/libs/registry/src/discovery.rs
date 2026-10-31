//! Immutable GHCR discovery only. No image is executed or declared verified here.
//!
//! OCI permits blob redirects and prohibits forwarding Authorization across hosts:
//! <https://github.com/opencontainers/distribution-spec/blob/main/spec.md#endpoints>.
//! GitHub lists its container CDN at:
//! <https://docs.github.com/en/actions/reference/runners/github-hosted-runners#communication-requirements-for-github-hosted-runners>.
//! Only blob GETs may follow at most two redirects to that exact HTTPS CDN host.
//! Token and manifest requests never redirect. CDN requests carry no credentials;
//! returned bytes must still match the original immutable descriptor digest.
use crate::{Credentials, bounded_body, token_request};
use ayni_orchestrator_oci::{ImageRepository, ImageTag, Sha256Digest};
use reqwest::{Client, RequestBuilder, Response, StatusCode, Url};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::{fmt, time::Duration};

const LIMIT: usize = 4 * 1024 * 1024;
const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const DOCKER_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json";
const OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
const DOCKER_INDEX: &str = "application/vnd.docker.distribution.manifest.list.v2+json";
const OCI_CONFIG: &str = "application/vnd.oci.image.config.v1+json";
const DOCKER_CONFIG: &str = "application/vnd.docker.container.image.v1+json";
const ACCEPT: &str = "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json";

/// Supported Linux image platforms. Platform selection never uses the control-plane host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    LinuxAmd64,
    LinuxArm64,
}
impl Platform {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LinuxAmd64 => "linux/amd64",
            Self::LinuxArm64 => "linux/arm64",
        }
    }
    const fn architecture(self) -> &'static str {
        match self {
            Self::LinuxAmd64 => "amd64",
            Self::LinuxArm64 => "arm64",
        }
    }
}
/// Discovery evidence; independent runtime verification is still required.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// Exact platform manifest reference, suitable for a later independent pull.
    pub image: String,
    /// Digest of the tag's index or direct manifest bytes.
    pub index_digest: String,
    pub manifest_digest: String,
    pub config_digest: String,
    pub platform: Platform,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Discovery {
    Missing,
    Candidate(Candidate),
}
/// Closed diagnostics. Provider text and credentials never enter returned errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveryError {
    InvalidInput,
    AccessDenied,
    RateLimited,
    Unavailable,
    ResponseInvalid,
    ResponseOversized,
    PlatformUnavailable,
}
impl DiscoveryError {
    #[must_use]
    pub const fn safe_reason(self) -> &'static str {
        match self {
            Self::InvalidInput => "registry_input_invalid",
            Self::AccessDenied => "registry_access_denied",
            Self::RateLimited => "registry_rate_limited",
            Self::Unavailable => "registry_unavailable",
            Self::ResponseInvalid => "registry_response_invalid",
            Self::ResponseOversized => "registry_response_oversized",
            Self::PlatformUnavailable => "registry_platform_unavailable",
        }
    }
}
impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.safe_reason())
    }
}
impl std::error::Error for DiscoveryError {}

/// Fixed-origin GHCR reader. It never follows redirects or uses ambient credentials.
pub struct GhcrClient {
    client: Client,
}
impl GhcrClient {
    /// # Errors
    /// Return a safe unavailable diagnostic if the HTTP client cannot be initialized.
    pub fn new() -> Result<Self, DiscoveryError> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| DiscoveryError::Unavailable)?,
        })
    }
    /// Resolve one configured destination tag and platform with the supplied pull credential.
    /// A successful result is discovery evidence, never verified runtime readiness.
    /// # Errors
    /// Reject unsafe identities, authentication/transport failures, malformed or oversized
    /// objects, digest mismatches and absent platforms. Only the initial manifest 404 is Missing.
    pub async fn discover(
        &self,
        repository: &str,
        tag: &str,
        platform: Platform,
        credentials: &Credentials,
    ) -> Result<Discovery, DiscoveryError> {
        validate_input(repository, tag, credentials)?;
        tokio::time::timeout(
            Duration::from_secs(20),
            self.resolve(repository, tag, platform, credentials),
        )
        .await
        .map_err(|_| DiscoveryError::Unavailable)?
    }
    async fn resolve(
        &self,
        repository: &str,
        tag: &str,
        platform: Platform,
        credentials: &Credentials,
    ) -> Result<Discovery, DiscoveryError> {
        let path = repository
            .strip_prefix("ghcr.io/")
            .ok_or(DiscoveryError::InvalidInput)?;
        let token = self.pull_token(path, credentials).await?;
        let Some(root) = self.manifest(path, tag, &token, true).await? else {
            return Ok(Discovery::Missing);
        };
        let index_digest = root.digest.clone();
        let manifest = if is_index(&root.media_type) {
            let descriptor = select_platform(&root.value, platform)?;
            let manifest = self
                .manifest(path, &descriptor.digest, &token, false)
                .await?
                .ok_or(DiscoveryError::ResponseInvalid)?;
            if manifest.size != descriptor.size
                || manifest.media_type != descriptor.media_type
                || is_index(&manifest.media_type)
            {
                return Err(DiscoveryError::ResponseInvalid);
            }
            manifest
        } else {
            root
        };
        let descriptor = config_descriptor(&manifest.value)?;
        let response = self.blob(path, &descriptor.digest, &token).await?;
        let claimed = claimed_digest(&response)?;
        let bytes = body(response).await?;
        validate_digest(&bytes, claimed.as_deref(), Some(&descriptor.digest))?;
        if bytes.len() != descriptor.size {
            return Err(DiscoveryError::ResponseInvalid);
        }
        let config: Value =
            serde_json::from_slice(&bytes).map_err(|_| DiscoveryError::ResponseInvalid)?;
        if !matches_platform(&config, platform) {
            return Err(DiscoveryError::PlatformUnavailable);
        }
        Ok(Discovery::Candidate(Candidate {
            image: format!("{repository}@{}", manifest.digest),
            index_digest,
            manifest_digest: manifest.digest,
            config_digest: descriptor.digest,
            platform,
        }))
    }
    fn origin_blob_request(&self, repository: &str, digest: &str, token: &str) -> RequestBuilder {
        self.client
            .get(format!("https://ghcr.io/v2/{repository}/blobs/{digest}"))
            .bearer_auth(token)
    }
    // No token/credential parameter exists at this cross-origin boundary.
    fn cdn_blob_request(&self, url: Url) -> RequestBuilder {
        self.client.get(url)
    }
    async fn blob(
        &self,
        repository: &str,
        digest: &str,
        token: &str,
    ) -> Result<Response, DiscoveryError> {
        let mut response = self
            .origin_blob_request(repository, digest, token)
            .send()
            .await
            .map_err(|_| DiscoveryError::Unavailable)?;
        let mut hops = 0;
        loop {
            if let Some(claimed) = claimed_digest(&response)? {
                Sha256Digest::parse(&claimed).map_err(|_| DiscoveryError::ResponseInvalid)?;
                if claimed != digest {
                    return Err(DiscoveryError::ResponseInvalid);
                }
            }
            if !matches!(response.status().as_u16(), 302 | 307) {
                status(response.status(), false)?;
                return Ok(response);
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .ok_or(DiscoveryError::ResponseInvalid)?;
            let url = cdn_redirect(location, hops)?;
            hops += 1;
            response = self
                .cdn_blob_request(url)
                .send()
                .await
                .map_err(|_| DiscoveryError::Unavailable)?;
        }
    }
    async fn pull_token(
        &self,
        repository: &str,
        credentials: &Credentials,
    ) -> Result<String, DiscoveryError> {
        let response = token_request(&self.client, repository, Some(credentials), "pull")
            .send()
            .await
            .map_err(|_| DiscoveryError::Unavailable)?;
        status(response.status(), false)?;
        let value: Value = serde_json::from_slice(&body(response).await?)
            .map_err(|_| DiscoveryError::ResponseInvalid)?;
        value
            .get("token")
            .or_else(|| value.get("access_token"))
            .and_then(Value::as_str)
            .filter(|token| {
                !token.is_empty()
                    && token.len() <= 8192
                    && token.bytes().all(|v| v.is_ascii_graphic())
            })
            .map(str::to_owned)
            .ok_or(DiscoveryError::ResponseInvalid)
    }
    async fn manifest(
        &self,
        repository: &str,
        reference: &str,
        token: &str,
        allow_missing: bool,
    ) -> Result<Option<Manifest>, DiscoveryError> {
        let response = self
            .client
            .get(format!(
                "https://ghcr.io/v2/{repository}/manifests/{reference}"
            ))
            .header("Accept", ACCEPT)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| DiscoveryError::Unavailable)?;
        if !status(response.status(), allow_missing)? {
            return Ok(None);
        }
        let media_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .ok_or(DiscoveryError::ResponseInvalid)?
            .to_owned();
        let claimed = claimed_digest(&response)?;
        let bytes = body(response).await?;
        let expected = reference.starts_with("sha256:").then_some(reference);
        parse_manifest(&bytes, &media_type, claimed.as_deref(), expected).map(Some)
    }
}
fn cdn_redirect(location: &str, hops: usize) -> Result<Url, DiscoveryError> {
    if hops >= 2 || location.len() > 16_384 || location.chars().any(char::is_control) {
        return Err(DiscoveryError::ResponseInvalid);
    }
    // Reject syntactic userinfo even when the URL parser normalizes an empty user.
    let authority = location
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default())
        .ok_or(DiscoveryError::ResponseInvalid)?;
    if authority.contains('@') {
        return Err(DiscoveryError::ResponseInvalid);
    }
    let url = Url::parse(location).map_err(|_| DiscoveryError::ResponseInvalid)?;
    if url.scheme() != "https"
        || url.host_str() != Some("pkg-containers.githubusercontent.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(DiscoveryError::ResponseInvalid);
    }
    Ok(url)
}
fn validate_input(
    repository: &str,
    tag: &str,
    credentials: &Credentials,
) -> Result<(), DiscoveryError> {
    ImageRepository::parse(repository).map_err(|_| DiscoveryError::InvalidInput)?;
    ImageTag::parse(tag).map_err(|_| DiscoveryError::InvalidInput)?;
    let path = repository
        .strip_prefix("ghcr.io/")
        .ok_or(DiscoveryError::InvalidInput)?;
    if !path.contains('/')
        || credentials.username.is_empty()
        || credentials.password.is_empty()
        || credentials.username.len() > 4096
        || credentials.password.len() > 32768
    {
        return Err(DiscoveryError::InvalidInput);
    }
    Ok(())
}
const fn status(value: StatusCode, allow_missing: bool) -> Result<bool, DiscoveryError> {
    match value.as_u16() {
        200 => Ok(true),
        404 if allow_missing => Ok(false),
        401 | 403 => Err(DiscoveryError::AccessDenied),
        429 => Err(DiscoveryError::RateLimited),
        500..=599 => Err(DiscoveryError::Unavailable),
        _ => Err(DiscoveryError::ResponseInvalid),
    }
}
async fn body(response: Response) -> Result<Vec<u8>, DiscoveryError> {
    bounded_body(response, LIMIT).await.map_err(|reason| {
        if reason == "registry_response_oversized" {
            DiscoveryError::ResponseOversized
        } else {
            DiscoveryError::Unavailable
        }
    })
}
fn claimed_digest(response: &Response) -> Result<Option<String>, DiscoveryError> {
    response
        .headers()
        .get("docker-content-digest")
        .map(|value| {
            value
                .to_str()
                .map(str::to_owned)
                .map_err(|_| DiscoveryError::ResponseInvalid)
        })
        .transpose()
}
fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn validate_digest(
    bytes: &[u8],
    claimed: Option<&str>,
    expected: Option<&str>,
) -> Result<String, DiscoveryError> {
    let digest = hash(bytes);
    for value in [claimed, expected].into_iter().flatten() {
        Sha256Digest::parse(value).map_err(|_| DiscoveryError::ResponseInvalid)?;
        if value != digest {
            return Err(DiscoveryError::ResponseInvalid);
        }
    }
    Ok(digest)
}
struct Manifest {
    value: Value,
    media_type: String,
    digest: String,
    size: usize,
}
fn is_index(value: &str) -> bool {
    matches!(value, OCI_INDEX | DOCKER_INDEX)
}
fn parse_manifest(
    bytes: &[u8],
    content_type: &str,
    claimed: Option<&str>,
    expected: Option<&str>,
) -> Result<Manifest, DiscoveryError> {
    if bytes.len() > LIMIT {
        return Err(DiscoveryError::ResponseOversized);
    }
    let media_type = content_type.split(';').next().unwrap_or_default().trim();
    if !matches!(
        media_type,
        OCI_MANIFEST | DOCKER_MANIFEST | OCI_INDEX | DOCKER_INDEX
    ) {
        return Err(DiscoveryError::ResponseInvalid);
    }
    let digest = validate_digest(bytes, claimed, expected)?;
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| DiscoveryError::ResponseInvalid)?;
    if value["schemaVersion"] != 2
        || value
            .get("mediaType")
            .is_some_and(|value| value.as_str() != Some(media_type))
    {
        return Err(DiscoveryError::ResponseInvalid);
    }
    if is_index(media_type) {
        let descriptors = value["manifests"]
            .as_array()
            .ok_or(DiscoveryError::ResponseInvalid)?;
        if descriptors.len() > 256 {
            return Err(DiscoveryError::ResponseOversized);
        }
    } else {
        config_descriptor(&value)?;
        validate_layers(&value)?;
    }
    Ok(Manifest {
        value,
        media_type: media_type.into(),
        digest,
        size: bytes.len(),
    })
}
struct Descriptor {
    digest: String,
    size: usize,
    media_type: String,
}
fn descriptor(value: &Value) -> Result<Descriptor, DiscoveryError> {
    let digest = value["digest"]
        .as_str()
        .ok_or(DiscoveryError::ResponseInvalid)?;
    Sha256Digest::parse(digest).map_err(|_| DiscoveryError::ResponseInvalid)?;
    let size = value["size"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n <= LIMIT)
        .ok_or(DiscoveryError::ResponseInvalid)?;
    if value.get("urls").is_some() {
        return Err(DiscoveryError::ResponseInvalid);
    }
    Ok(Descriptor {
        digest: digest.into(),
        size,
        media_type: value["mediaType"]
            .as_str()
            .ok_or(DiscoveryError::ResponseInvalid)?
            .into(),
    })
}
fn config_descriptor(value: &Value) -> Result<Descriptor, DiscoveryError> {
    if !matches!(
        value["config"]["mediaType"].as_str(),
        Some(OCI_CONFIG | DOCKER_CONFIG)
    ) {
        return Err(DiscoveryError::ResponseInvalid);
    }
    descriptor(&value["config"])
}
fn matches_platform(value: &Value, platform: Platform) -> bool {
    if value
        .get("variant")
        .is_some_and(|variant| !variant.is_string())
    {
        return false;
    }
    let variant = value["variant"].as_str().unwrap_or_default();
    value["os"] == "linux"
        && value["architecture"] == platform.architecture()
        && (variant.is_empty() || platform == Platform::LinuxArm64 && variant == "v8")
        && value.get("os.version").is_none()
        && value.get("os.features").is_none()
}
fn select_platform(value: &Value, platform: Platform) -> Result<Descriptor, DiscoveryError> {
    let items = value["manifests"]
        .as_array()
        .ok_or(DiscoveryError::ResponseInvalid)?;
    let selected = items
        .iter()
        .filter(|item| matches_platform(&item["platform"], platform))
        .collect::<Vec<_>>();
    match selected.as_slice() {
        [] => Err(DiscoveryError::PlatformUnavailable),
        [item]
            if matches!(
                item["mediaType"].as_str(),
                Some(OCI_MANIFEST | DOCKER_MANIFEST)
            ) =>
        {
            descriptor(item)
        }
        _ => Err(DiscoveryError::ResponseInvalid),
    }
}
fn validate_layers(value: &Value) -> Result<(), DiscoveryError> {
    let layers = value["layers"]
        .as_array()
        .ok_or(DiscoveryError::ResponseInvalid)?;
    if layers.len() > 256 {
        return Err(DiscoveryError::ResponseOversized);
    }
    for layer in layers {
        let digest = layer["digest"]
            .as_str()
            .ok_or(DiscoveryError::ResponseInvalid)?;
        Sha256Digest::parse(digest).map_err(|_| DiscoveryError::ResponseInvalid)?;
        if layer["size"].as_u64().is_none() || layer.get("urls").is_some() {
            return Err(DiscoveryError::ResponseInvalid);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest() -> Value {
        json!({"schemaVersion":2,"mediaType":OCI_MANIFEST,
            "config":{"mediaType":OCI_CONFIG,"digest":hash(b"config"),"size":6},
            "layers":[{"mediaType":"application/vnd.oci.image.layer.v1.tar+gzip","digest":hash(b"layer"),"size":5}]})
    }
    fn platform_descriptor(platform: Platform, digest: &str) -> Value {
        json!({"mediaType":OCI_MANIFEST,"digest":digest,"size":100,
            "platform":{"os":"linux","architecture":platform.architecture()}})
    }
    #[test]
    fn blob_redirects_only_allow_bounded_https_github_cdn_without_userinfo() {
        let accepted =
            "https://pkg-containers.githubusercontent.com/blobs/sha256:abc?signature=fixture";
        assert!(cdn_redirect(accepted, 0).is_ok());
        assert!(cdn_redirect(accepted, 1).is_ok());
        assert_eq!(
            cdn_redirect(accepted, 2),
            Err(DiscoveryError::ResponseInvalid)
        );
        assert_eq!(
            cdn_redirect(accepted, usize::MAX),
            Err(DiscoveryError::ResponseInvalid)
        );
        for invalid in [
            "http://pkg-containers.githubusercontent.com/blob",
            "https://evil.test/blob",
            "https://pkg-containers.githubusercontent.com.evil.test/blob",
            "https://pkg-containers.githubusercontent.com:444/blob",
            "https://user@pkg-containers.githubusercontent.com/blob",
            "https://user:password@pkg-containers.githubusercontent.com/blob",
            "https://@pkg-containers.githubusercontent.com/blob",
            "https://pkg-containers.githubusercontent.com/blob#fragment",
            "https://ghcr.io/v2/team/runtime/blobs/sha256:abc",
            "/relative/blob",
            "https://pkg-containers.githubusercontent.com/blob\n",
        ] {
            assert_eq!(
                cdn_redirect(invalid, 0),
                Err(DiscoveryError::ResponseInvalid)
            );
        }
        assert!(cdn_redirect("https://pkg-containers.githubusercontent.com:443/blob", 0).is_ok());
    }
    #[test]
    fn cross_origin_blob_requests_never_inherit_registry_authorization()
    -> Result<(), Box<dyn std::error::Error>> {
        let client = GhcrClient::new()?;
        let origin = client
            .origin_blob_request("team/runtime", &hash(b"blob"), "fixture-token")
            .build()?;
        assert_eq!(origin.url().host_str(), Some("ghcr.io"));
        assert!(origin.headers().contains_key("authorization"));
        let cdn = client
            .cdn_blob_request(cdn_redirect(
                "https://pkg-containers.githubusercontent.com/blob?signature=fixture",
                0,
            )?)
            .build()?;
        assert_eq!(
            cdn.url().host_str(),
            Some("pkg-containers.githubusercontent.com")
        );
        assert!(!cdn.headers().contains_key("authorization"));
        assert!(!cdn.headers().contains_key("cookie"));
        assert!(!cdn.headers().contains_key("referer"));
        Ok(())
    }
    #[test]
    fn only_initial_manifest_not_found_can_be_missing() {
        assert_eq!(status(StatusCode::NOT_FOUND, true), Ok(false));
        assert_eq!(
            status(StatusCode::NOT_FOUND, false),
            Err(DiscoveryError::ResponseInvalid)
        );
        for allow_missing in [true, false] {
            for status_code in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
                assert_eq!(
                    status(status_code, allow_missing),
                    Err(DiscoveryError::AccessDenied)
                );
            }
            assert_eq!(
                status(StatusCode::TOO_MANY_REQUESTS, allow_missing),
                Err(DiscoveryError::RateLimited)
            );
            assert_eq!(
                status(StatusCode::SERVICE_UNAVAILABLE, allow_missing),
                Err(DiscoveryError::Unavailable)
            );
            assert_eq!(
                status(StatusCode::TEMPORARY_REDIRECT, allow_missing),
                Err(DiscoveryError::ResponseInvalid)
            );
            assert_eq!(status(StatusCode::OK, allow_missing), Ok(true));
        }
    }
    #[test]
    fn configured_destination_and_tags_cannot_redirect_credentials() {
        let credentials = Credentials {
            username: "pull-user".into(),
            password: "fixture".into(),
        };
        assert!(validate_input("ghcr.io/team/runtime", "ayni-lock-123", &credentials).is_ok());
        for repository in [
            "evil.test/team/runtime",
            "ghcr.io.evil.test/team/runtime",
            "https://ghcr.io/team/runtime",
            "ghcr.io/team/../runtime",
            "ghcr.io/team/runtime?scope=push",
            "ghcr.io/team/runtime@sha256:abc",
            "ghcr.io/runtime",
        ] {
            assert_eq!(
                validate_input(repository, "tag", &credentials),
                Err(DiscoveryError::InvalidInput)
            );
        }
        for tag in [
            "../latest",
            "tag?scope=push",
            "tag#fragment",
            "tag/name",
            "",
        ] {
            assert_eq!(
                validate_input("ghcr.io/team/runtime", tag, &credentials),
                Err(DiscoveryError::InvalidInput)
            );
        }
        let empty = Credentials {
            username: "user".into(),
            password: String::new(),
        };
        assert_eq!(
            validate_input("ghcr.io/team/runtime", "tag", &empty),
            Err(DiscoveryError::InvalidInput)
        );
    }
    #[test]
    fn manifest_identity_binds_exact_bytes_header_and_requested_digest()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = serde_json::to_vec(&manifest())?;
        let digest = hash(&bytes);
        let parsed = parse_manifest(&bytes, OCI_MANIFEST, Some(&digest), Some(&digest))?;
        assert_eq!(parsed.digest, digest);
        assert_eq!(parsed.size, bytes.len());
        let wrong = hash(b"other");
        assert!(parse_manifest(&bytes, OCI_MANIFEST, Some(&wrong), None).is_err());
        assert!(parse_manifest(&bytes, OCI_MANIFEST, None, Some(&wrong)).is_err());
        assert!(parse_manifest(&bytes, OCI_MANIFEST, Some("sha256:abc"), None).is_err());
        assert!(parse_manifest(&bytes, "text/html", None, None).is_err());
        assert!(parse_manifest(&bytes, DOCKER_MANIFEST, None, None).is_err());
        assert!(parse_manifest(b"not json", OCI_MANIFEST, None, None).is_err());
        assert!(
            parse_manifest(
                &bytes,
                "application/vnd.oci.image.manifest.v1+json; charset=utf-8",
                None,
                None
            )
            .is_ok()
        );
        assert!(matches!(
            parse_manifest(&vec![0; LIMIT + 1], OCI_MANIFEST, None, None),
            Err(DiscoveryError::ResponseOversized)
        ));
        Ok(())
    }
    #[test]
    fn index_selection_requires_one_exact_supported_platform()
    -> Result<(), Box<dyn std::error::Error>> {
        let arm = platform_descriptor(Platform::LinuxArm64, &hash(b"arm"));
        let amd = platform_descriptor(Platform::LinuxAmd64, &hash(b"amd"));
        let index = json!({"schemaVersion":2,"mediaType":OCI_INDEX,"manifests":[arm,amd]});
        let bytes = serde_json::to_vec(&index)?;
        assert!(parse_manifest(&bytes, OCI_INDEX, None, None).is_ok());
        assert_eq!(
            select_platform(&index, Platform::LinuxAmd64)?.digest,
            hash(b"amd")
        );
        assert_eq!(
            select_platform(&index, Platform::LinuxArm64)?.digest,
            hash(b"arm")
        );
        assert!(matches!(
            select_platform(&json!({"manifests":[arm,arm]}), Platform::LinuxArm64),
            Err(DiscoveryError::ResponseInvalid)
        ));
        assert!(matches!(
            select_platform(&json!({"manifests":[amd]}), Platform::LinuxArm64),
            Err(DiscoveryError::PlatformUnavailable)
        ));
        let mut nested = arm;
        nested["mediaType"] = OCI_INDEX.into();
        assert!(matches!(
            select_platform(&json!({"manifests":[nested]}), Platform::LinuxArm64),
            Err(DiscoveryError::ResponseInvalid)
        ));
        Ok(())
    }
    #[test]
    fn config_platform_and_descriptors_reject_unsafe_or_unbounded_shapes()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(matches_platform(
            &json!({"os":"linux","architecture":"arm64","variant":"v8"}),
            Platform::LinuxArm64
        ));
        for config in [
            json!({"os":"windows","architecture":"amd64"}),
            json!({"os":"linux","architecture":"arm64"}),
            json!({"os":"linux","architecture":"amd64","os.features":["special"]}),
        ] {
            assert!(!matches_platform(&config, Platform::LinuxAmd64));
        }
        let original = manifest();
        for (field, value) in [
            ("digest", json!("sha256:BAD")),
            ("size", json!(LIMIT + 1)),
            ("urls", json!(["https://evil.test/blob"])),
        ] {
            let mut changed = original.clone();
            changed["config"][field] = value;
            let bytes = serde_json::to_vec(&changed)?;
            assert!(parse_manifest(&bytes, OCI_MANIFEST, None, None).is_err());
        }
        let mut changed = original;
        changed["schemaVersion"] = 1.into();
        assert!(parse_manifest(&serde_json::to_vec(&changed)?, OCI_MANIFEST, None, None).is_err());
        Ok(())
    }
}
