//! The trusted broker supervisor provisions credential-free conversation workers.
//! Session identity and generation originate in the authoritative command bytes.
use super::{Configuration, RuntimeResult};
use crate::session_worker::{ExecutionResult, digest};
use ayni_orchestrator_events::{v1::SessionExecutionScope, workflow::decode_command};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Secret};
use kube::{
    Api, Client,
    api::{DeleteParams, PostParams},
};
use rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::json;
use std::{env, io, time::Duration};

fn name(scope: &SessionExecutionScope) -> RuntimeResult<String> {
    let id = uuid::Uuid::parse_str(&scope.session_id)?;
    if scope.generation == 0 || !(30..=86400).contains(&scope.idle_timeout_seconds) {
        return Err(io::Error::other("session identity invalid").into());
    }
    Ok(format!("session-{}-{}", id.simple(), scope.generation))
}

fn namespace() -> RuntimeResult<String> {
    let value = env::var("ORCHESTRATOR_SESSION_NAMESPACE")
        .map_err(|_| io::Error::other("isolated session namespace is required"))?;
    if value.is_empty()
        || value.len() > 63
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(io::Error::other("session namespace invalid").into());
    }
    Ok(value)
}

pub async fn execute(
    configuration: &Configuration,
    command_bytes: &[u8],
) -> RuntimeResult<ExecutionResult> {
    let (identity, _, command) = decode_command(command_bytes)?;
    let scope = command
        .session
        .as_ref()
        .ok_or_else(|| io::Error::other("session scope missing"))?;
    let binding = command
        .binding
        .as_ref()
        .ok_or_else(|| io::Error::other("session binding missing"))?;
    if binding.executor_generation != scope.generation {
        return Err(io::Error::other("session generation mismatch").into());
    }
    let image = env::var("ORCHESTRATOR_SESSION_IMAGE")
        .map_err(|_| io::Error::other("isolated session image is required"))?;
    let client = Client::try_default().await?;
    let namespace = namespace()?;
    let name = name(scope)?;
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);
    let secrets: Api<Secret> = Api::namespaced(client, &namespace);
    retire_previous_pod(&pods, scope, binding).await?;
    ensure_storage(&pvcs, &name, scope).await?;
    let token = ensure_token(&secrets, &name, scope).await?;
    ensure_pod(&pods, &name, scope, binding, &image).await?;
    let address = wait_address(&pods, &name).await?;
    retain_route(
        configuration,
        &identity.id,
        &name,
        &digest(command_bytes),
        command_bytes,
    )?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    if ayni_orchestrator_events::workflow::binding_provider(&binding.execution_binding_revision)
        .is_some()
    {
        return execute_provider(&http, &address, &token, command_bytes).await;
    }
    let mut response = http
        .post(format!("http://{address}:8081/v1/execute"))
        .bearer_auth(&token)
        .header("content-type", "application/octet-stream")
        .body(command_bytes.to_vec())
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 32768 {
            return Err(io::Error::other("session response bound exceeded").into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let response: ExecutionResult = serde_json::from_slice(&bytes)?;
    if response.response_content.len() > 16384 {
        return Err(io::Error::other("session response bound exceeded").into());
    }
    if response.command_id != identity.id || response.command_digest != digest(command_bytes) {
        return Err(io::Error::other("session result binding mismatch").into());
    }
    Ok(response)
}

// A newly accepted generation fences the immediately preceding generation.
// Delete only the owned Pod; unresolved ledger PVCs and bearer Secrets remain.
async fn retire_previous_pod(
    api: &Api<Pod>,
    scope: &SessionExecutionScope,
    binding: &ayni_orchestrator_events::v1::ConversationTurnBinding,
) -> RuntimeResult {
    if scope.generation <= 1 {
        return Ok(());
    }
    let previous = SessionExecutionScope {
        generation: scope.generation - 1,
        ..scope.clone()
    };
    let previous_name = name(&previous)?;
    if let Some(pod) = api.get_opt(&previous_name).await? {
        let delete = retirement_preconditions(&pod, &previous, binding)?;
        match api.delete(&previous_name, &delete).await {
            Ok(_) => {}
            Err(kube::Error::Api(error)) if error.code == 404 => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn retirement_preconditions(
    pod: &Pod,
    scope: &SessionExecutionScope,
    binding: &ayni_orchestrator_events::v1::ConversationTurnBinding,
) -> RuntimeResult<DeleteParams> {
    verify_labels(pod.metadata.labels.as_ref(), scope)?;
    if !pod.metadata.labels.as_ref().is_some_and(|labels| {
        labels
            .get("app.kubernetes.io/name")
            .is_some_and(|value| value == "ayni-conversation-session")
            && labels
                .get("app.kubernetes.io/part-of")
                .is_some_and(|value| value == "ayni-orchestrator")
    }) {
        return Err(io::Error::other("retired session ownership mismatch").into());
    }
    let env = pod
        .spec
        .as_ref()
        .and_then(|spec| {
            spec.containers
                .iter()
                .find(|container| container.name == "session")
        })
        .and_then(|container| container.env.as_ref())
        .ok_or_else(|| io::Error::other("retired session scope missing"))?;
    for (name, expected) in [
        (
            "ORCHESTRATOR_SESSION_WORKSPACE_ID",
            binding.workspace_id.as_str(),
        ),
        (
            "ORCHESTRATOR_SESSION_PROJECT_ID",
            binding.project_id.as_str(),
        ),
        (
            "ORCHESTRATOR_SESSION_CONVERSATION_ID",
            binding.conversation_id.as_str(),
        ),
    ] {
        if !env
            .iter()
            .any(|item| item.name == name && item.value.as_deref() == Some(expected))
        {
            return Err(io::Error::other("retired session scope mismatch").into());
        }
    }
    let uid = pod
        .metadata
        .uid
        .clone()
        .filter(|uid| !uid.is_empty())
        .ok_or_else(|| io::Error::other("retired session UID missing"))?;
    Ok(DeleteParams {
        preconditions: Some(kube::api::Preconditions {
            uid: Some(uid),
            resource_version: None,
        }),
        ..DeleteParams::default()
    })
}

async fn ensure_storage(
    api: &Api<PersistentVolumeClaim>,
    name: &str,
    scope: &SessionExecutionScope,
) -> RuntimeResult {
    if let Some(existing) = api.get_opt(name).await? {
        verify_labels(existing.metadata.labels.as_ref(), scope)?;
        return Ok(());
    }
    let pvc: PersistentVolumeClaim = serde_json::from_value(
        json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"name":name,"labels":labels(scope)},"spec":{"accessModes":["ReadWriteOnce"],"resources":{"requests":{"storage":"512Mi"}}}}),
    )?;
    create_or_race(api.create(&PostParams::default(), &pvc).await)?;
    Ok(())
}

async fn ensure_token(
    api: &Api<Secret>,
    name: &str,
    scope: &SessionExecutionScope,
) -> RuntimeResult<String> {
    if let Some(existing) = api.get_opt(name).await? {
        verify_labels(existing.metadata.labels.as_ref(), scope)?;
        return token_value(existing);
    }
    let token = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    let secret: Secret = serde_json::from_value(
        json!({"apiVersion":"v1","kind":"Secret","metadata":{"name":name,"labels":labels(scope)},"immutable":true,"stringData":{"token":token}}),
    )?;
    create_or_race(api.create(&PostParams::default(), &secret).await)?;
    let retained = api.get(name).await?;
    verify_labels(retained.metadata.labels.as_ref(), scope)?;
    token_value(retained)
}

fn token_value(secret: Secret) -> RuntimeResult<String> {
    secret
        .data
        .and_then(|mut data| data.remove("token"))
        .and_then(|value| String::from_utf8(value.0).ok())
        .ok_or_else(|| io::Error::other("session token unavailable").into())
}

fn create_or_race<T>(result: Result<T, kube::Error>) -> RuntimeResult {
    match result {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(error)) if error.code == 409 => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn labels(scope: &SessionExecutionScope) -> serde_json::Value {
    json!({"app.kubernetes.io/name":"ayni-conversation-session","app.kubernetes.io/part-of":"ayni-orchestrator","build.ayni/session-id":scope.session_id,"build.ayni/session-generation":scope.generation.to_string()})
}

fn verify_labels(
    labels: Option<&std::collections::BTreeMap<String, String>>,
    scope: &SessionExecutionScope,
) -> RuntimeResult {
    let valid = labels.is_some_and(|labels| {
        labels.get("build.ayni/session-id") == Some(&scope.session_id)
            && labels.get("build.ayni/session-generation") == Some(&scope.generation.to_string())
    });
    if !valid {
        return Err(io::Error::other("session resource identity conflict").into());
    }
    Ok(())
}

async fn ensure_pod(
    api: &Api<Pod>,
    name: &str,
    scope: &SessionExecutionScope,
    binding: &ayni_orchestrator_events::v1::ConversationTurnBinding,
    image: &str,
) -> RuntimeResult {
    let anthropic = binding.execution_binding_revision == "anthropic-messages-v1";
    let (enable, endpoint_name, default_endpoint) = if anthropic {
        (
            "ORCHESTRATOR_ANTHROPIC_ENABLED",
            "ANTHROPIC_BASE_URL",
            "https://api.anthropic.com/v1",
        )
    } else {
        (
            "ORCHESTRATOR_OPENAI_ENABLED",
            "OPENAI_BASE_URL",
            "https://api.openai.com/v1",
        )
    };
    let endpoint = (env::var(enable).as_deref() == Ok("true"))
        .then(|| env::var(endpoint_name).unwrap_or_else(|_| default_endpoint.into()));
    let pod = desired_pod(name, scope, binding, image, endpoint.as_deref())?;
    if let Some(existing) = api.get_opt(name).await? {
        verify_labels(existing.metadata.labels.as_ref(), scope)?;
        let phase = existing
            .status
            .as_ref()
            .and_then(|status| status.phase.as_deref());
        if !matches!(phase, Some("Succeeded" | "Failed")) && same_recipe(&existing, &pod) {
            return Ok(());
        }
        // Adopt the same retained generation/PVC. A sending ledger survives this
        // process restart as uncertainty; changing a workload never admits a call.
        let delete = DeleteParams {
            preconditions: Some(kube::api::Preconditions {
                uid: existing.metadata.uid,
                resource_version: None,
            }),
            ..DeleteParams::default()
        };
        api.delete(name, &delete).await?;
        wait_deleted(api, name).await?;
    }
    create_or_race(api.create(&PostParams::default(), &pod).await)
}

fn desired_pod(
    name: &str,
    scope: &SessionExecutionScope,
    binding: &ayni_orchestrator_events::v1::ConversationTurnBinding,
    image: &str,
    provider_endpoint: Option<&str>,
) -> RuntimeResult<Pod> {
    let mut pod: Pod = serde_json::from_value(
        json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"labels":labels(scope)},"spec":{"automountServiceAccountToken":false,"restartPolicy":"Never","terminationGracePeriodSeconds":10,"securityContext":{"runAsNonRoot":true,"runAsUser":10001,"runAsGroup":10001,"fsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},"containers":[{"name":"session","image":image,"imagePullPolicy":"IfNotPresent","args":["session-worker"],"env":[{"name":"ORCHESTRATOR_SESSION_ID","value":scope.session_id},{"name":"ORCHESTRATOR_SESSION_GENERATION","value":scope.generation.to_string()},{"name":"ORCHESTRATOR_SESSION_CONVERSATION_ID","value":binding.conversation_id},{"name":"ORCHESTRATOR_SESSION_WORKSPACE_ID","value":binding.workspace_id},{"name":"ORCHESTRATOR_SESSION_PROJECT_ID","value":binding.project_id},{"name":"ORCHESTRATOR_SESSION_IDLE_SECONDS","value":scope.idle_timeout_seconds.to_string()},{"name":"ORCHESTRATOR_SESSION_TOKEN_PATH","value":"/var/run/session/token"},{"name":"ORCHESTRATOR_SESSION_LEDGER_PATH","value":"/var/lib/ayni-session/results.sqlite"}],"ports":[{"name":"session","containerPort":8081}],"readinessProbe":{"httpGet":{"path":"/health","port":"session"},"periodSeconds":1},"resources":{"requests":{"cpu":"10m","memory":"32Mi"},"limits":{"cpu":"500m","memory":"256Mi"}},"securityContext":{"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}},"volumeMounts":[{"name":"ledger","mountPath":"/var/lib/ayni-session"},{"name":"token","mountPath":"/var/run/session","readOnly":true}]}],"volumes":[{"name":"ledger","persistentVolumeClaim":{"claimName":name}},{"name":"token","secret":{"secretName":name,"items":[{"key":"token","path":"token"}]}}]}}),
    )?;
    if let Some(endpoint) = provider_endpoint {
        let workspace = uuid::Uuid::parse_str(&binding.workspace_id)?;
        let anthropic = binding.execution_binding_revision == "anthropic-messages-v1";
        let provider = if anthropic { "anthropic" } else { "openai" };
        let key_path_name = if anthropic {
            "ORCHESTRATOR_ANTHROPIC_API_KEY_PATH"
        } else {
            "ORCHESTRATOR_OPENAI_API_KEY_PATH"
        };
        let endpoint_name = if anthropic {
            "ANTHROPIC_BASE_URL"
        } else {
            "OPENAI_BASE_URL"
        };
        let spec = pod
            .spec
            .as_mut()
            .ok_or_else(|| io::Error::other("session Pod invalid"))?;
        let container = spec
            .containers
            .first_mut()
            .ok_or_else(|| io::Error::other("session Pod invalid"))?;
        container
            .env
            .get_or_insert_default()
            .extend(serde_json::from_value::<
                Vec<k8s_openapi::api::core::v1::EnvVar>,
            >(json!([
                {"name":key_path_name,"value":"/var/run/provider/api-key"},
                {"name":endpoint_name,"value":endpoint},
                {"name":"ORCHESTRATOR_SESSION_PROVIDER","value":provider}
            ]))?);
        container
            .volume_mounts
            .get_or_insert_default()
            .push(serde_json::from_value(
                json!({"name":"provider","mountPath":"/var/run/provider","readOnly":true}),
            )?);
        spec.volumes.get_or_insert_default().push(serde_json::from_value(json!({"name":"provider","secret":{"secretName":format!("provider-{provider}-{}",workspace.simple()),"items":[{"key":"api-key","path":"api-key"}],"optional":true}}))?);
    }
    if env::var("ORCHESTRATOR_MCP_ENABLED").as_deref() == Ok("true") {
        let workspace = uuid::Uuid::parse_str(&binding.workspace_id)?;
        let spec = pod
            .spec
            .as_mut()
            .ok_or_else(|| io::Error::other("session Pod invalid"))?;
        let container = spec
            .containers
            .first_mut()
            .ok_or_else(|| io::Error::other("session Pod invalid"))?;
        container.env.get_or_insert_default().push(serde_json::from_value(json!({"name":"ORCHESTRATOR_MCP_CONFIG_PATH","value":"/var/run/mcp-config/grants.json"}))?);
        container
            .volume_mounts
            .get_or_insert_default()
            .extend(serde_json::from_value::<
                Vec<k8s_openapi::api::core::v1::VolumeMount>,
            >(json!([
                {"name":"mcp-config","mountPath":"/var/run/mcp-config","readOnly":true},
                {"name":"mcp-credentials","mountPath":"/var/run/mcp","readOnly":true}
            ]))?);
        spec.volumes.get_or_insert_default().extend(serde_json::from_value::<Vec<k8s_openapi::api::core::v1::Volume>>(json!([
            {"name":"mcp-config","configMap":{"name":format!("mcp-config-{}",workspace.simple()),"items":[{"key":"grants.json","path":"grants.json"}]}},
            {"name":"mcp-credentials","secret":{"secretName":format!("mcp-credentials-{}",workspace.simple()),"optional":true}}
        ]))?);
    }
    let mut recipe_spec = pod.spec.clone();
    if let Some(spec) = recipe_spec.as_mut() {
        for container in &mut spec.containers {
            if let Some(env) = container.env.as_mut() {
                env.retain(|entry| entry.name != "ORCHESTRATOR_SESSION_IDLE_SECONDS");
            }
        }
    }
    let recipe = digest(&serde_json::to_vec(&recipe_spec)?);
    pod.metadata
        .annotations
        .get_or_insert_default()
        .insert("build.ayni/session-recipe-digest".into(), recipe);
    Ok(pod)
}

