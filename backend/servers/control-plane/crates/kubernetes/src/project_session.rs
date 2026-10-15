//! Fixed-command resident planner transport in the trusted controller namespace.
use crate::{execution::RuntimeKubernetes, project_agent::ProjectObservation};
use ayni_orchestrator_runtime::project_agent::{
    ProjectSessionReceipt, ProjectSessionSubmission, ProjectSessionTurn,
};
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::{
    Api,
    api::{AttachParams, DeleteParams, PostParams, Preconditions},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
const WORKER: &str = "/usr/local/bin/ayni-orchestrator-worker";
const RECEIPT_BYTES: usize = 262_144;

impl RuntimeKubernetes {
    /// Create or adopt one immutable live generation without reading Secret values.
    /// # Errors
    /// Rejects conflicting ownership, image, model binding or Kubernetes failures.
    pub async fn ensure_project_session(
        &self,
        name: &str,
        session_id: &str,
        generation: u64,
        model_secret: &str,
        model_uid: &str,
        model_revision: &str,
    ) -> Result<String, &'static str> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let secrets: Api<Secret> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let model = secrets
            .get_metadata(model_secret)
            .await
            .map_err(|_| "project_model_credential_unavailable")?;
        if model.metadata.uid.as_deref() != Some(model_uid)
            || model.metadata.resource_version.as_deref() != Some(model_revision)
        {
            return Err("project_model_credential_changed");
        }
        let expected = session_pod(
            name,
            session_id,
            generation,
            &self.configuration.worker_image,
            model_secret,
            model_uid,
            model_revision,
        )?;
        let pod = match pods.create(&PostParams::default(), &expected).await {
            Ok(pod) => pod,
            Err(kube::Error::Api(error)) if error.code == 409 => {
                pods.get(name).await.map_err(|_| "kubernetes_unavailable")?
            }
            Err(_) => return Err("kubernetes_unavailable"),
        };
        if !owned(&pod, name) || !spec_matches(&pod, &expected) {
            return Err("project_session_conflict");
        }
        pod.metadata.uid.ok_or("project_session_uid_missing")
    }
    /// Probe an idle generation without dispatching a turn or reading credentials.
    /// # Errors
    /// Foreign identity or a changed Pod specification cannot be adopted.
    pub async fn project_session_alive(
        &self,
        name: &str,
        pod_uid: &str,
    ) -> Result<bool, &'static str> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let Some(pod) = pods
            .get_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
        else {
            return Ok(false);
        };
        if !owned(&pod, name) || pod.metadata.uid.as_deref() != Some(pod_uid) {
            return Err("project_session_conflict");
        }
        validate_pod_spec(&pod)?;
        Ok(idle_compatible(&pod, &self.configuration.worker_image))
    }
    /// Stream scoped source credentials only into the trusted checkout container.
    /// # Errors
    /// Pending or transient transport failures leave the same frozen turn retryable.
    pub async fn dispatch_project_session(
        &self,
        name: &str,
        turn: &ProjectSessionTurn,
        source_tokens: BTreeMap<String, String>,
    ) -> Result<(), &'static str> {
        let pod = self
            .session_for_turn(name, turn)
            .await?
            .ok_or("project_session_missing")?;
        if !running(&pod) {
            return Err("project_session_starting");
        }
        self.check_session_model(&pod).await?;
        let bytes = serde_json::to_vec(&ProjectSessionSubmission {
            turn: turn.clone(),
            source_tokens,
        })
        .map_err(|_| "project_session_input_invalid")?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("project_session_input_oversized");
        }
        let result = self
            .session_exec(
                name,
                turn,
                "checkout",
                "project-session-submit",
                &bytes,
                1024,
            )
            .await?;
        let ack: serde_json::Value =
            serde_json::from_slice(&result).map_err(|_| "project_session_receipt_invalid")?;
        if ack != json!({"accepted":true}) {
            return Err("project_session_receipt_invalid");
        }
        Ok(())
    }
    /// Read a bounded receipt from its owned container, never from cumulative logs.
    /// # Errors
    /// Rejects stale Pod identities or transport failures without retrying model work.
    pub async fn observe_project_session(
        &self,
        name: &str,
        turn: &ProjectSessionTurn,
    ) -> Result<ProjectObservation, &'static str> {
        let Some(pod) = self.session_for_turn(name, turn).await? else {
            return Ok(ProjectObservation::Missing);
        };
        if failed(&pod) {
            return Ok(ProjectObservation::Failed("project_agent_failed"));
        }
        if !running(&pod) {
            return Ok(ProjectObservation::Running);
        }
        let bytes = serde_json::to_vec(turn).map_err(|_| "project_session_input_invalid")?;
        let result = self
            .session_exec(
                name,
                turn,
                "research",
                "project-session-result",
                &bytes,
                RECEIPT_BYTES,
            )
            .await?;
        let receipt: Option<ProjectSessionReceipt> =
            serde_json::from_slice(&result).map_err(|_| "project_session_receipt_invalid")?;
        receipt.map_or(Ok(ProjectObservation::Running), |receipt| {
            accepted_receipt(turn, receipt)
        })
    }
    /// Delete only an owned generation using its observed UID and wait for absence.
    /// An unknown recorded UID still probes ownership after a lost create response.
    /// # Errors
    /// Conflicting ownership or unavailable Kubernetes prevents cleanup acknowledgement.
    pub async fn cleanup_project_session(
        &self,
        name: &str,
        pod_uid: &str,
    ) -> Result<bool, &'static str> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let Some(pod) = pods
            .get_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
        else {
            return Ok(true);
        };
        if !owned(&pod, name)
            || (!pod_uid.is_empty() && pod.metadata.uid.as_deref() != Some(pod_uid))
        {
            return Err("project_session_conflict");
        }
        validate_pod_spec(&pod)?;
        let uid = pod
            .metadata
            .uid
            .clone()
            .ok_or("project_session_uid_missing")?;
        let params = DeleteParams {
            preconditions: Some(Preconditions {
                uid: Some(uid),
                resource_version: None,
            }),
            ..Default::default()
        };
        match pods.delete(name, &params).await {
            Ok(_) => Ok(false),
            Err(kube::Error::Api(error)) if error.code == 404 => Ok(true),
            Err(_) => Err("kubernetes_unavailable"),
        }
    }
    async fn check_session_model(&self, pod: &Pod) -> Result<(), &'static str> {
        let name = annotation(pod, "ayni.io/model-secret").ok_or("project_session_conflict")?;
        let secrets: Api<Secret> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let current = secrets
            .get_metadata(name)
            .await
            .map_err(|_| "project_model_credential_unavailable")?;
        if current.metadata.uid.as_deref() != annotation(pod, "ayni.io/model-uid")
            || current.metadata.resource_version.as_deref()
                != annotation(pod, "ayni.io/model-revision")
        {
            return Err("project_model_credential_changed");
        }
        Ok(())
    }
    async fn session_for_turn(
        &self,
        name: &str,
        turn: &ProjectSessionTurn,
    ) -> Result<Option<Pod>, &'static str> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let Some(pod) = pods
            .get_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
        else {
            return Ok(None);
        };
        if !owned(&pod, name)
            || pod.metadata.uid.as_deref() != Some(&turn.pod_uid)
            || annotation(&pod, "ayni.io/session-id") != Some(turn.session_id.as_str())
            || annotation(&pod, "ayni.io/session-generation")
                != Some(turn.generation.to_string().as_str())
        {
            return Err("project_session_conflict");
        }
        validate_pod_spec(&pod)?;
        Ok(Some(pod))
    }
    async fn session_exec(
        &self,
        name: &str,
        turn: &ProjectSessionTurn,
        container: &str,
        operation: &str,
        input: &[u8],
        maximum: usize,
    ) -> Result<Vec<u8>, &'static str> {
        self.session_for_turn(name, turn)
            .await?
            .ok_or("project_session_missing")?;
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let params = AttachParams::default()
            .container(container)
            .stdin(true)
            .stdout(true)
            .stderr(false)
            .max_stdout_buf_size(65_536);
        let command = [WORKER, operation];
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            let mut attached = pods
                .exec(name, command, &params)
                .await
                .map_err(|_| "project_session_transport_unavailable")?;
            let status = attached
                .take_status()
                .ok_or("project_session_transport_unavailable")?;
            let mut input_writer = attached
                .stdin()
                .ok_or("project_session_transport_unavailable")?;
            input_writer
                .write_u32(
                    u32::try_from(input.len()).map_err(|_| "project_session_input_oversized")?,
                )
                .await
                .map_err(|_| "project_session_transport_unavailable")?;
            input_writer
                .write_all(input)
                .await
                .map_err(|_| "project_session_transport_unavailable")?;
            input_writer
                .flush()
                .await
                .map_err(|_| "project_session_transport_unavailable")?;
            let output = attached
                .stdout()
                .ok_or("project_session_transport_unavailable")?;
            let mut bytes = Vec::new();
            output
                .take(u64::try_from(maximum + 1).map_err(|_| "project_session_input_oversized")?)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| "project_session_transport_unavailable")?;
            if bytes.len() > maximum {
                attached.abort();
                return Err("project_session_receipt_oversized");
            }
            let status = status
                .await
                .ok_or("project_session_transport_unavailable")?;
            if status.status.as_deref() != Some("Success") {
                return Err("project_session_command_unavailable");
            }
            attached
                .join()
                .await
                .map_err(|_| "project_session_transport_unavailable")?;
            Ok(bytes)
        })
        .await
        .map_err(|_| "project_session_transport_unavailable")??;
        self.session_for_turn(name, turn)
            .await?
            .ok_or("project_session_missing")?;
        Ok(result)
    }
}
fn accepted_receipt(
    turn: &ProjectSessionTurn,
    receipt: ProjectSessionReceipt,
) -> Result<ProjectObservation, &'static str> {
    if !turn.matches_receipt(&receipt) {
        return Err("project_session_receipt_stale");
    }
    if !receipt.failure.is_empty() {
        return Ok(ProjectObservation::Failed(match receipt.failure.as_str() {
            "project_agent_deadline_exceeded" => "project_agent_deadline_exceeded",
            "project_result_oversized" => "project_result_oversized",
            _ => "project_agent_model_failed",
        }));
    }
    let result = receipt.result.ok_or("project_session_receipt_invalid")?;
    if result.run_id != turn.request.run_id
        || result.model_calls > turn.request.max_model_calls
        || result.message.len() > 16_384
        || result.proposals.len() > 100
    {
        return Err("project_session_receipt_invalid");
    }
    Ok(ProjectObservation::Completed(result))
}
fn annotation<'a>(pod: &'a Pod, key: &str) -> Option<&'a str> {
    pod.metadata
        .annotations
        .as_ref()?
        .get(key)
        .map(String::as_str)
}
fn owned(pod: &Pod, name: &str) -> bool {
    pod.metadata.labels.as_ref().is_some_and(|labels| {
        labels
            .get("ayni.io/runtime-operation")
            .is_some_and(|value| value == name)
            && labels
                .get("app.kubernetes.io/component")
                .is_some_and(|value| value == "project-session")
    })
}
fn running(pod: &Pod) -> bool {
    pod.metadata.deletion_timestamp.is_none()
        && pod
            .status
            .as_ref()
            .and_then(|s| s.container_statuses.as_ref())
            .is_some_and(|statuses| {
                statuses.len() == 2
                    && statuses.iter().all(|s| {
                        s.state
                            .as_ref()
                            .is_some_and(|state| state.running.is_some())
                    })
            })
}
fn idle_compatible(pod: &Pod, current_image: &str) -> bool {
    !failed(pod)
        && pod.spec.as_ref().is_some_and(|spec| {
            spec.containers
                .iter()
                .all(|container| container.image.as_deref() == Some(current_image))
        })
}

