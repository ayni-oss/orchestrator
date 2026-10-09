//! GHCR lookup with pull-only authority, outside the control-plane process.
use ayni_orchestrator_runtime::{image_discovery::ImageDiscovery, runtime_contract::RuntimeImage};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, StatusCode};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::Path,
    time::Duration,
};

const ACCEPT: &str = "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json";

/// Inspect the discovery tag and persist a bounded registry outcome.
/// # Errors
/// Returns an error if completion metadata cannot be written.
pub async fn run() -> io::Result<()> {
    let result = match resolve().await {
        Ok(value) => value,
        Err(reason) => ImageDiscovery::Failed {
            reason: reason.into(),
        },
    };
    fs::write(
        "/dev/termination-log",
        serde_json::to_vec(&result).map_err(io::Error::other)?,
    )
}

async fn body(mut response: reqwest::Response) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "registry_unavailable")? {
        if bytes.len() + chunk.len() > 4 * 1_048_576 {
            return Err("registry_response_invalid");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

// This fixed, read-only Kubernetes Secret mount is trusted orchestration input.
// Projected Secret keys are symlinks; repository inputs still use read_bounded.
fn read_registry_credentials(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 65_536 {
        return Err(io::Error::other(
            "invalid or oversized registry credentials",
        ));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(65_537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65_536 {
        return Err(io::Error::other("oversized registry credentials"));
    }
    Ok(bytes)
}

fn credentials() -> Result<Option<(String, String)>, &'static str> {
    let path = std::path::Path::new("/registry/config.json");
    if !path.exists() {
        return Ok(None);
    }
    let bytes = read_registry_credentials(path).map_err(|_| "registry_credentials_unavailable")?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "registry_credentials_invalid")?;
    let entry = &value["auths"]["ghcr.io"];
    if let Some(auth) = entry["auth"].as_str() {
        let bytes = STANDARD
            .decode(auth)
            .map_err(|_| "registry_credentials_invalid")?;
        let decoded = String::from_utf8(bytes).map_err(|_| "registry_credentials_invalid")?;
        let (user, password) = decoded
            .split_once(':')
            .ok_or("registry_credentials_invalid")?;
        return Ok(Some((user.into(), password.into())));
    }
    Ok(Some((
        entry["username"]
            .as_str()
            .ok_or("registry_credentials_invalid")?
            .into(),
        entry["password"]
            .as_str()
            .ok_or("registry_credentials_invalid")?
            .into(),
    )))
}

const fn response_status(status: StatusCode) -> Result<bool, &'static str> {
    match status {
        StatusCode::NOT_FOUND => Ok(false),
        StatusCode::OK => Ok(true),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err("registry_access_denied"),
        StatusCode::TOO_MANY_REQUESTS => Err("registry_rate_limited"),
        _ => Err("registry_unavailable"),
    }
}

async fn resolve() -> Result<ImageDiscovery, &'static str> {
    let image = std::env::var("DISCOVERY_IMAGE").map_err(|_| "runtime_image_invalid")?;
    let platform = std::env::var("DISCOVERY_PLATFORM").map_err(|_| "runtime_platform_invalid")?;
    inspect(&image, &platform).await
}

fn discovery_reference(image: &str) -> Result<(&str, &str, &str), &'static str> {
    let (repository, tag) = image.rsplit_once(':').ok_or("runtime_image_invalid")?;
    RuntimeImage::parse(&format!("{repository}@sha256:{}", "a".repeat(64)))
        .map_err(|_| "runtime_image_invalid")?;
    if tag.is_empty()
        || tag.len() > 128
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    {
        return Err("runtime_image_invalid");
    }
    let path = repository
        .strip_prefix("ghcr.io/")
        .ok_or("registry_provider_unsupported")?;
    Ok((repository, tag, path))
}

