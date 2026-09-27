use super::{Application, ErrorReason, PageRequest};
use crate::{
    AppState,
    development::{AgentProfile, DevelopmentInput, RepositoryBinding},
    preparation::PrepareRepositoryInput,
};
use ayni_orchestrator_domain::{ChangeConstraints, IdempotencyKey, LifecycleError};
use tempfile::tempdir;

#[test]
fn malformed_idempotency_keys_are_safe_invalid_input() {
    let secret_like = "Bearer secret value".to_owned();
    let Err(error) = super::parse_key(secret_like.clone()) else {
        panic!("an unsafe idempotency key was accepted");
    };
    assert_eq!(error.reason, ErrorReason::InvalidInput);
    assert!(!error.message.contains(&secret_like));
}

#[test]
fn corrupt_durable_state_is_internal() {
    let error = super::ApplicationError::from_store(crate::StoreError::CorruptState(
        "invalid persisted fixture",
    ));
    assert_eq!(error.reason, ErrorReason::Internal);
    assert!(!error.message.contains("fixture"));

    let secret_like = "secret-retained-task-id";
    let error = super::ApplicationError::from_store(crate::StoreError::InvalidLifecycleData(
        LifecycleError::InvalidTaskId(secret_like.to_owned()),
    ));
    assert_eq!(error.reason, ErrorReason::Internal);
    assert!(!error.message.contains(secret_like));
}

#[test]
fn application_errors_are_bounded_by_utf8_bytes() {
    let error = super::ApplicationError::new(ErrorReason::InvalidInput, "é".repeat(300));
    assert_eq!(error.message.len(), 512);
    assert!(error.message.is_char_boundary(error.message.len()));
}

fn repository(id: &str) -> RepositoryBinding {
    RepositoryBinding {
        id: id.to_owned(),
        owner: "ayni-oss".to_owned(),
        repository: "sandbox".to_owned(),
        installation_id: 42,
        base_branch: "main".to_owned(),
        lock_sha256: "a".repeat(64),
    }
}

#[test]
fn page_tokens_retain_membership_across_writes_and_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let database = directory.path().join("orchestrator.sqlite");
    let application = Application::new(AppState::open(&database, 3)?);
    application.save_repository("save-a".to_owned(), &repository("a"))?;
    application.save_repository("save-c".to_owned(), &repository("c"))?;

    let first = application.list_repositories(&PageRequest {
        page_size: 1,
        page_token: String::new(),
    })?;
    assert_eq!(first.items[0].binding.id, "a");
    assert!(!first.next_page_token.is_empty());
    application.save_repository("save-b".to_owned(), &repository("b"))?;
    let mut changed_c = repository("c");
    changed_c.base_branch = "release".to_owned();
    application.replace_repository(&changed_c)?;
    drop(application);

    let reopened = Application::new(AppState::open(&database, 3)?);
    let second = reopened.list_repositories(&PageRequest {
        page_size: 1,
        page_token: first.next_page_token.clone(),
    })?;
    assert_eq!(second.items[0].binding.id, "c");
    assert_eq!(second.items[0].binding.base_branch, "main");
    assert!(second.next_page_token.is_empty());

    let Err(error) = reopened.list_preparations(
        "a",
        &PageRequest {
            page_size: 1,
            page_token: first.next_page_token,
        },
    ) else {
        return Err("a repository-list token worked for preparations".into());
    };
    assert_eq!(error.reason, ErrorReason::InvalidInput);
    Ok(())
}

#[test]
fn legacy_preparation_action_replays_after_method_scoping() -> Result<(), Box<dyn std::error::Error>>
{
    let state = AppState::open(":memory:", 3)?;
    let raw_key = IdempotencyKey::try_from("legacy-prepare".to_owned())?;
    let original = {
        let mut store = state.store.lock().map_err(|_| "store lock was poisoned")?;
        store.put_repository_binding(&repository("trial"))?;
        store.prepare_repository(
            "trial",
            &raw_key,
            &PrepareRepositoryInput {
                revision: "b".repeat(40),
            },
        )?
    };
    let replay = Application::new(state).prepare_repository(
        raw_key.as_str().to_owned(),
        "trial",
        "b".repeat(40),
    )?;
    assert_eq!(replay, original);
    Ok(())
}