fn failed(pod: &Pod) -> bool {
    pod.metadata.deletion_timestamp.is_some()
        || pod.status.as_ref().is_some_and(|status| {
            status.phase.as_deref() == Some("Failed")
                || status.container_statuses.as_ref().is_some_and(|statuses| {
                    statuses.iter().any(|status| {
                        status
                            .state
                            .as_ref()
                            .is_some_and(|state| state.terminated.is_some())
                    })
                })
        })
}
fn spec_matches(actual: &Pod, expected: &Pod) -> bool {
    serde_json::to_value(&actual.spec)
        .ok()
        .zip(serde_json::to_value(&expected.spec).ok())
        .is_some_and(|(a, e)| contains_expected(&a, &e, ""))
        && actual.spec.as_ref().is_some_and(|s| {
            s.init_containers.as_ref().is_none_or(Vec::is_empty)
                && s.ephemeral_containers.as_ref().is_none_or(Vec::is_empty)
        })
        && [
            "ayni.io/session-spec-digest",
            "ayni.io/session-id",
            "ayni.io/session-generation",
            "ayni.io/model-secret",
            "ayni.io/model-uid",
            "ayni.io/model-revision",
        ]
        .iter()
        .all(|key| annotation(actual, key) == annotation(expected, key))
}
fn contains_expected(actual: &serde_json::Value, expected: &serde_json::Value, path: &str) -> bool {
    match expected {
        serde_json::Value::Object(expected) => actual.as_object().is_some_and(|actual| {
            expected.iter().all(|(key, value)| {
                actual.get(key).map_or_else(
                    || {
                        path.is_empty()
                            && matches!(key.as_str(), "hostNetwork" | "hostPID" | "hostIPC")
                            && value == false
                    },
                    |actual| contains_expected(actual, value, &format!("{path}/{key}")),
                )
            }) && actual
                .iter()
                .all(|(key, value)| expected.contains_key(key) || server_default(path, key, value))
        }),
        serde_json::Value::Array(expected) => actual.as_array().is_some_and(|actual| {
            actual.len() == expected.len()
                && actual.iter().zip(expected).all(|(actual, expected)| {
                    contains_expected(actual, expected, &format!("{path}/*"))
                })
        }),
        _ => actual == expected,
    }
}
// Admission may add these defaults, but never additional execution, credential,
// mount or security fields. Keep this list explicit rather than trusting extras.
fn server_default(path: &str, key: &str, value: &serde_json::Value) -> bool {
    match (path, key) {
        ("", "nodeName") => value.as_str().is_some_and(|name| !name.is_empty()),
        ("", "dnsPolicy") => value == "ClusterFirst",
        ("", "serviceAccount") => value == "runtime-coding",
        ("", "enableServiceLinks") => value == true,
        ("", "preemptionPolicy") => value == "PreemptLowerPriority",
        ("", "priority") => value == 0,
        ("", "schedulerName") => value == "default-scheduler",
        ("", "terminationGracePeriodSeconds") => value == 30,
        ("", "tolerations") => default_tolerations(value),
        ("/containers/*", "imagePullPolicy") => value == "Always" || value == "IfNotPresent",
        ("/containers/*", "terminationMessagePath") => value == "/dev/termination-log",
        ("/containers/*", "terminationMessagePolicy") => value == "File",
        ("/containers/*/env/*/valueFrom/fieldRef", "apiVersion") => value == "v1",
        ("/containers/*/volumeMounts/*", "readOnly") => value == false,
        ("/volumes/*/secret", "defaultMode") => value == 420,
        _ => false,
    }
}
fn default_tolerations(value: &serde_json::Value) -> bool {
    value.as_array().is_some_and(|items| {
        items.len() <= 2 && items.iter().all(|item| {
            let Some(key) = item.get("key").and_then(serde_json::Value::as_str) else {
                return false;
            };
            matches!(key, "node.kubernetes.io/not-ready" | "node.kubernetes.io/unreachable")
                && *item == json!({"key":key,"operator":"Exists","effect":"NoExecute","tolerationSeconds":300})
        })
    })
}
fn validate_pod_spec(pod: &Pod) -> Result<(), &'static str> {
    let spec = pod.spec.as_ref().ok_or("project_session_conflict")?;
    let image = spec
        .containers
        .first()
        .and_then(|c| c.image.as_deref())
        .ok_or("project_session_conflict")?;
    let generation = annotation(pod, "ayni.io/session-generation")
        .ok_or("project_session_conflict")?
        .parse()
        .map_err(|_| "project_session_conflict")?;
    let expected = session_pod(
        pod.metadata
            .name
            .as_deref()
            .ok_or("project_session_conflict")?,
        annotation(pod, "ayni.io/session-id").ok_or("project_session_conflict")?,
        generation,
        image,
        annotation(pod, "ayni.io/model-secret").ok_or("project_session_conflict")?,
        annotation(pod, "ayni.io/model-uid").ok_or("project_session_conflict")?,
        annotation(pod, "ayni.io/model-revision").ok_or("project_session_conflict")?,
    )?;
    if !spec_matches(pod, &expected) {
        return Err("project_session_conflict");
    }
    Ok(())
}
fn session_pod(
    name: &str,
    session: &str,
    generation: u64,
    image: &str,
    model: &str,
    uid: &str,
    revision: &str,
) -> Result<Pod, &'static str> {
    if name.is_empty()
        || name.len() > 63
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || session.is_empty()
        || session.len() > 256
        || generation == 0
    {
        return Err("project_session_input_invalid");
    }
    let security = json!({"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}});
    let env = json!([{"name":"PROJECT_SESSION_ID","value":session},{"name":"PROJECT_SESSION_GENERATION","value":generation.to_string()},{"name":"POD_UID","valueFrom":{"fieldRef":{"fieldPath":"metadata.uid"}}}]);
    let mut pod:Pod=serde_json::from_value(json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"labels":{"ayni.io/runtime-operation":name,"app.kubernetes.io/component":"project-session"},"annotations":{"ayni.io/session-id":session,"ayni.io/session-generation":generation.to_string(),"ayni.io/model-secret":model,"ayni.io/model-uid":uid,"ayni.io/model-revision":revision}},"spec":{"restartPolicy":"Never","automountServiceAccountToken":false,"serviceAccountName":"runtime-coding","hostNetwork":false,"hostPID":false,"hostIPC":false,"shareProcessNamespace":false,"affinity":crate::workload::trusted_node_affinity(),"securityContext":{"runAsNonRoot":true,"runAsUser":10001,"runAsGroup":10001,"fsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},"containers":[
        {"name":"checkout","image":image,"args":["project-session-checkout"],"securityContext":security,"env":env,"resources":{"requests":{"cpu":"100m","memory":"128Mi"},"limits":{"cpu":"1","memory":"512Mi"}},"volumeMounts":[{"name":"workspace","mountPath":"/workspace"},{"name":"results","mountPath":"/results","readOnly":true},{"name":"checkout-scratch","mountPath":"/scratch"},{"name":"checkout-scratch","mountPath":"/tmp"}]},
        {"name":"research","image":image,"args":["project-session-research"],"securityContext":security,"env":env,"resources":{"requests":{"cpu":"100m","memory":"128Mi"},"limits":{"cpu":"1","memory":"1Gi"}},"volumeMounts":[{"name":"workspace","mountPath":"/workspace","readOnly":true},{"name":"results","mountPath":"/results"},{"name":"research-scratch","mountPath":"/scratch"},{"name":"research-scratch","mountPath":"/tmp"},{"name":"model","mountPath":"/model/api-key","subPath":"api-key","readOnly":true}]}
    ],"volumes":[{"name":"workspace","emptyDir":{"sizeLimit":"1Gi"}},{"name":"results","emptyDir":{"sizeLimit":"1Mi"}},{"name":"checkout-scratch","emptyDir":{"sizeLimit":"128Mi"}},{"name":"research-scratch","emptyDir":{"sizeLimit":"128Mi"}},{"name":"model","secret":{"secretName":model,"items":[{"key":"api-key","path":"api-key"}]}}]}})).map_err(|_|"project_session_input_invalid")?;
    let digest = format!(
        "sha256:{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&pod.spec, session, generation, model, uid, revision))
                .map_err(|_| "project_session_input_invalid")?
        )
    );
    pod.metadata
        .annotations
        .get_or_insert_default()
        .insert("ayni.io/session-spec-digest".into(), digest);
    Ok(pod)
}

#[cfg(test)]
#[path = "project_session_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "project_session_tests.rs"]
mod tests;
