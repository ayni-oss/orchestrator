//! Scoped `PostgreSQL` credential delivery and ownership-checked derived copies.
use super::{
    ProviderRuntime, RuntimeResult, SessionExecutionScope, create_or_race, ensure_token, labels,
    verify_labels, wait_address, wait_deleted,
};
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::{
    Api,
    api::{DeleteParams, PostParams},
};
use serde_json::json;
use std::{io, time::Duration};
pub(super) async fn ensure_provider_credential(
    api: &Api<Secret>,
    name: &str,
    scope: &SessionExecutionScope,
    config: &ayni_orchestrator_events::v1::AgentExecutionConfiguration,
    command_bytes: &[u8],
) -> RuntimeResult {
    let secret_name = name.replacen("session-", "provider-", 1);
    let verify = |secret: &Secret| -> RuntimeResult {
        verify_labels(secret.metadata.labels.as_ref(), scope)?;
        if secret.immutable != Some(true)
            || secret
                .metadata
                .annotations
                .as_ref()
                .and_then(|a| a.get("build.ayni/credential-revision"))
                != Some(&config.credential_revision)
        {
            return Err(io::Error::other("provider credential ownership mismatch").into());
        }
        Ok(())
    };
    if let Some(existing) = api.get_opt(&secret_name).await? {
        return verify(&existing);
    }
    let runtime = ProviderRuntime::from_environment("openai")?;
    let url = runtime
        .authority_url
        .strip_suffix("/v1/session-authority")
        .ok_or_else(|| io::Error::other("credential authority unavailable"))?;
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .post(format!("{url}/v1/session-credential"))
        .bearer_auth(runtime.authority_token.trim())
        .json(&json!({"command_bytes":command_bytes,"action":"credential"}))
        .send()
        .await?
        .error_for_status()?;
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 4096 {
            return Err(io::Error::other("credential delivery invalid").into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value =
        String::from_utf8(bytes).map_err(|_| io::Error::other("credential delivery invalid"))?;
    if value.is_empty() {
        return Err(io::Error::other("credential delivery unavailable").into());
    }
    let secret: Secret = serde_json::from_value(
        json!({"apiVersion":"v1","kind":"Secret","metadata":{"name":secret_name,"labels":labels(scope),"annotations":{"build.ayni/credential-revision":config.credential_revision}},"immutable":true,"stringData":{"api-key":value}}),
    )?;
    create_or_race(api.create(&PostParams::default(), &secret).await)?;
    verify(&api.get(&secret_name).await?)
}

pub(super) async fn acknowledge_configured_route(
    pods: &Api<Pod>,
    secrets: &Api<Secret>,
    name: &str,
    scope: &SessionExecutionScope,
    config: &ayni_orchestrator_events::v1::AgentExecutionConfiguration,
    command_id: &str,
) -> RuntimeResult {
    if let Some(pod) = pods.get_opt(name).await? {
        verify_labels(pod.metadata.labels.as_ref(), scope)?;
        let token = ensure_token(secrets, name, scope).await?;
        let ip = wait_address(pods, name).await?;
        reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()?
            .post(format!("http://{ip}:8081/v1/acknowledge"))
            .bearer_auth(token)
            .body(command_id.to_owned())
            .send()
            .await?
            .error_for_status()?;
        pods.delete(
            name,
            &DeleteParams {
                preconditions: Some(kube::api::Preconditions {
                    uid: pod.metadata.uid,
                    resource_version: None,
                }),
                ..Default::default()
            },
        )
        .await?;
        wait_deleted(pods, name).await?;
    }
    let secret_name = name.replacen("session-", "provider-", 1);
    if let Some(secret) = secrets.get_opt(&secret_name).await? {
        verify_labels(secret.metadata.labels.as_ref(), scope)?;
        if secret
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get("build.ayni/credential-revision"))
            != Some(&config.credential_revision)
        {
            return Err(io::Error::other("provider credential cleanup identity conflict").into());
        }
        secrets
            .delete(
                &secret_name,
                &DeleteParams {
                    preconditions: Some(kube::api::Preconditions {
                        uid: secret.metadata.uid,
                        resource_version: None,
                    }),
                    ..Default::default()
                },
            )
            .await?;
    }
    Ok(())
}