fn same_recipe(existing: &Pod, desired: &Pod) -> bool {
    let recipe = |pod: &Pod| {
        pod.metadata
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.get("build.ayni/session-recipe-digest"))
            .cloned()
    };
    recipe(existing).is_some() && recipe(existing) == recipe(desired)
}

async fn wait_deleted(api: &Api<Pod>, name: &str) -> RuntimeResult {
    for _ in 0..120 {
        if api.get_opt(name).await?.is_none() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(io::Error::other("session restart waiting for retained workload termination").into())
}

async fn wait_address(api: &Api<Pod>, name: &str) -> RuntimeResult<String> {
    for _ in 0..120 {
        let pod = api.get(name).await?;
        if let Some(status) = pod.status
            && status.conditions.as_ref().is_some_and(|conditions| {
                conditions
                    .iter()
                    .any(|condition| condition.type_ == "Ready" && condition.status == "True")
            })
            && let Some(ip) = status.pod_ip
        {
            ip.parse::<std::net::IpAddr>()?;
            return Ok(ip);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(io::Error::other("session provisioning timed out").into())
}

fn routes(configuration: &Configuration) -> RuntimeResult<Connection> {
    let connection = Connection::open(
        configuration
            .ledger_path
            .with_file_name("session-routes.sqlite"),
    )?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS session_routes(command_id TEXT PRIMARY KEY,pod_name TEXT NOT NULL,digest TEXT NOT NULL,command_bytes BLOB NOT NULL,report_id TEXT,decided INTEGER NOT NULL DEFAULT 0);")?;
    Ok(connection)
}

fn retain_route(
    configuration: &Configuration,
    command_id: &str,
    pod_name: &str,
    digest: &str,
    command_bytes: &[u8],
) -> RuntimeResult {
    let mut connection = routes(configuration)?;
    let tx = connection.transaction()?;
    let retained: Option<(String, String)> = tx
        .query_row(
            "SELECT pod_name,digest FROM session_routes WHERE command_id=?1",
            [command_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((retained_name, retained_digest)) = retained {
        if retained_name != pod_name || retained_digest != digest {
            return Err(io::Error::other("session route identity conflict").into());
        }
    } else {
        let (count, bytes): (i64, i64) = tx.query_row(
            "SELECT count(*),COALESCE(sum(length(command_bytes)),0) FROM session_routes",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if count >= 50000 || bytes + i64::try_from(command_bytes.len())? > 256 * 1024 * 1024 {
            return Err(io::Error::other("session route capacity exhausted").into());
        }
        tx.execute("INSERT INTO session_routes(command_id,pod_name,digest,command_bytes) VALUES(?1,?2,?3,?4)",params![command_id,pod_name,digest,command_bytes])?;
    }
    tx.commit()?;
    Ok(())
}

pub fn bind_report(
    configuration: &Configuration,
    command_id: &str,
    report_id: &str,
) -> RuntimeResult {
    routes(configuration)?.execute(
        "UPDATE session_routes SET report_id=?2 WHERE command_id=?1",
        params![command_id, report_id],
    )?;
    Ok(())
}

pub fn mark_decided(configuration: &Configuration, report_id: &str) -> RuntimeResult {
    routes(configuration)?.execute(
        "UPDATE session_routes SET decided=1 WHERE report_id=?1",
        [report_id],
    )?;
    Ok(())
}

pub async fn acknowledge(configuration: &Configuration, report_id: &str) -> RuntimeResult {
    let connection = routes(configuration)?;
    let route: Option<(String, String, Vec<u8>)> = connection
        .query_row(
            "SELECT command_id,pod_name,command_bytes FROM session_routes WHERE report_id=?1",
            [report_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((command_id, name, command_bytes)) = route else {
        return Ok(());
    };
    connection.execute(
        "UPDATE session_routes SET decided=1 WHERE report_id=?1",
        [report_id],
    )?;
    acknowledge_route(&command_id, &name, &command_bytes).await?;
    connection.execute(
        "DELETE FROM session_routes WHERE command_id=?1",
        [command_id],
    )?;
    Ok(())
}

async fn acknowledge_route(command_id: &str, name: &str, command_bytes: &[u8]) -> RuntimeResult {
    let (_, _, command) = decode_command(command_bytes)?;
    let scope = command
        .session
        .ok_or_else(|| io::Error::other("retained session scope missing"))?;
    let binding = command
        .binding
        .ok_or_else(|| io::Error::other("retained session binding missing"))?;
    if self::name(&scope)? != name {
        return Err(io::Error::other("retained session route conflict").into());
    }
    let image = env::var("ORCHESTRATOR_SESSION_IMAGE")
        .map_err(|_| io::Error::other("isolated session image is required"))?;
    let namespace = namespace()?;
    let client = Client::try_default().await?;
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let secrets: Api<Secret> = Api::namespaced(client, &namespace);
    let token = ensure_token(&secrets, name, &scope).await?;
    ensure_pod(&pods, name, &scope, &binding, &image).await?;
    let ip = wait_address(&pods, name).await?;
    reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?
        .post(format!("http://{ip}:8081/v1/acknowledge"))
        .bearer_auth(token)
        .body(command_id.to_owned())
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

pub async fn cleanup(configuration: &Configuration) -> RuntimeResult {
    let connection = routes(configuration)?;
    let pending = {
        let mut statement = connection
            .prepare("SELECT command_id,pod_name,command_bytes FROM session_routes WHERE decided=1 LIMIT 100")?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    for (command_id, name, command_bytes) in pending {
        if acknowledge_route(&command_id, &name, &command_bytes)
            .await
            .is_ok()
        {
            connection.execute(
                "DELETE FROM session_routes WHERE command_id=?1",
                [&command_id],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration(directory: &std::path::Path) -> Configuration {
        Configuration {
            broker_url: "nats://unused:4222".into(),
            username: "unused".into(),
            password: String::new(),
            instance_id: uuid::Uuid::now_v7().to_string(),
            seed: [0; 32],
            ledger_path: directory.join("ledger.sqlite"),
        }
    }

    #[test]
    fn application_decision_retains_cleanup_obligation_across_supervisor_restart() -> RuntimeResult
    {
        let directory = tempfile::tempdir()?;
        let configuration = configuration(directory.path());
        let command_id = uuid::Uuid::now_v7().to_string();
        let report_id = uuid::Uuid::now_v7().to_string();
        let exact = b"retained app-owned exact command";
        let digest = digest(exact);
        retain_route(&configuration, &command_id, "session-test", &digest, exact)?;
        bind_report(&configuration, &command_id, &report_id)?;
        // This durable mark precedes the main ledger's ACK-floor reconciliation.
        // A crash can therefore never strand the session result after broker ACK.
        mark_decided(&configuration, &report_id)?;
        let reopened = routes(&configuration)?;
        let (decided, retained): (bool, Vec<u8>) = reopened.query_row(
            "SELECT decided,command_bytes FROM session_routes WHERE command_id=?1",
            [&command_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert!(decided);
        assert_eq!(retained, exact);
        assert!(
            retain_route(&configuration, &command_id, "other-session", &digest, exact).is_err()
        );
        Ok(())
    }

    #[test]
    fn retired_pod_deletion_requires_scope_labels_and_uid() -> RuntimeResult {
        let scope = SessionExecutionScope {
            session_id: uuid::Uuid::now_v7().to_string(),
            generation: 1,
            idle_timeout_seconds: 30,
        };
        let binding = ayni_orchestrator_events::v1::ConversationTurnBinding {
            workspace_id: "workspace".into(),
            project_id: "project".into(),
            conversation_id: "conversation".into(),
            ..Default::default()
        };
        let mut pod = desired_pod(&name(&scope)?, &scope, &binding, "fixture-image", None)?;
        assert!(retirement_preconditions(&pod, &scope, &binding).is_err());
        pod.metadata.uid = Some("retained-pod-uid".into());
        let delete = retirement_preconditions(&pod, &scope, &binding)?;
        assert_eq!(
            delete
                .preconditions
                .and_then(|preconditions| preconditions.uid)
                .as_deref(),
            Some("retained-pod-uid")
        );
        let other = ayni_orchestrator_events::v1::ConversationTurnBinding {
            conversation_id: "other".into(),
            ..binding.clone()
        };
        assert!(retirement_preconditions(&pod, &scope, &other).is_err());
        let other_scope = SessionExecutionScope {
            generation: 2,
            ..scope.clone()
        };
        assert!(retirement_preconditions(&pod, &other_scope, &binding).is_err());
        pod.metadata.labels = None;
        assert!(retirement_preconditions(&pod, &scope, &binding).is_err());
        Ok(())
    }

    #[test]
    fn session_resource_name_remains_within_kubernetes_limit() -> RuntimeResult {
        let scope = SessionExecutionScope {
            session_id: uuid::Uuid::now_v7().to_string(),
            generation: u64::MAX,
            idle_timeout_seconds: 30,
        };
        assert!(name(&scope)?.len() <= 63);
        Ok(())
    }
}

#[derive(serde::Deserialize)]
struct Authority {
    #[serde(default)]
    records: Vec<ayni_orchestrator_events::v1::ConversationContextRecord>,
    #[serde(default)]
    next_after_ordinal: u64,
    #[serde(default)]
    allowed: bool,
    #[serde(default)]
    cancellation_requested: bool,
    #[serde(default)]
    state: String,
}

struct ProviderRuntime {
    enabled: bool,
    authority_url: String,
    authority_token: String,
}

impl ProviderRuntime {
    fn from_environment(provider: &str) -> RuntimeResult<Self> {
        let path = env::var("ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH")
            .map_err(|_| io::Error::other("session authority unavailable"))?;
        let authority_token = std::fs::read_to_string(path)
            .map_err(|_| io::Error::other("session authority unavailable"))?;
        let authority_url = env::var("ORCHESTRATOR_SESSION_AUTHORITY_URL")
            .map_err(|_| io::Error::other("session authority unavailable"))?;
        Ok(Self {
            enabled: env::var(if provider == "anthropic" {
                "ORCHESTRATOR_ANTHROPIC_ENABLED"
            } else {
                "ORCHESTRATOR_OPENAI_ENABLED"
            })
            .as_deref()
                == Ok("true"),
            authority_url,
            authority_token,
        })
    }
}

async fn authority(
    http: &reqwest::Client,
    body: serde_json::Value,
    runtime: &ProviderRuntime,
) -> RuntimeResult<Authority> {
    let response = http
        .post(&runtime.authority_url)
        .bearer_auth(runtime.authority_token.trim())
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    bounded_json(response, 256 * 1024).await
}

async fn bounded_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
    maximum: usize,
) -> RuntimeResult<T> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > maximum {
            return Err(io::Error::other("session response bound exceeded").into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}

async fn execute_provider(
    http: &reqwest::Client,
    address: &str,
    token: &str,
    command_bytes: &[u8],
) -> RuntimeResult<ExecutionResult> {
    let (_, _, command) = decode_command(command_bytes)?;
    let provider = command
        .frozen_configuration
        .as_ref()
        .map_or("", |c| c.provider.as_str());
    let runtime = ProviderRuntime::from_environment(provider)?;
    execute_provider_with_runtime(
        http,
        &format!("http://{address}:8081"),
        token,
        command_bytes,
        &runtime,
    )
    .await
}

async fn execute_provider_with_runtime(
    http: &reqwest::Client,
    worker_url: &str,
    token: &str,
    command_bytes: &[u8],
    runtime: &ProviderRuntime,
) -> RuntimeResult<ExecutionResult> {
    use crate::session_worker::provider_session::{
        CancelInput, Preparation, PrepareInput, SendInput,
    };
    let (identity, _, command) = decode_command(command_bytes)?;
    let command_digest = digest(command_bytes);
    let failure = |reason: &str| ExecutionResult {
        command_id: identity.id.clone(),
        command_digest: command_digest.clone(),
        response_content: String::new(),
        failure_reason: reason.into(),
        provider_call: None,
        plan_proposal: None,
    };
    let mut records = Vec::new();
    let mut after = 0;
    loop {
        let page = authority(
            http,
            json!({"command_bytes":command_bytes,"action":"context","after_ordinal":after}),
            runtime,
        )
        .await?;
        if records.len() + page.records.len() > 10000 {
            return Ok(failure("provider_input_limit"));
        }
        records.extend(page.records);
        let bytes = records.iter().try_fold(0_usize, |sum, record| {
            sum.checked_add(record.content.len() + 128)
        });
        let input_bound = command
            .frozen_configuration
            .as_ref()
            .map_or(0, |config| config.max_input_tokens);
        if bytes.is_none_or(|bytes| u64::try_from(bytes).unwrap_or(u64::MAX) > input_bound) {
            return Ok(failure("provider_input_limit"));
        }
        if page.next_after_ordinal == 0 {
            break;
        }
        if page.next_after_ordinal <= after {
            return Err(io::Error::other("context cursor invalid").into());
        }
        after = page.next_after_ordinal;
    }
    let prepared = http
        .post(format!("{worker_url}/v1/provider-prepare"))
        .bearer_auth(token)
        .json(&PrepareInput {
            command_bytes: command_bytes.to_vec(),
            context: records,
        })
        .send()
        .await?;
    if prepared.status() == reqwest::StatusCode::FAILED_DEPENDENCY {
        return Ok(failure("provider_credentials_unavailable"));
    }
    if matches!(
        prepared.status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY | reqwest::StatusCode::PAYLOAD_TOO_LARGE
    ) {
        return Ok(failure("provider_input_limit"));
    }
    let prepared: Preparation = bounded_json(prepared.error_for_status()?, 256 * 1024).await?;
    if let Some(result) = prepared.result {
        return validate_result(result, &identity.id, &command_digest);
    }
    let disabled = !runtime.enabled;
    let permit = provider_permit(http, command_bytes, &prepared, runtime).await?;
    let disabled_without_call = disabled && permit.state == "cancelled";
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(150))
        .retry(reqwest::retry::never())
        .build()?;
    let send = client
        .post(format!("{worker_url}/v1/provider-send"))
        .bearer_auth(token)
        .json(&SendInput {
            command_id: identity.id.clone(),
            request_digest: prepared.request_digest,
            allowed: permit.allowed,
            state: permit.state,
        })
        .send();
    tokio::pin!(send);
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    let mut stopped = false;
    loop {
        tokio::select! {
            response=&mut send=>{let response:ExecutionResult=bounded_json(response?.error_for_status()?,256*1024).await?;if disabled_without_call {return Ok(failure("provider_credentials_unavailable"));} return validate_result(response,&identity.id,&command_digest);}
            _=interval.tick(),if !stopped=>{
                let cancellation=authority(http,json!({"command_bytes":command_bytes,"action":"poll"}),runtime).await;
                if cancellation.as_ref().map_or(true, |state|state.cancellation_requested) {
                    http.post(format!("{worker_url}/v1/cancel")).bearer_auth(token).json(&CancelInput {command_id:identity.id.clone(),command_digest:command_digest.clone()}).send().await?.error_for_status()?;
                    stopped=true;
                }
            }
        }
    }
}

async fn provider_permit(
    http: &reqwest::Client,
    command_bytes: &[u8],
    prepared: &crate::session_worker::provider_session::Preparation,
    runtime: &ProviderRuntime,
) -> RuntimeResult<Authority> {
    if runtime.enabled {
        return authority(http,json!({"command_bytes":command_bytes,"action":"admit","call_id":prepared.call_id,"request_digest":prepared.request_digest,"context_digest":prepared.context_digest,"reserved_input_tokens":prepared.reserved_input_tokens,"reserved_output_tokens":prepared.reserved_output_tokens,"reserved_total_tokens":prepared.reserved_total_tokens}),runtime).await;
    }
    let mut prior = authority(
        http,
        json!({"command_bytes":command_bytes,"action":"poll"}),
        runtime,
    )
    .await?;
    prior.allowed = false;
    if !matches!(prior.state.as_str(), "sending" | "uncertain") {
        prior.state = "cancelled".into();
    }
    Ok(prior)
}

fn validate_result(
    result: ExecutionResult,
    command_id: &str,
    command_digest: &str,
) -> RuntimeResult<ExecutionResult> {
    if result.command_id != command_id
        || result.command_digest != command_digest
        || result.response_content.len() > 16384
        || result.plan_proposal.as_ref().is_some_and(|proposal| {
            !result.failure_reason.is_empty()
                || result
                    .provider_call
                    .as_ref()
                    .is_none_or(|call| call.state != "completed")
                || ayni_orchestrator_events::plan_proposals::validate_proposal(
                    &proposal.proposal_json,
                )
                .is_err()
        })
    {
        return Err(io::Error::other("session result binding mismatch").into());
    }
    Ok(result)
}

#[cfg(test)]
#[path = "session_supervisor_tests.rs"]
mod provider_recovery_tests;