#[test]
fn legacy_same_method_key_changed_target_is_a_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState::open(":memory:", 3)?;
    let (first_id, second_id) = {
        let mut store = state.store.lock().map_err(|_| "store lock was poisoned")?;
        store.put_repository_binding(&repository("trial"))?;
        store.put_agent_profile(&AgentProfile {
            id: "legacy-profile".to_owned(),
            provider: "legacy-provider".to_owned(),
            model: "legacy-model".to_owned(),
            secret_ref: "legacy-secret".to_owned(),
        })?;
        let first_input = DevelopmentInput {
            repository_id: "trial".to_owned(),
            profile_id: "legacy-profile".to_owned(),
            start_revision: "b".repeat(40),
            request: "first request".to_owned(),
            acceptance_criteria: vec!["first result".to_owned()],
            change_constraints: ChangeConstraints::default(),
            deadline_seconds: None,
        };
        let second_input = DevelopmentInput {
            request: "second request".to_owned(),
            ..first_input.clone()
        };
        let first = store.submit_development(
            &IdempotencyKey::try_from("legacy-first-submit".to_owned())?,
            &first_input,
        )?;
        let second = store.submit_development(
            &IdempotencyKey::try_from("legacy-second-submit".to_owned())?,
            &second_input,
        )?;
        store.cancel_development(
            first.id,
            &IdempotencyKey::try_from("legacy-cancel".to_owned())?,
        )?;
        let ids = (first.id, second.id);
        drop(store);
        ids
    };

    let application = Application::new(state);
    let Err(error) =
        application.cancel_development_task("legacy-cancel".to_owned(), &second_id.to_string())
    else {
        return Err("a legacy cancel key was reused for another task".into());
    };
    assert_eq!(error.reason, ErrorReason::IdempotencyKeyReused);
    assert_ne!(first_id, second_id);
    Ok(())
}

#[test]
fn migration_repository_replace_can_restore_an_earlier_binding()
-> Result<(), Box<dyn std::error::Error>> {
    let application = Application::new(AppState::open(":memory:", 3)?);
    let original = repository("trial");
    application.replace_repository(&original)?;
    let mut changed = original.clone();
    changed.base_branch = "release".to_owned();
    application.replace_repository(&changed)?;
    application.replace_repository(&original)?;
    assert_eq!(
        application.get_repository("trial")?.binding.base_branch,
        "main"
    );
    Ok(())
}

#[tokio::test]
async fn provider_page_tokens_retain_values_across_restart_without_provider_access()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let database = directory.path().join("orchestrator.sqlite");
    let models = vec![
        crate::AvailableModel {
            id: "gpt-5-a".to_owned(),
            name: Some("Original A".to_owned()),
            owned_by: Some("openai".to_owned()),
            created_at: Some(1),
            context_length: Some(1_024),
            max_output_tokens: Some(128),
        },
        crate::AvailableModel {
            id: "gpt-5-b".to_owned(),
            name: Some("Original B".to_owned()),
            owned_by: Some("openai".to_owned()),
            created_at: Some(2),
            context_length: Some(2_048),
            max_output_tokens: Some(256),
        },
    ];
    let application = Application::new(
        AppState::open(&database, 3)?.with_model_catalog(crate::ModelCatalog::fixed(models)),
    );
    let (first_observation, first) = application
        .list_provider_models(&PageRequest {
            page_size: 1,
            page_token: String::new(),
        })
        .await?;
    assert_eq!(first.items[0].id, "gpt-5-a");
    assert!(!first.next_page_token.is_empty());
    drop(application);

    // A durable token must not refetch the provider after process restart.
    let reopened = Application::new(AppState::open(&database, 3)?);
    let (second_observation, second) = reopened
        .list_provider_models(&PageRequest {
            page_size: 1,
            page_token: first.next_page_token,
        })
        .await?;
    assert_eq!(second.items[0].id, "gpt-5-b");
    assert_eq!(second.items[0].name.as_deref(), Some("Original B"));
    assert_eq!(
        second_observation.observed_at,
        first_observation.observed_at
    );
    assert!(second.next_page_token.is_empty());
    Ok(())
}

#[test]
fn repository_save_is_durably_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let database = directory.path().join("orchestrator.sqlite");
    let application = Application::new(AppState::open(&database, 3)?);
    let original = repository("trial");
    let first = application.save_repository("repository-save".to_owned(), &original)?;
    let replay = application.save_repository("repository-save".to_owned(), &original)?;
    assert_eq!(first, replay);

    let mut changed = original;
    changed.base_branch = "release".to_owned();
    let Err(error) = application.save_repository("repository-save".to_owned(), &changed) else {
        return Err("changed request content reused an idempotency key".into());
    };
    assert_eq!(error.reason, ErrorReason::IdempotencyKeyReused);
    Ok(())
}
