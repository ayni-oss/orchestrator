//! Deterministic Kubernetes Job rendering for one isolated coding attempt.

use crate::runtime_contract::{RuntimeContract, RuntimeContractError};
use serde::Serialize;
use std::collections::BTreeMap;
use thiserror::Error;

const WORKSPACE: &str = "/workspace";
const SCRATCH: &str = "/scratch";
const OUTPUT: &str = "/output";
const AGENT_UID: i64 = 10_002;
const AGENT_GID: i64 = 10_001;

/// Trusted controller inputs used to render one attempt-owned Job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeJobRequest {
    /// Stable attempt UUID used in labels, not in image identity.
    pub attempt_id: String,
    /// Namespace selected by the controller configuration.
    pub namespace: String,
    /// Immutable runtime admission.
    pub runtime: RuntimeContract,
    /// Reference to a short-lived, attempt-owned registry credential secret.
    pub pull_secret: Option<String>,
    /// Bounded execution deadline in seconds.
    pub deadline_seconds: u32,
}

impl RuntimeJobRequest {
    /// Renders a Kubernetes Job manifest with no service account or host mounts.
    pub fn render(&self) -> Result<RenderedRuntimeJob, RuntimeJobError> {
        if !dns_label(&self.attempt_id) || !dns_label(&self.namespace) {
            return Err(RuntimeJobError::InvalidIdentity);
        }
        if self.deadline_seconds == 0 || self.deadline_seconds > 86_400 {
            return Err(RuntimeJobError::InvalidDeadline(self.deadline_seconds));
        }
        if self
            .pull_secret
            .as_deref()
            .is_some_and(|value| !dns_label(value))
        {
            return Err(RuntimeJobError::InvalidPullSecret);
        }
        let name = format!("runtime-{}", self.attempt_id);
        if name.len() > 63 {
            return Err(RuntimeJobError::InvalidIdentity);
        }
        let labels = BTreeMap::from([
            ("app.kubernetes.io/name", "ayni-orchestrator-runtime"),
            ("app.kubernetes.io/component", "coding-attempt"),
            ("ayni.io/attempt-id", self.attempt_id.as_str()),
            ("ayni.io/runtime-platform", self.runtime.platform().as_str()),
        ]);
        let manifest = Job {
            api_version: "batch/v1",
            kind: "Job",
            metadata: Metadata {
                name: &name,
                namespace: &self.namespace,
                labels,
            },
            spec: JobSpec {
                backoff_limit: 0,
                active_deadline_seconds: self.deadline_seconds,
                ttl_seconds_after_finished: 3_600,
                template: PodTemplate {
                    metadata: PodMetadata {
                        labels: BTreeMap::from([("ayni.io/attempt-id", self.attempt_id.as_str())]),
                    },
                    spec: PodSpec {
                        restart_policy: "Never",
                        automount_service_account_token: false,
                        image_pull_secrets: self
                            .pull_secret
                            .as_deref()
                            .map(|name| vec![SecretRef { name }])
                            .unwrap_or_default(),
                        security_context: PodSecurity {
                            run_as_non_root: true,
                            seccomp_profile: Seccomp {
                                kind: "RuntimeDefault",
                            },
                        },
                        containers: vec![Container {
                            name: "agent",
                            image: self.runtime.image().as_str(),
                            image_pull_policy: "IfNotPresent",
                            command: self.runtime.command().arguments(),
                            env: vec![
                                Env {
                                    name: "ORCHESTRATOR_WORKSPACE",
                                    value: WORKSPACE,
                                },
                                Env {
                                    name: "ORCHESTRATOR_OUTPUT",
                                    value: OUTPUT,
                                },
                            ],
                            security_context: ContainerSecurity {
                                allow_privilege_escalation: false,
                                read_only_root_filesystem: true,
                                run_as_user: AGENT_UID,
                                run_as_group: AGENT_GID,
                                capabilities: Capabilities { drop: vec!["ALL"] },
                            },
                            resources: Resources {
                                requests: BTreeMap::from([("cpu", "500m"), ("memory", "1Gi")]),
                                limits: BTreeMap::from([("cpu", "2"), ("memory", "4Gi")]),
                            },
                            volume_mounts: vec![
                                Mount {
                                    name: "workspace",
                                    mount_path: WORKSPACE,
                                },
                                Mount {
                                    name: "scratch",
                                    mount_path: SCRATCH,
                                },
                                Mount {
                                    name: "output",
                                    mount_path: OUTPUT,
                                },
                            ],
                        }],
                        volumes: vec![
                            Volume { name: "workspace" },
                            Volume { name: "scratch" },
                            Volume { name: "output" },
                        ],
                    },
                },
            },
        };
        let manifest = serde_json::to_value(manifest).expect("job manifest is serializable");
        Ok(RenderedRuntimeJob { name, manifest })
    }
}

