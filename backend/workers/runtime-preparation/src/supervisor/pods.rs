use super::{Failure, Pod, StageClaim, Supervisor};
use ayni_orchestrator_events::runtime_preparation::RuntimeProfile;
use serde_json::{Value, json};
pub(super) fn name(c: &StageClaim) -> Result<String, Failure> {
    let id = uuid::Uuid::parse_str(&c.operation.id)
        .map_err(|_| Failure::InvalidInput)?
        .simple()
        .to_string();
    Ok(format!(
        "runtime-{}-{}-{}",
        &id[..20],
        c.attempt.stage.replace('_', "-"),
        c.attempt.generation
    ))
}
pub(super) fn output_role(c: &StageClaim) -> Option<&'static str> {
    match c.attempt.stage.as_str() {
        "acquire_source" => Some("source"),
        "pull_existing" | "pull_final" => Some("pull"),
        "build" => Some("candidate"),
        "freeze_candidate" => Some("frozen"),
        "certify" => Some("certification"),
        "finalize" => Some("final"),
        _ => None,
    }
}
pub(super) fn resource(c: &StageClaim, role: &str) -> String {
    format!("runtime-{}-{role}", c.operation.id)
}
pub(super) fn labels(c: &StageClaim) -> Value {
    json!({"app.kubernetes.io/managed-by":"ayni-runtime-supervisor","ayni.build/operation":c.operation.id,"ayni.build/stage":c.attempt.stage,"ayni.build/generation":c.attempt.generation.to_string()})
}
pub(super) fn verify(p: &Pod, c: &StageClaim) -> Result<(), Failure> {
    let labels = p
        .metadata
        .labels
        .as_ref()
        .ok_or(Failure::ArtifactConflict)?;
    let annotations = p
        .metadata
        .annotations
        .as_ref()
        .ok_or(Failure::ArtifactConflict)?;
    if p.metadata.name.as_deref() != Some(name(c)?.as_str())
        || labels.get("ayni.build/operation") != Some(&c.operation.id)
        || labels.get("ayni.build/generation") != Some(&c.attempt.generation.to_string())
        || labels.get("ayni.build/stage") != Some(&c.attempt.stage)
        || annotations.get("ayni.build/command-digest") != Some(&c.attempt.command_digest)
        || annotations.get("ayni.build/frozen-input-digest")
            != Some(&c.operation.frozen_input_digest)
        || (!c.attempt.pod_uid.is_empty() && p.metadata.uid.as_deref() != Some(&c.attempt.pod_uid))
    {
        return Err(Failure::ArtifactConflict);
    }
    Ok(())
}
fn stage_volumes(c: &StageClaim, profile: &RuntimeProfile) -> (Vec<Value>, Vec<Value>) {
    let stage = c.attempt.stage.as_str();
    let mut volumes = vec![
        json!({"name":"scratch","emptyDir":{"sizeLimit":format!("{}Mi",profile.max_storage_mib)}}),
        json!({"name":"home","emptyDir":{"sizeLimit":"64Mi"}}),
        json!({"name":"tmp","emptyDir":{"sizeLimit":"256Mi"}}),
    ];
    let mut mounts = vec![
        json!({"name":"scratch","mountPath":"/scratch"}),
        json!({"name":"home","mountPath":"/home/podman"}),
        json!({"name":"tmp","mountPath":"/tmp"}),
    ];
    for role in [
        "source",
        "candidate",
        "frozen",
        "certification",
        "final",
        "pull",
    ] {
        let output = output_role(c) == Some(role);
        let input = match stage {
            "build" | "verify_content" | "verify_compatibility" => role == "source",
            "freeze_candidate" => role == "candidate",
            "certify" => matches!(role, "source" | "frozen"),
            "finalize" => matches!(role, "frozen" | "certification"),
            "publish" => role == "final",
            _ => false,
        };
        if output || input {
            volumes.push(json!({"name":role,"persistentVolumeClaim":{"claimName":resource(c,role),"readOnly":!output}}));
            mounts.push(json!({"name":role,"mountPath":if output{"/outputs".to_string()}else{format!("/inputs/{role}")},"readOnly":!output}));
        }
    }
    (volumes, mounts)
}
fn stage_environment(s: &Supervisor, c: &StageClaim) -> Vec<Value> {
    let stage = c.attempt.stage.as_str();
    let env = vec![
        json!({"name":"ORCHESTRATOR_RUNTIME_OPERATION","value":c.operation.id}),
        json!({"name":"ORCHESTRATOR_RUNTIME_STAGE","value":stage}),
        json!({"name":"ORCHESTRATOR_RUNTIME_GENERATION","value":c.attempt.generation.to_string()}),
        json!({"name":"ORCHESTRATOR_RUNTIME_COMMAND_DIGEST","value":c.attempt.command_digest}),
        json!({"name":"ORCHESTRATOR_RUNTIME_POD_UID","valueFrom":{"fieldRef":{"fieldPath":"metadata.uid"}}}),
        json!({"name":"ORCHESTRATOR_RUNTIME_AUTHORITY_URL","value":s.origin}),
        json!({"name":"ORCHESTRATOR_RUNTIME_CAPABILITY_PATH","value":"/runtime-capability/token"}),
    ];
    env
}
fn deadline_seconds(deadline: u64, now: u64, maximum: u32) -> Result<u64, Failure> {
    let remaining = deadline
        .checked_sub(now)
        .filter(|value| *value > 0)
        .ok_or(Failure::ExecutionDeadline)?;
    let seconds = remaining.div_ceil(1000).min(u64::from(maximum));
    if seconds == 0 {
        return Err(Failure::ExecutionDeadline);
    }
    Ok(seconds)
}
fn active_deadline(c: &StageClaim, profile: &RuntimeProfile) -> Result<u64, Failure> {
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Failure::ExecutionDeadline)?
            .as_millis(),
    )
    .map_err(|_| Failure::ExecutionDeadline)?;
    deadline_seconds(c.attempt.deadline_unix_ms, now, profile.max_seconds)
}
fn stage_security(builder: bool) -> Value {
    let mut security = json!({"runAsUser":1000,"runAsGroup":1000,"readOnlyRootFilesystem":true,"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]},"seccompProfile":{"type":"RuntimeDefault"}});
    if builder {
        security["allowPrivilegeEscalation"] = json!(true);
        security["capabilities"]["add"] = json!(["SETUID", "SETGID"]);
        security["seccompProfile"] = json!({"type":"Unconfined"});
        security["procMount"] = json!("Unmasked");
    }
    security
}
pub(super) fn render(s: &Supervisor, c: &StageClaim) -> Result<Pod, Failure> {
    let profile: RuntimeProfile = serde_json::from_str(&c.operation.frozen.profile_json)
        .map_err(|_| Failure::InvalidInput)?;
    let stage = c.attempt.stage.as_str();
    let verify = matches!(stage, "verify_content" | "verify_compatibility");
    let builder = stage == "build";
    let (mut volumes, mut mounts) = stage_volumes(c, &profile);
    let mut security = stage_security(builder);
    let mut env = stage_environment(s, c);
    let mut command = json!(["/usr/local/bin/ayni-runtime-worker"]);
    let mut image = profile.worker_image.clone();
    let mut init = vec![];
    if matches!(stage, "pull_existing" | "pull_final") {
        for mount in &mut mounts {
            if mount.get("name").and_then(Value::as_str) == Some("pull") {
                mount["subPath"] = json!(stage);
            }
        }
        init.push(json!({"name":"prepare-pull-output","image":profile.worker_image,"imagePullPolicy":s.pull_policy,"command":["/bin/sh","-c",format!("mkdir -p /pull/{stage}")],"securityContext":security,"volumeMounts":[{"name":"pull","mountPath":"/pull"}]}));
    }
    if !verify && stage != "cleanup" {
        volumes.push(json!({"name":"capability","secret":{"secretName":resource(c,&format!("cap-{}-{}",stage,c.attempt.generation)),"defaultMode":288}}));
        mounts.push(json!({"name":"capability","mountPath":"/runtime-capability","readOnly":true}));
    }
    if builder {
        volumes.push(json!({"name":"engine","configMap":{"name":"runtime-preparation-engine"}}));
        mounts.push(json!({"name":"engine","mountPath":"/runtime-config","readOnly":true}));
    }
    if stage == "certify" {
        volumes.push(json!({"name":"signer","secret":{"secretName":c.operation.frozen.signer_secret_name,"defaultMode":288,"items":[{"key":"seed","path":"seed"}]}}));
        mounts.push(json!({"name":"signer","mountPath":"/runtime-signer","readOnly":true}));
    }
    if verify {
        image.clone_from(&c.operation.evidence.candidate_image);
        if !image.contains("@sha256:") {
            return Err(Failure::ArtifactInvalid);
        }
        volumes.push(json!({"name":"trusted","emptyDir":{"sizeLimit":"64Mi"}}));
        mounts.push(json!({"name":"trusted","mountPath":"/trusted","readOnly":true}));
        init.push(json!({"name":"install-probes","image":profile.verifier_image,"imagePullPolicy":s.pull_policy,"command":["/runtime-worker","--install-probes"],"volumeMounts":[{"name":"trusted","mountPath":"/trusted"}],"securityContext":{"runAsUser":0,"runAsGroup":0,"readOnlyRootFilesystem":true,"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]},"seccompProfile":{"type":"RuntimeDefault"}}}));
        security["runAsUser"] = json!(10001);
        security["runAsGroup"] = json!(10001);
        if stage == "verify_content" {
            mounts.push(
                json!({"name":"source","mountPath":"/source","subPath":"locked","readOnly":true}),
            );
            command = json!([
                "/trusted/ayni-runtime-verifier",
                "--source",
                "/source",
                "--expected-platform",
                profile.platform,
                "--expected-cli-sha256",
                profile.trusted_cli_sha256
            ]);
        } else {
            // Kubernetes does not retain effective caps across non-root exec here.
            // This trusted static parent is root only inside the pod user namespace;
            // the repository child still runs as 10002 with no groups/capabilities.
            security["runAsUser"] = json!(0);
            security["runAsGroup"] = json!(0);
            for volume in ["workspace", "runtime-evidence", "compat-cache", "quality"] {
                volumes.push(json!({"name":volume,"emptyDir":{"sizeLimit":format!("{}Mi",profile.max_storage_mib)}}));
            }
            security["capabilities"]["add"] = json!(["SETUID", "SETGID"]);
            let compat_env = json!([
             {"name":"ORCHESTRATOR_RUNTIME_TRUSTED_CLI_SHA256","value":profile.trusted_cli_sha256},
             {"name":"ORCHESTRATOR_RUNTIME_CERTIFICATE_DIGEST","value":c.operation.evidence.certificate_digest},
             {"name":"ORCHESTRATOR_RUNTIME_LOCK_FINGERPRINT","value":c.operation.frozen.lock_fingerprint},
             {"name":"ORCHESTRATOR_RUNTIME_SOURCE_DIGEST","value":c.operation.evidence.source_digest},
             {"name":"ORCHESTRATOR_RUNTIME_PLATFORM","value":profile.platform},
             {"name":"ORCHESTRATOR_RUNTIME_MAX_SECONDS","value":profile.max_seconds.to_string()}
            ]);
            env = serde_json::from_value(compat_env.clone()).map_err(|_| Failure::InvalidInput)?;

            mounts.retain(|m| m.get("mountPath").and_then(Value::as_str) != Some("/tmp"));
            let prepare_mounts = json!([{"name":"trusted","mountPath":"/trusted","readOnly":true},{"name":"source","mountPath":"/inputs/source","readOnly":true},{"name":"workspace","mountPath":"/workspace"},{"name":"runtime-evidence","mountPath":"/runtime-evidence"},{"name":"compat-cache","mountPath":"/tmp"}]);
            init.push(json!({"name":"compat-prepare","env":compat_env,"image":image,"command":["/trusted/runtime-worker","--compat-prepare"],"securityContext":security,"volumeMounts":prepare_mounts,"resources":{"limits":{"cpu":format!("{}m",profile.max_cpu_millis),"memory":format!("{}Mi",profile.max_memory_mib)}}}));
            mounts.extend([
                json!({"name":"workspace","mountPath":"/workspace","readOnly":true}),
                json!({"name":"quality","mountPath":"/workspace/.ayni"}),
                json!({"name":"runtime-evidence","mountPath":"/runtime-evidence","readOnly":true}),
                json!({"name":"compat-cache","mountPath":"/tmp"}),
            ]);
            command = json!(["/trusted/runtime-worker", "--compat-launch"]);
        }
    }
    if stage == "cleanup" {
        command = json!(["/bin/sh", "-c", "exec sleep 300"]);
    }
    let mut container = json!({"name":"stage","image":image,"imagePullPolicy":s.pull_policy,"command":command,"securityContext":security,"volumeMounts":mounts,"resources":{"requests":{"cpu":format!("{}m",profile.max_cpu_millis.min(100)),"memory":format!("{}Mi",profile.max_memory_mib.min(128))},"limits":{"cpu":format!("{}m",profile.max_cpu_millis),"memory":format!("{}Mi",profile.max_memory_mib),"ephemeral-storage":format!("{}Mi",profile.max_storage_mib+1024)}}});
    if (!verify || stage == "verify_compatibility") && stage != "cleanup" {
        container["env"] = json!(env);
    }
    let mut value = json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name(c)?,"namespace":s.namespace,"labels":labels(c),"annotations":{"ayni.build/command-digest":c.attempt.command_digest,"ayni.build/frozen-input-digest":c.operation.frozen_input_digest}},"spec":{"hostUsers":false,"automountServiceAccountToken":false,"restartPolicy":"Never","terminationGracePeriodSeconds":5,"activeDeadlineSeconds":active_deadline(c,&profile)?,"securityContext":{"fsGroup":if stage=="verify_compatibility"{10002}else{1000},"fsGroupChangePolicy":"OnRootMismatch"},"containers":[container],"initContainers":init,"volumes":volumes}});
    if verify {
        value["spec"]["imagePullSecrets"] =
            json!([{"name":resource(c,&format!("pull-{}-{}",stage,c.attempt.generation))}]);
    }
    serde_json::from_value(value).map_err(|_| Failure::InvalidInput)
}

#[cfg(test)]
mod tests {
    use super::{Failure, StageClaim, Supervisor, deadline_seconds, render, verify};
    use kube::Client;
    use serde_json::json;
    fn claim(stage: &str) -> Result<StageClaim, Box<dyn std::error::Error>> {
        let mut frozen = json!({"sources":[],"installation_id":1,"provider_repository_id":1,"source_connection_revision":1,"registry_connection_revision":1,"signer_revision":1});
        for key in [
            "repository_id",
            "inspection_id",
            "source_commit",
            "source_tree",
            "source_connection_id",
            "source_credential_revision",
            "repository_full_name",
            "registry_connection_id",
            "registry_credential_revision",
            "registry_destination",
            "discovery_tag",
            "reusable_image",
            "publication_tag",
            "signer_id",
            "signer_active_revision",
            "signer_key_id",
            "signer_public_key",
            "signer_secret_name",
            "signer_secret_uid",
            "policy",
            "lock",
            "policy_digest",
            "lock_digest",
            "lock_fingerprint",
        ] {
            frozen[key] = json!("fixture");
        }
        frozen["profile_json"]=json!(json!({"profile_id":"fixture","qualified":true,"platform":"linux/arm64","worker_image":"worker@sha256:fixture","executor_image":"executor@sha256:fixture","verifier_image":"verifier@sha256:fixture","trusted_cli_sha256":"fixture","max_cpu_millis":2000,"max_memory_mib":2048,"max_storage_mib":8192,"max_seconds":1800,"max_builds":1}).to_string());
        Ok(serde_json::from_value(
            json!({"operation":{"id":"891964ce-ecf6-444a-a4c1-84e2cc8f4ffe","idempotency_key":"fixture","repository_id":"fixture","inspection_id":"fixture","registry_connection_id":"fixture","signer_id":"fixture","platform":"linux/arm64","state":"running","stage":stage,"safe_reason":"","generation":1,"allow_build":true,"allow_replace":false,"max_builds":1,"builds_started":0,"deadline_unix_ms":9_999_999_999_999_u64,"accepted_at_unix_ms":1,"updated_at_unix_ms":1,"frozen_input_digest":"fixture","frozen":frozen,"evidence":{"candidate_image":"registry.test/image@sha256:fixture"}},"attempt":{"id":"fixture","operation_id":"891964ce-ecf6-444a-a4c1-84e2cc8f4ffe","stage":stage,"generation":1,"command_digest":"fixture","pod_uid":"","state":"claimed","deadline_unix_ms":9_999_999_999_999_u64,"accepted_at_unix_ms":1,"report_digest":""}}),
        )?)
    }
    fn supervisor() -> Result<Supervisor, Box<dyn std::error::Error>> {
        Ok(Supervisor {
            client: Client::try_from(kube::Config::new("http://127.0.0.1:1".parse()?))?,
            http: reqwest::Client::new(),
            namespace: "fixture-runtime".into(),
            origin: "http://authority".into(),
            authority: "synthetic-fixture-authority-only".into(),
            storage_class: None,
            pull_policy: "Never".into(),
        })
    }
    #[tokio::test]
    async fn verifier_has_no_capability_and_fenced_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let s = supervisor()?;
        let mut c = claim("verify_content")?;
        let mut p = render(&s, &c)?;
        let value = serde_json::to_value(&p)?;
        assert!(!value.to_string().contains("runtime-capability"));
        assert!(!value.to_string().contains("synthetic-fixture-authority"));
        assert_eq!(value["spec"]["hostUsers"], false);
        assert_eq!(value["spec"]["automountServiceAccountToken"], false);
        assert_eq!(
            value["spec"]["containers"][0]["securityContext"]["runAsUser"],
            10001
        );
        p.metadata.uid = Some("original".into());
        c.attempt.pod_uid = "original".into();
        assert!(verify(&p, &c).is_ok());
        p.metadata.uid = Some("replacement".into());
        assert_eq!(verify(&p, &c), Err(Failure::ArtifactConflict));
        Ok(())
    }
    #[tokio::test]
    async fn only_builder_receives_user_namespace_helper_exceptions()
    -> Result<(), Box<dyn std::error::Error>> {
        let s = supervisor()?;
        for stage in ["build", "certify", "freeze_candidate"] {
            let value = serde_json::to_value(render(&s, &claim(stage)?)?)?;
            let security = &value["spec"]["containers"][0]["securityContext"];
            assert_eq!(security["allowPrivilegeEscalation"], stage == "build");
            assert_eq!(
                security["seccompProfile"]["type"],
                if stage == "build" {
                    "Unconfined"
                } else {
                    "RuntimeDefault"
                }
            );
        }
        Ok(())
    }
    #[tokio::test]
    async fn pod_requests_fit_qualified_profile_limits() -> Result<(), Box<dyn std::error::Error>> {
        let supervisor = supervisor()?;
        for (cpu, memory) in [(1_u32, 64_u32), (2000, 2048)] {
            let mut claim = claim("build")?;
            let mut profile: serde_json::Value =
                serde_json::from_str(&claim.operation.frozen.profile_json)?;
            profile["max_cpu_millis"] = json!(cpu);
            profile["max_memory_mib"] = json!(memory);
            claim.operation.frozen.profile_json = profile.to_string();
            let pod = render(&supervisor, &claim)?;
            let resources = pod.spec.ok_or("spec missing")?.containers[0]
                .resources
                .clone()
                .ok_or("resources missing")?;
            let requested = resources.requests.ok_or("requests missing")?;
            assert!(requested["cpu"].0.trim_end_matches('m').parse::<u32>()? <= cpu);
            assert!(
                requested["memory"]
                    .0
                    .trim_end_matches("Mi")
                    .parse::<u32>()?
                    <= memory
            );
        }
        Ok(())
    }
    #[test]
    fn pod_execution_deadline_uses_remaining_attempt_authority() {
        assert_eq!(deadline_seconds(65_000, 5_000, 1800), Ok(60));
        assert_eq!(deadline_seconds(5_500, 5_000, 1800), Ok(1));
        assert_eq!(deadline_seconds(65_000, 5_000, 20), Ok(20));
        assert_eq!(
            deadline_seconds(5_000, 5_000, 1800),
            Err(Failure::ExecutionDeadline)
        );
        assert_eq!(
            deadline_seconds(4_000, 5_000, 1800),
            Err(Failure::ExecutionDeadline)
        );
    }
    #[tokio::test]
    async fn compatibility_parent_has_only_identity_drop_authority_in_the_pod_user_namespace()
    -> Result<(), Box<dyn std::error::Error>> {
        let value = serde_json::to_value(render(&supervisor()?, &claim("verify_compatibility")?)?)?;
        let context = &value["spec"]["containers"][0]["securityContext"];
        assert_eq!(value["spec"]["hostUsers"], false);
        assert_eq!(context["runAsUser"], 0);
        assert_eq!(context["runAsGroup"], 0);
        assert_eq!(context["allowPrivilegeEscalation"], false);
        assert_eq!(context["capabilities"]["drop"], json!(["ALL"]));
        assert_eq!(context["capabilities"]["add"], json!(["SETUID", "SETGID"]));
        assert_eq!(
            value["spec"]["initContainers"][1]["securityContext"],
            *context
        );
        assert!(!value.to_string().contains("runtime-capability"));
        Ok(())
    }
}
