//! Attempt-local Docker engines. TLS material is generated once and never read back.
use crate::{configuration::WorkloadRuntime, execution::RuntimeKubernetes, workload};
use k8s_openapi::api::{
    batch::v1::Job,
    core::v1::{Pod, Secret, Service},
    networking::v1::NetworkPolicy,
};
use kube::{
    Api,
    api::{DeleteParams, PostParams},
};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[must_use]
/// Bind a frozen runtime to its attempt-specific engine endpoint and credentials.
pub fn bind(runtime: &WorkloadRuntime, session: &str) -> WorkloadRuntime {
    let mut runtime = runtime.clone();
    if let Some(build) = &mut runtime.build
        && build.engine.is_some()
    {
        build.engine_host = format!("tcp://{session}-engine:2376");
        build.engine_secret = format!("{session}-engine");
    }
    runtime
}

fn tls(name: &str) -> Result<Value, &'static str> {
    let key = KeyPair::generate().map_err(|_| "engine_tls_generation_failed")?;
    let mut ca =
        CertificateParams::new(Vec::<String>::new()).map_err(|_| "engine_tls_generation_failed")?;
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    ca.distinguished_name = DistinguishedName::new();
    let issuer = rcgen::Issuer::from_params(&ca, &key);
    let certificate = ca
        .self_signed(&key)
        .map_err(|_| "engine_tls_generation_failed")?;
    let server_key = KeyPair::generate().map_err(|_| "engine_tls_generation_failed")?;
    let mut server =
        CertificateParams::new(vec![name.into()]).map_err(|_| "engine_tls_generation_failed")?;
    server.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server = server
        .signed_by(&server_key, &issuer)
        .map_err(|_| "engine_tls_generation_failed")?;
    let client_key = KeyPair::generate().map_err(|_| "engine_tls_generation_failed")?;
    let mut client = CertificateParams::default();
    client.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let client = client
        .signed_by(&client_key, &issuer)
        .map_err(|_| "engine_tls_generation_failed")?;
    Ok(
        json!({"ca.pem":certificate.pem(), "server-cert.pem":server.pem(),"server-key.pem":server_key.serialize_pem(),"cert.pem":client.pem(),"key.pem":client_key.serialize_pem()}),
    )
}

fn manifest(name: &str, runtime: &WorkloadRuntime) -> Result<Pod, &'static str> {
    let build = runtime.build.as_ref().ok_or("runtime_builder_missing")?;
    let engine = build
        .engine
        .as_ref()
        .ok_or("runtime_engine_configuration_invalid")?;
    engine.validate()?;
    let dedicated = engine.security_mode == "dedicated-builder";
    let mut selector = workload::node_selector(&runtime.platform, false);
    if dedicated {
        selector[workload::BUILDER_NODE_LABEL] = json!("true");
    }
    serde_json::from_value(json!({
      "apiVersion":"v1","kind":"Pod","metadata":{"name":name,"labels":{"ayni.io/build-engine":name}},
      "spec":{"restartPolicy":"Never","activeDeadlineSeconds":7200,"automountServiceAccountToken":false,
        "nodeSelector":selector,
        "tolerations":if dedicated { json!([{"key":"ayni.io/builder","operator":"Equal","value":"true","effect":"NoSchedule"}]) } else { json!([]) },
        "imagePullSecrets":runtime.pull_secret.iter().map(|s|json!({"name":s})).collect::<Vec<_>>(),
        "containers":[{"name":"engine","image":engine.image,"imagePullPolicy":"Always",
          "command":["dockerd"],"args":["--host=tcp://0.0.0.0:2376","--tlsverify","--tlscacert=/tls/ca.pem","--tlscert=/tls/server-cert.pem","--tlskey=/tls/server-key.pem","--storage-driver=overlay2"],
          "securityContext":{"runAsUser":0,"privileged":true,"allowPrivilegeEscalation":true},
          "resources":{"requests":{"cpu":"500m","memory":"1Gi"},"limits":{"cpu":"2","memory":"4Gi","ephemeral-storage":"20Gi"}},
          "ports":[{"containerPort":2376}],"readinessProbe":{"tcpSocket":{"port":2376},"periodSeconds":2},
          "volumeMounts":[{"name":"data","mountPath":"/var/lib/docker"},{"name":"run","mountPath":"/var/run"},{"name":"tls","mountPath":"/tls","readOnly":true}]}],
        "volumes":[{"name":"data","emptyDir":{"sizeLimit":"16Gi"}},{"name":"run","emptyDir":{}},{"name":"tls","secret":{"secretName":name,"items":[{"key":"ca.pem","path":"ca.pem"},{"key":"server-cert.pem","path":"server-cert.pem"},{"key":"server-key.pem","path":"server-key.pem"}]}}]
      }})).map_err(|_| "engine_manifest_invalid")
}

