//! Explicit local acceptance fixture seeding. Packaged only in the development image.
use ayni_orchestrator_control_plane::Configuration;
use ayni_orchestrator_workflow_postgres::{
    WorkflowStore,
    store::{
        connections::{ProviderSettings, SaveGithub, SaveProvider},
        repositories::{Connect, ObservedRepository},
        runtime_signers::SaveSigner,
    },
};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    if std::env::args().nth(1).as_deref() != Some("--canonical-local-acceptance") {
        return Err("explicit canonical local acceptance flag required".into());
    }
    let configuration = Configuration::from_environment()?;
    let store = WorkflowStore::new(configuration.runtime_database_url()?);
    let mut identities = connections(&store).await?;
    identities.extend(repositories(&store).await?);
    println!("{}", serde_json::to_string(&identities)?);
    Ok(())
}
async fn connections(
    store: &WorkflowStore,
) -> Result<std::collections::BTreeMap<String, String>, Box<dyn Error>> {
    let fixture = store
        .save_provider_connection(
            ayni_orchestrator_workflow_postgres::store::connections::SaveProvider {
                key: &uuid::Uuid::now_v7().to_string(),
                id: "",
                expected: 0,
                name: "Browser OpenAI fixture",
                provider: "openai",
                capability: "llm",
                settings: ayni_orchestrator_workflow_postgres::store::connections::ProviderSettings::default(),
                credential: "browser-only-synthetic-key",
            },
        )
        .await?;
    store
        .finish_provider_validation(&fixture.id, "", &["fixture-model".into()])
        .await?;
    let github = store
        .save_github_connection(SaveGithub {
            key: &uuid::Uuid::now_v7().to_string(),
            id: "",
            expected: 0,
            name: "Browser GitHub revalidation fixture",
            app_id: "123",
            private_key_pem: "synthetic-invalid-key",
        })
        .await?;
    // Admit the fixture locally so the browser can verify the active-key check
    // rejects the synthetic key without changing its configured revision.
    store.finish_github_validation(&github.id, "").await?;
    Ok(std::collections::BTreeMap::from([
        ("AYNI_BROWSER_CONNECTION_ID".into(), fixture.connection_id),
        (
            "AYNI_BROWSER_GITHUB_CONNECTION_ID".into(),
            github.connection_id,
        ),
    ]))
}
async fn repositories(
    store: &WorkflowStore,
) -> Result<std::collections::BTreeMap<String, String>, Box<dyn Error>> {
    let key = || uuid::Uuid::now_v7().to_string();
    let source = store
        .save_github_connection(SaveGithub {
            key: &key(),
            id: "",
            expected: 0,
            name: "Repository browser fixture",
            app_id: "123",
            private_key_pem: "synthetic-repository-browser-key",
        })
        .await?;
    store.finish_github_validation(&source.id, "").await?;
    let command = key();
    let provider_id = u64::try_from(uuid::Uuid::new_v4().as_u128() % 1_000_000_000_000)? + 1;
    store
        .begin_repository_connect(Connect {
            key: &command,
            connection_id: &source.connection_id,
            expected_revision: 1,
            installation_id: 123,
            provider_repository_id: provider_id,
        })
        .await?;
    let repo = store
        .finish_repository_connect(
            &command,
            Ok(ObservedRepository {
                provider_repository_id: provider_id,
                full_name: "fixture/browser-repository".into(),
                default_branch: "main".into(),
                private: true,
            }),
        )
        .await?
        .repository
        .ok_or("fixture repository missing")?;
    seed_static_inspection(store, &repo.id).await?;
    let a = store
        .create_workspace(&key(), "Repository browser A")
        .await?
        .resource;
    let b = store
        .create_workspace(&key(), "Repository browser B")
        .await?
        .resource;
    let pa = store
        .create_project(&key(), &a.id, "Source A", "Use a shared catalog")
        .await?
        .resource;
    let pb = store
        .create_project(&key(), &b.id, "Source B", "Reuse a shared catalog")
        .await?
        .resource;
    let (runtime_registry, runtime_signer) = seed_runtime_configuration(store).await?;
    Ok(std::collections::BTreeMap::from([
        ("AYNI_REPOSITORY_FIXTURE".into(), repo.id),
        ("AYNI_REPOSITORY_REGISTRY".into(), runtime_registry),
        ("AYNI_REPOSITORY_SIGNER".into(), runtime_signer),
        ("AYNI_REPOSITORY_SOURCE".into(), source.connection_id),
        ("AYNI_REPOSITORY_WORKSPACE_A".into(), a.id),
        ("AYNI_REPOSITORY_WORKSPACE_B".into(), b.id),
        ("AYNI_REPOSITORY_PROJECT_A".into(), pa.id),
        ("AYNI_REPOSITORY_PROJECT_B".into(), pb.id),
    ]))
}
async fn seed_static_inspection(
    store: &WorkflowStore,
    repository_id: &str,
) -> Result<(), Box<dyn Error>> {
    let inspection_key = uuid::Uuid::now_v7().to_string();
    store
        .begin_repository_inspection(
            ayni_orchestrator_workflow_postgres::store::repository_inspections::Inspect {
                key: &inspection_key,
                repository_id,
                expected_connection_revision: 1,
                commit_sha: &"a".repeat(40),
            },
        )
        .await?;
    store.finish_repository_inspection(&inspection_key, Ok(
        ayni_orchestrator_workflow_postgres::store::repository_inspections::InspectionResult {
            policy_digest: "sha256:fixture-policy".into(), lock_digest: "sha256:fixture-lock".into(),
            lock_fingerprint: String::new(), ayni_version: String::new(), lock_schema: String::new(),
            tools: vec![], trust_key_ids: vec![], trusted_public_keys: std::collections::BTreeMap::new(), findings: vec![
                ayni_orchestrator_workflow_postgres::store::repository_inspections::Finding {
                    code: "policy_missing".into(), path: ".ayni.toml".into(),
                    action: "Commit an Ayni policy before preparing this repository.".into(),
                }
            ],
        }
    )).await?;
    Ok(())
}

async fn seed_runtime_configuration(
    store: &WorkflowStore,
) -> Result<(String, String), Box<dyn Error>> {
    let registry = store
        .save_provider_connection(SaveProvider {
            key: &uuid::Uuid::now_v7().to_string(),
            id: "",
            expected: 0,
            name: "Repository runtime fixture registry",
            provider: "github",
            capability: "container_registry",
            settings: ProviderSettings {
                registry_destination: "ghcr.io/fixture".into(),
                push_username: "fixture".into(),
                pull_username: "fixture".into(),
            },
            credential: &serde_json::to_string(&["synthetic-push", "synthetic-pull"])?,
        })
        .await?;
    store
        .finish_provider_validation(&registry.id, "", &[])
        .await?;
    let signer = store
        .save_runtime_signer(SaveSigner {
            key: &uuid::Uuid::now_v7().to_string(),
            id: "",
            expected_revision: 0,
            display_name: "Repository runtime fixture signer",
            key_id: "fixture",
            public_key: &"e".repeat(64),
            secret_name: "ayni-runtime-signer-browser-fixture",
            secret_uid: &uuid::Uuid::new_v4().to_string(),
        })
        .await?;
    Ok((registry.connection_id, signer.id))
}
