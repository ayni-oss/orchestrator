//! Fixed-origin Kubernetes provisioning. Only metadata GET responses are decoded.
use super::material::Material;
use ayni_orchestrator_workflow_postgres::store::runtime_signer_provisioning::SignerProvisioning;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};
use zeroize::Zeroizing;

const METADATA_ACCEPT: &str = "application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1";
const ORIGIN: &str = "https://kubernetes.default.svc";
const CONFIG_ROOT: &str = "/var/run/secrets/ayni/signer-provisioning";
const UNAVAILABLE: &str = "signer_provisioning_unavailable";
const CONFLICT: &str = "signer_provisioning_conflict";

pub(super) struct Kubernetes {
    client: Client,
    origin: String,
    namespace: String,
    token: Zeroizing<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MetadataResponse {
    api_version: String,
    kind: String,
    metadata: Metadata,
}
#[derive(Serialize, Deserialize)]
struct Metadata {
    name: String,
    namespace: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    uid: String,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Secret<'a> {
    api_version: &'static str,
    kind: &'static str,
    metadata: Metadata,
    immutable: bool,
    #[serde(rename = "type")]
    kind_type: &'static str,
    data: BTreeMap<&'static str, &'a str>,
}
const fn reason(status: StatusCode) -> &'static str {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        "signer_provisioning_access_denied"
    } else {
        UNAVAILABLE
    }
}
fn valid_namespace(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}
fn annotations(op: &SignerProvisioning, public_key: &str) -> BTreeMap<String, String> {
    [
        ("ayni.build/signer-provisioning", op.id.as_str()),
        ("ayni.build/signer-schema", "1"),
        ("ayni.build/signer-mode", op.mode.as_str()),
        ("ayni.build/signer-key-id", op.key_id.as_str()),
        ("ayni.build/signer-public-key", public_key),
        ("ayni.build/signer-immutable", "true"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}
impl Kubernetes {
    pub(super) fn configured(expected_namespace: &str) -> Result<Self, &'static str> {
        if std::env::var("ORCHESTRATOR_SIGNER_PROVISIONING_ENABLED").as_deref() != Ok("true") {
            return Err("signer_provisioning_disabled");
        }
        let namespace = target_namespace()?;
        if namespace != expected_namespace {
            return Err(CONFLICT);
        }
        let ca = std::fs::read(format!("{CONFIG_ROOT}/ca.crt")).map_err(|_| UNAVAILABLE)?;
        let ca = reqwest::Certificate::from_pem(&ca).map_err(|_| UNAVAILABLE)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .tls_certs_only([ca])
            .build()
            .map_err(|_| UNAVAILABLE)?;
        let token = Zeroizing::new(
            std::fs::read_to_string(format!("{CONFIG_ROOT}/token")).map_err(|_| UNAVAILABLE)?,
        );
        if token.trim().is_empty() || token.len() > 16_384 {
            return Err(UNAVAILABLE);
        }
        Ok(Self {
            client,
            origin: ORIGIN.to_owned(),
            namespace,
            token,
        })
    }
    fn collection(&self) -> String {
        format!(
            "{}/api/v1/namespaces/{}/secrets",
            self.origin, self.namespace
        )
    }
    pub(super) async fn observed(
        &self,
        op: &SignerProvisioning,
    ) -> Result<Option<(String, String)>, &'static str> {
        validate_intent(op)?;
        let response = self
            .client
            .get(format!("{}/{}", self.collection(), op.secret_name))
            .bearer_auth(self.token.trim())
            .header("Accept", METADATA_ACCEPT)
            .send()
            .await
            .map_err(|_| UNAVAILABLE)?;
        match response.status() {
            StatusCode::NOT_FOUND => return Ok(None),
            StatusCode::OK => {}
            status => return Err(reason(status)),
        }
        if response.content_length().is_some_and(|size| size > 16_384) {
            return Err(CONFLICT);
        }
        let mut response = response;
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| UNAVAILABLE)? {
            if chunk.len() > 16_384usize.saturating_sub(bytes.len()) {
                return Err(CONFLICT);
            }
            bytes.extend_from_slice(&chunk);
        }
        validate_observed(&bytes, &self.namespace, op).map(Some)
    }
    pub(super) async fn create(
        &self,
        op: &SignerProvisioning,
        material: &Material,
    ) -> Result<(), &'static str> {
        let encoded = Zeroizing::new(STANDARD.encode(material.seed.as_bytes()));
        let secret = Secret {
            api_version: "v1",
            kind: "Secret",
            immutable: true,
            kind_type: "Opaque",
            metadata: Metadata {
                name: op.secret_name.clone(),
                namespace: self.namespace.clone(),
                uid: String::new(),
                annotations: annotations(op, &material.public_key),
            },
            data: BTreeMap::from([("seed", encoded.as_str())]),
        };
        let payload = serde_json::to_vec(&secret).map_err(|_| UNAVAILABLE)?;
        let response = self
            .client
            .post(self.collection())
            .bearer_auth(self.token.trim())
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .await
            .map_err(|_| UNAVAILABLE)?;
        // Kubernetes POST echoes Secret.data. Never buffer, deserialize, log or
        // attach that response body to an error; recovery reads public metadata.
        match response.status() {
            StatusCode::CREATED | StatusCode::CONFLICT => Ok(()),
            status => Err(reason(status)),
        }
    }
}
fn validate_observed(
    bytes: &[u8],
    namespace: &str,
    op: &SignerProvisioning,
) -> Result<(String, String), &'static str> {
    let observed: MetadataResponse = serde_json::from_slice(bytes).map_err(|_| CONFLICT)?;
    if observed.kind != "PartialObjectMetadata"
        || observed.api_version != "meta.k8s.io/v1"
        || observed.metadata.name != op.secret_name
        || observed.metadata.namespace != namespace
    {
        return Err(CONFLICT);
    }
    let uid = uuid::Uuid::parse_str(&observed.metadata.uid).map_err(|_| CONFLICT)?;
    if uid.to_string() != observed.metadata.uid {
        return Err(CONFLICT);
    }
    let public_key = observed
        .metadata
        .annotations
        .get("ayni.build/signer-public-key")
        .ok_or(CONFLICT)?;
    if public_key.len() != 64
        || !public_key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || op.mode == "import" && *public_key != op.import_public_key
        || annotations(op, public_key)
            .iter()
            .any(|(key, value)| observed.metadata.annotations.get(key) != Some(value))
    {
        return Err(CONFLICT);
    }
    Ok((observed.metadata.uid, public_key.clone()))
}

pub(super) fn target_namespace() -> Result<String, &'static str> {
    let namespace = std::env::var("ORCHESTRATOR_SIGNER_PROVISIONING_NAMESPACE")
        .map_err(|_| "signer_provisioning_disabled")?;
    if !valid_namespace(&namespace) {
        return Err(UNAVAILABLE);
    }
    Ok(namespace)
}

fn validate_intent(op: &SignerProvisioning) -> Result<(), &'static str> {
    let id = uuid::Uuid::parse_str(&op.id).map_err(|_| CONFLICT)?;
    if id.to_string() != op.id || op.secret_name != format!("ayni-runtime-signer-{}", id.simple()) {
        return Err(CONFLICT);
    }
    Ok(())
}
#[cfg(test)]
mod tests;