impl RuntimeKubernetes {
    pub(crate) async fn ensure_engine(
        &self,
        session: &str,
        runtime: &WorkloadRuntime,
    ) -> Result<(), &'static str> {
        let Some(engine) = runtime.build.as_ref().and_then(|b| b.engine.as_ref()) else {
            return Ok(());
        };
        if engine.security_mode != self.configuration.engine_mode {
            return Err("engine_deployment_mode_mismatch");
        }
        engine.validate()?;
        let name = format!("{session}-engine");
        let namespace = &self.configuration.namespace;
        // A missing engine after a Job started invalidates the attempt's Docker state.
        // Never recreate it underneath a retained build or publication candidate.
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        if pods
            .get_opt(&name)
            .await
            .map_err(|_| "engine_startup_failed")?
            .is_none()
            && Api::<Job>::namespaced(self.client.clone(), namespace)
                .get_opt(session)
                .await
                .map_err(|_| "engine_startup_failed")?
                .is_some()
        {
            return Err("engine_lost");
        }
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let secret: Secret = serde_json::from_value(json!({"apiVersion":"v1","kind":"Secret","metadata":{"name":name,"labels":{"ayni.io/build-engine":name}},"immutable":true,"stringData":tls(&name)?})).map_err(|_|"engine_manifest_invalid")?;
        tolerate_exists(secrets.create(&PostParams::default(), &secret).await)?;
        let services: Api<Service> = Api::namespaced(self.client.clone(), namespace);
        let service = serde_json::from_value(json!({"apiVersion":"v1","kind":"Service","metadata":{"name":name},"spec":{"selector":{"ayni.io/build-engine":name},"ports":[{"port":2376,"targetPort":2376}]}})).map_err(|_|"engine_manifest_invalid")?;
        tolerate_exists(services.create(&PostParams::default(), &service).await)?;
        let policies: Api<NetworkPolicy> = Api::namespaced(self.client.clone(), namespace);
        let policy: NetworkPolicy = serde_json::from_value(json!({"apiVersion":"networking.k8s.io/v1","kind":"NetworkPolicy","metadata":{"name":name},"spec":{"podSelector":{"matchLabels":{"ayni.io/build-engine":name}},"policyTypes":["Ingress","Egress"],"ingress":[{"from":[{"podSelector":{"matchLabels":{"ayni.io/engine-session":session}}}],"ports":[{"protocol":"TCP","port":2376}]}],"egress":[{"to":[{"namespaceSelector":{"matchLabels":{"kubernetes.io/metadata.name":"kube-system"}}}],"ports":[{"protocol":"UDP","port":53},{"protocol":"TCP","port":53}]},{"to":[{"ipBlock":{"cidr":"0.0.0.0/0"}},{"ipBlock":{"cidr":"::/0"}}],"ports":[{"protocol":"TCP","port":443},{"protocol":"TCP","port":80}]}]}})).map_err(|_|"engine_manifest_invalid")?;
        tolerate_exists(policies.create(&PostParams::default(), &policy).await)?;
        let client_policy:NetworkPolicy=serde_json::from_value(json!({"apiVersion":"networking.k8s.io/v1","kind":"NetworkPolicy","metadata":{"name":format!("{name}-clients")},"spec":{"podSelector":{"matchLabels":{"ayni.io/engine-session":session}},"policyTypes":["Ingress","Egress"],"ingress":[],"egress":[{"to":[{"podSelector":{"matchLabels":{"ayni.io/build-engine":name}}}],"ports":[{"protocol":"TCP","port":2376}]},{"to":[{"namespaceSelector":{"matchLabels":{"kubernetes.io/metadata.name":"kube-system"}},"podSelector":{"matchLabels":{"k8s-app":"kube-dns"}}}],"ports":[{"protocol":"UDP","port":53},{"protocol":"TCP","port":53}]},{"to":[{"ipBlock":{"cidr":"0.0.0.0/0"}},{"ipBlock":{"cidr":"::/0"}}],"ports":[{"protocol":"TCP","port":443},{"protocol":"TCP","port":80}]}]}})).map_err(|_|"engine_manifest_invalid")?;
        tolerate_exists(
            policies
                .create(&PostParams::default(), &client_policy)
                .await,
        )?;

        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let mut pod = manifest(&name, runtime)?;
        let identity = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&pod.spec).map_err(|_| "engine_manifest_invalid")?)
        );
        pod.metadata
            .annotations
            .get_or_insert_default()
            .insert("ayni.io/spec-digest".into(), identity.clone());
        match pods.create(&PostParams::default(), &pod).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(e)) if e.code == 409 => {
                let existing = pods.get(&name).await.map_err(|_| "engine_startup_failed")?;
                if existing
                    .metadata
                    .annotations
                    .as_ref()
                    .and_then(|a| a.get("ayni.io/spec-digest"))
                    != Some(&identity)
                {
                    return Err("engine_identity_conflict");
                }
                if existing
                    .status
                    .and_then(|s| s.phase)
                    .is_some_and(|p| matches!(p.as_str(), "Failed" | "Succeeded"))
                {
                    return Err("engine_lost");
                }
                Ok(())
            }
            Err(_) => Err("engine_startup_failed"),
        }
    }

    /// Observe retained Docker state without creating replacement resources.
    /// # Errors
    /// Reports unavailable Kubernetes observations.
    pub async fn engine_alive(
        &self,
        session: &str,
        runtime: &WorkloadRuntime,
    ) -> Result<bool, &'static str> {
        if runtime
            .build
            .as_ref()
            .and_then(|b| b.engine.as_ref())
            .is_none()
        {
            return Ok(true);
        }
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let pod = pods
            .get_opt(&format!("{session}-engine"))
            .await
            .map_err(|_| "kubernetes_unavailable")?;
        Ok(pod.is_some_and(|pod| {
            pod.metadata.deletion_timestamp.is_none()
                && pod.status.is_some_and(|status| {
                    !matches!(status.phase.as_deref(), Some("Failed" | "Succeeded"))
                        && status.container_statuses.as_ref().is_none_or(|containers| {
                            containers.iter().all(|c| {
                                c.restart_count == 0
                                    && c.state.as_ref().is_none_or(|s| s.terminated.is_none())
                            })
                        })
                })
        }))
    }

    pub(crate) async fn cleanup_engine(&self, session: &str) -> Result<(), &'static str> {
        let name = format!("{session}-engine");
        let ns = &self.configuration.namespace;
        for result in [
            Api::<Pod>::namespaced(self.client.clone(), ns)
                .delete(&name, &DeleteParams::default())
                .await
                .map(|_| ()),
            Api::<Service>::namespaced(self.client.clone(), ns)
                .delete(&name, &DeleteParams::default())
                .await
                .map(|_| ()),
            Api::<Secret>::namespaced(self.client.clone(), ns)
                .delete(&name, &DeleteParams::default())
                .await
                .map(|_| ()),
            Api::<NetworkPolicy>::namespaced(self.client.clone(), ns)
                .delete(&format!("{name}-clients"), &DeleteParams::default())
                .await
                .map(|_| ()),
            Api::<NetworkPolicy>::namespaced(self.client.clone(), ns)
                .delete(&name, &DeleteParams::default())
                .await
                .map(|_| ()),
        ] {
            match result {
                Ok(()) => {}
                Err(kube::Error::Api(e)) if e.code == 404 => {}
                Err(_) => return Err("engine_cleanup_failed"),
            }
        }
        if Api::<Pod>::namespaced(self.client.clone(), ns)
            .get_opt(&name)
            .await
            .map_err(|_| "engine_cleanup_failed")?
            .is_some()
        {
            return Err("engine_cleanup_pending");
        }
        Ok(())
    }
}
fn tolerate_exists<T>(result: Result<T, kube::Error>) -> Result<(), &'static str> {
    match result {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(e)) if e.code == 409 => Ok(()),
        Err(_) => Err("engine_startup_failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_orchestrator_application::execution_settings::{BuildRuntime, EngineRuntime};
    fn runtime(mode: &str) -> WorkloadRuntime {
        WorkloadRuntime {
            image: "ghcr.io/test/sandbox:lock".into(),
            platform: "linux/arm64".into(),
            service_account: "runtime-coding".into(),
            pull_secret: Some("pull".into()),
            build: Some(BuildRuntime {
                configuration_revision: 0,
                image: format!("ghcr.io/test/builder@sha256:{}", "a".repeat(64)),
                service_account: "runtime-builder".into(),
                registry_secret: "push".into(),
                signing_secret: "signing".into(),
                engine_secret: String::new(),
                engine_host: String::new(),
                isolated_engine: false,
                engine: Some(EngineRuntime {
                    image: format!("docker.io/library/docker@sha256:{}", "b".repeat(64)),
                    security_mode: mode.into(),
                }),
            }),
        }
    }
    #[test]
    fn dedicated_engine_requires_builder_placement_and_scoped_mounts()
    -> Result<(), Box<dyn std::error::Error>> {
        let pod = manifest("build-engine", &runtime("dedicated-builder"))?;
        let value = serde_json::to_value(pod)?;
        let spec = &value["spec"];
        assert!(spec["hostUsers"].is_null());
        assert_eq!(spec["nodeSelector"][workload::BUILDER_NODE_LABEL], "true");
        assert_eq!(spec["nodeSelector"]["kubernetes.io/arch"], "arm64");
        assert_eq!(spec["tolerations"][0]["key"], "ayni.io/builder");
        assert_eq!(spec["containers"][0]["securityContext"]["privileged"], true);
        assert_eq!(spec["automountServiceAccountToken"], false);
        assert!(spec["hostNetwork"].is_null());
        assert!(spec["hostPID"].is_null());
        assert!(!value.to_string().contains("hostPath"));
        let items = spec["volumes"][2]["secret"]["items"]
            .as_array()
            .ok_or("TLS items")?;
        assert_eq!(items.len(), 3);
        assert!(!items.iter().any(|i| i["key"] == "key.pem"));
        assert!(!value.to_string().contains("signing"));
        assert!(!value.to_string().contains("push"));
        Ok(())
    }
    #[test]
    fn development_engine_mode_is_explicit_and_attempts_never_share_endpoints()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = runtime("local-development");
        let first = bind(&original, "first");
        let second = bind(&original, "second");
        assert_ne!(
            first.build.as_ref().ok_or("build")?.engine_host,
            second.build.as_ref().ok_or("build")?.engine_host
        );
        let pod = serde_json::to_value(manifest("first-engine", &first)?)?;
        assert!(pod["spec"]["hostUsers"].is_null());
        assert!(pod["spec"]["nodeSelector"][workload::BUILDER_NODE_LABEL].is_null());
        assert_eq!(
            pod["spec"]["containers"][0]["securityContext"]["privileged"],
            true
        );
        assert_eq!(original.build.ok_or("build")?.engine_host, "");
        Ok(())
    }
    #[test]
    fn engine_requires_supported_mode_and_pinned_image() {
        let mut value = runtime("dedicated-builder");
        let engine = value
            .build
            .as_mut()
            .and_then(|b| b.engine.as_mut())
            .unwrap_or_else(|| panic!("engine"));
        assert!(engine.validate().is_ok());
        engine.security_mode = "unsupported".into();
        assert!(engine.validate().is_err());
        engine.security_mode = "dedicated-builder".into();
        engine.image = "docker:latest".into();
        assert!(engine.validate().is_err());
    }
    #[test]
    fn client_and_server_tls_keys_are_distinct() -> Result<(), &'static str> {
        let value = tls("fixture-engine")?;
        assert_ne!(value["server-key.pem"], value["key.pem"]);
        assert!(
            value["ca.pem"]
                .as_str()
                .ok_or("ca")?
                .contains("BEGIN CERTIFICATE")
        );
        Ok(())
    }
}