pub(crate) async fn inspect(image: &str, platform: &str) -> Result<ImageDiscovery, &'static str> {
    let (repository, tag, path) = discovery_reference(image)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "registry_unavailable")?;
    let mut auth = client.get("https://ghcr.io/token").query(&[
        ("service", "ghcr.io"),
        ("scope", &format!("repository:{path}:pull")),
    ]);
    if let Some((user, password)) = credentials()? {
        auth = auth.basic_auth(user, Some(password));
    }
    let response = auth.send().await.map_err(|_| "registry_unavailable")?;
    if !response_status(response.status())? {
        return Err("registry_access_denied");
    }
    let auth: Value =
        serde_json::from_slice(&body(response).await?).map_err(|_| "registry_response_invalid")?;
    let token = auth["token"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 65_536)
        .ok_or("registry_access_denied")?;
    let response = client
        .get(format!("https://ghcr.io/v2/{path}/manifests/{tag}"))
        .bearer_auth(token)
        .header("accept", ACCEPT)
        .send()
        .await
        .map_err(|_| "registry_unavailable")?;
    if !response_status(response.status())? {
        return Ok(ImageDiscovery::Missing);
    }
    let mut bytes = body(response).await?;
    let manifest: Value =
        serde_json::from_slice(&bytes).map_err(|_| "registry_response_invalid")?;
    if let Some(manifests) = manifest["manifests"].as_array() {
        let (os, arch) = platform.split_once('/').ok_or("runtime_platform_invalid")?;
        let digest = manifests
            .iter()
            .find(|m| m["platform"]["os"] == os && m["platform"]["architecture"] == arch)
            .and_then(|m| m["digest"].as_str())
            .ok_or("runtime_platform_mismatch")?;
        RuntimeImage::parse(&format!("{repository}@{digest}"))
            .map_err(|_| "registry_response_invalid")?;
        let response = client
            .get(format!("https://ghcr.io/v2/{path}/manifests/{digest}"))
            .bearer_auth(token)
            .header("accept", ACCEPT)
            .send()
            .await
            .map_err(|_| "registry_unavailable")?;
        if !response_status(response.status())? {
            return Err("registry_response_invalid");
        }
        bytes = body(response).await?;
        if format!("sha256:{:x}", Sha256::digest(&bytes)) != digest {
            return Err("runtime_digest_mismatch");
        }
    }
    let manifest: Value =
        serde_json::from_slice(&bytes).map_err(|_| "registry_response_invalid")?;
    let image_id = manifest["config"]["digest"]
        .as_str()
        .ok_or("registry_response_invalid")?;
    RuntimeImage::parse(&format!("{repository}@{image_id}"))
        .map_err(|_| "registry_response_invalid")?;
    verify_config(&client, path, token, image_id, platform).await?;
    Ok(ImageDiscovery::Resolved {
        image_id: image_id.into(),
        digest: format!("sha256:{:x}", Sha256::digest(bytes)),
    })
}

async fn verify_config(
    client: &Client,
    path: &str,
    token: &str,
    image_id: &str,
    platform: &str,
) -> Result<(), &'static str> {
    // A single-platform tag has no index platform descriptor. The config is
    // authoritative in both cases, and is independently bound to its digest.
    let response = client
        .get(format!("https://ghcr.io/v2/{path}/blobs/{image_id}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "registry_unavailable")?;
    let config = body(config_response(client, response).await?).await?;
    if format!("sha256:{:x}", Sha256::digest(&config)) != image_id {
        return Err("runtime_digest_mismatch");
    }
    let config: Value = serde_json::from_slice(&config).map_err(|_| "registry_response_invalid")?;
    validate_platform(&config, platform)?;
    Ok(())
}

// GHCR delegates blob delivery to its object store. Never forward registry
// authorization to that host or follow arbitrary registry-provided redirects.
async fn config_response(
    client: &Client,
    response: reqwest::Response,
) -> Result<reqwest::Response, &'static str> {
    if !response.status().is_redirection() {
        if !response_status(response.status())? {
            return Err("registry_response_invalid");
        }
        return Ok(response);
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or("registry_response_invalid")?;
    let url = reqwest::Url::parse(location).map_err(|_| "registry_response_invalid")?;
    if url.scheme() != "https"
        || url.host_str() != Some("pkg-containers.githubusercontent.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("registry_response_invalid");
    }
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| "registry_unavailable")?;
    if !response_status(response.status())? {
        return Err("registry_response_invalid");
    }
    Ok(response)
}

fn validate_platform(config: &Value, platform: &str) -> Result<(), &'static str> {
    let (os, architecture) = platform.split_once('/').ok_or("runtime_platform_invalid")?;
    if config["os"] != os || config["architecture"] != architecture {
        return Err("runtime_platform_mismatch");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn projected_secret_credentials_are_read_with_a_size_bound() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let revision = directory.path().join("revision");
        fs::create_dir(&revision)?;
        let content = b"{\"auths\":{}}";
        fs::write(revision.join("config.json"), content)?;
        std::os::unix::fs::symlink("revision", directory.path().join("..data"))?;
        let key = directory.path().join("config.json");
        std::os::unix::fs::symlink("..data/config.json", &key)?;
        assert_eq!(read_registry_credentials(&key)?, content);
        assert!(ayni_orchestrator_runtime::runtime_execution::read_bounded(&key, 65_536).is_err());
        fs::write(revision.join("config.json"), vec![0; 65_537])?;
        assert!(read_registry_credentials(&key).is_err());
        assert!(read_registry_credentials(directory.path()).is_err());
        Ok(())
    }
    #[test]
    fn only_manifest_not_found_authorizes_a_build() {
        assert_eq!(response_status(StatusCode::NOT_FOUND), Ok(false));
        for status in [
            StatusCode::FORBIDDEN,
            StatusCode::UNAUTHORIZED,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert!(response_status(status).is_err());
        }
    }

    #[test]
    fn manifest_config_must_match_requested_platform() {
        let config = serde_json::json!({"os":"linux","architecture":"arm64"});
        assert!(validate_platform(&config, "linux/arm64").is_ok());
        assert!(validate_platform(&config, "linux/amd64").is_err());
        assert!(validate_platform(&serde_json::json!({}), "linux/arm64").is_err());
    }
}
