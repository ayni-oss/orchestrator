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

pub async fn execute(configuration: &Configuration, command_bytes: &[u8]) -> RuntimeResult<String> {
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
    Ok(response.response_content)
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
    if let Some(existing) = api.get_opt(name).await? {
        verify_labels(existing.metadata.labels.as_ref(), scope)?;
        let phase = existing
            .status
            .as_ref()
            .and_then(|status| status.phase.as_deref());
        if !matches!(phase, Some("Succeeded" | "Failed")) {
            return Ok(());
        }
        let delete = DeleteParams {
            preconditions: Some(kube::api::Preconditions {
                uid: existing.metadata.uid,
                resource_version: None,
            }),
            ..DeleteParams::default()
        };
        api.delete(name, &delete).await?;
        for _ in 0..60 {
            if api.get_opt(name).await?.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    let pod: Pod = serde_json::from_value(
        json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"labels":labels(scope)},"spec":{"automountServiceAccountToken":false,"restartPolicy":"Never","terminationGracePeriodSeconds":10,"securityContext":{"runAsNonRoot":true,"runAsUser":10001,"runAsGroup":10001,"fsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},"containers":[{"name":"session","image":image,"imagePullPolicy":"IfNotPresent","args":["session-worker"],"env":[{"name":"ORCHESTRATOR_SESSION_ID","value":scope.session_id},{"name":"ORCHESTRATOR_SESSION_GENERATION","value":scope.generation.to_string()},{"name":"ORCHESTRATOR_SESSION_CONVERSATION_ID","value":binding.conversation_id},{"name":"ORCHESTRATOR_SESSION_WORKSPACE_ID","value":binding.workspace_id},{"name":"ORCHESTRATOR_SESSION_PROJECT_ID","value":binding.project_id},{"name":"ORCHESTRATOR_SESSION_IDLE_SECONDS","value":scope.idle_timeout_seconds.to_string()},{"name":"ORCHESTRATOR_SESSION_TOKEN_PATH","value":"/var/run/session/token"},{"name":"ORCHESTRATOR_SESSION_LEDGER_PATH","value":"/var/lib/ayni-session/results.sqlite"}],"ports":[{"name":"session","containerPort":8081}],"readinessProbe":{"httpGet":{"path":"/health","port":"session"},"periodSeconds":1},"resources":{"requests":{"cpu":"10m","memory":"32Mi"},"limits":{"cpu":"500m","memory":"256Mi"}},"securityContext":{"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}},"volumeMounts":[{"name":"ledger","mountPath":"/var/lib/ayni-session"},{"name":"token","mountPath":"/var/run/session","readOnly":true}]}],"volumes":[{"name":"ledger","persistentVolumeClaim":{"claimName":name}},{"name":"token","secret":{"secretName":name,"items":[{"key":"token","path":"token"}]}}]}}),
    )?;
    create_or_race(api.create(&PostParams::default(), &pod).await)
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