/// Rendered Job identity and provider-neutral JSON manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedRuntimeJob {
    pub name: String,
    pub manifest: serde_json::Value,
}

fn dns_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Job admission failed before Kubernetes mutation.
#[derive(Debug, Error)]
pub enum RuntimeJobError {
    /// Runtime admission was invalid.
    #[error(transparent)]
    Contract(#[from] RuntimeContractError),
    /// Namespace or attempt identity is unsafe for a Job name.
    #[error("runtime Job namespace or attempt identity is invalid")]
    InvalidIdentity,
    /// Pull secret reference is unsafe.
    #[error("runtime pull secret name is invalid")]
    InvalidPullSecret,
    /// Deadline is outside the bounded execution window.
    #[error("runtime deadline must be between 1 and 86400 seconds, got {0}")]
    InvalidDeadline(u32),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Job<'a> {
    api_version: &'a str,
    kind: &'a str,
    metadata: Metadata<'a>,
    spec: JobSpec<'a>,
}
#[derive(Serialize)]
struct Metadata<'a> {
    name: &'a str,
    namespace: &'a str,
    labels: BTreeMap<&'a str, &'a str>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JobSpec<'a> {
    backoff_limit: u8,
    active_deadline_seconds: u32,
    ttl_seconds_after_finished: u32,
    template: PodTemplate<'a>,
}
#[derive(Serialize)]
struct PodTemplate<'a> {
    metadata: PodMetadata<'a>,
    spec: PodSpec<'a>,
}
#[derive(Serialize)]
struct PodMetadata<'a> {
    labels: BTreeMap<&'a str, &'a str>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PodSpec<'a> {
    restart_policy: &'a str,
    automount_service_account_token: bool,
    image_pull_secrets: Vec<SecretRef<'a>>,
    security_context: PodSecurity<'a>,
    containers: Vec<Container<'a>>,
    volumes: Vec<Volume<'a>>,
}
#[derive(Serialize)]
struct SecretRef<'a> {
    name: &'a str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PodSecurity<'a> {
    run_as_non_root: bool,
    seccomp_profile: Seccomp<'a>,
}
#[derive(Serialize)]
struct Seccomp<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Container<'a> {
    name: &'a str,
    image: &'a str,
    image_pull_policy: &'a str,
    command: &'a [String],
    env: Vec<Env<'a>>,
    security_context: ContainerSecurity<'a>,
    resources: Resources<'a>,
    volume_mounts: Vec<Mount<'a>>,
}
#[derive(Serialize)]
struct Env<'a> {
    name: &'a str,
    value: &'a str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContainerSecurity<'a> {
    allow_privilege_escalation: bool,
    read_only_root_filesystem: bool,
    run_as_user: i64,
    run_as_group: i64,
    capabilities: Capabilities<'a>,
}
#[derive(Serialize)]
struct Capabilities<'a> {
    drop: Vec<&'a str>,
}
#[derive(Serialize)]
struct Resources<'a> {
    requests: BTreeMap<&'a str, &'a str>,
    limits: BTreeMap<&'a str, &'a str>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Mount<'a> {
    name: &'a str,
    mount_path: &'a str,
}
#[derive(Serialize)]
struct Volume<'a> {
    name: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_contract::{CommandOverride, RuntimeImage, RuntimePlatform};
    fn request() -> RuntimeJobRequest {
        RuntimeJobRequest { attempt_id: "a5f31d70-13b0-4d64-b7a4-cb9823a2a61d".into(), namespace: "ayni-orchestrator".into(), runtime: RuntimeContract::new(RuntimeImage::parse("ghcr.io/acme/runtime@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(), RuntimePlatform::LinuxAmd64, CommandOverride::new(vec!["/opt/ayni/agent".into()]).unwrap()), pull_secret: Some("attempt-pull".into()), deadline_seconds: 3600 }
    }
    #[test]
    fn renders_isolated_digest_pinned_job() {
        let job = request().render().unwrap();
        let pod = &job.manifest["spec"]["template"]["spec"];
        assert_eq!(
            job.manifest["metadata"]["name"],
            "runtime-a5f31d70-13b0-4d64-b7a4-cb9823a2a61d"
        );
        assert_eq!(pod["automountServiceAccountToken"], false);
        assert_eq!(
            pod["containers"][0]["image"],
            "ghcr.io/acme/runtime@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            pod["containers"][0]["securityContext"]["readOnlyRootFilesystem"],
            true
        );
        assert_eq!(pod["imagePullSecrets"][0]["name"], "attempt-pull");
    }
    #[test]
    fn rejects_unsafe_job_inputs() {
        let mut value = request();
        value.attempt_id = "BAD".into();
        assert!(value.render().is_err());
        let mut value = request();
        value.deadline_seconds = 0;
        assert!(value.render().is_err());
    }
}
