use ayni_orchestrator_events::{
    EventIdentity,
    v1::{
        ConversationTurnBinding, ConversationTurnReport, ConversationTurnReportKind,
        ProviderCallEvidence,
    },
    workflow::{
        WorkflowEventMetadata, decode_command, encode_report, sign_report, signing_key_digest,
    },
};
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, store::provider_calls::AuthorityRequest,
};
use sha2::{Digest as _, Sha256};
use std::error::Error;
type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

fn key() -> String {
    uuid::Uuid::now_v7().to_string()
}
struct Fixture {
    store: WorkflowStore,
    workspace: String,
    project: String,
    conversation: String,
    client: tokio_postgres::Client,
    driver: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
        ayni_orchestrator_workflow_postgres::migrate_for_runtime(
            &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
            &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
        )
        .await?;
        let store = WorkflowStore::new(url.clone());
        let workspace = store
            .create_workspace(&key(), "Provider authority")
            .await?
            .resource
            .id;
        let current = store.get_agent_configuration(&workspace, "").await?;
        let config = ayni_orchestrator_workflow_postgres::model::AgentConfiguration {
            revision_id: String::new(),
            provider: "openai".into(),
            model: "explicit-fixture-model".into(),
            ..current.clone()
        };
        store
            .update_agent_configuration(&key(), &current.revision_id, &config)
            .await?;
        let project = store
            .create_project(&key(), &workspace, "Provider", "paid-call safety")
            .await?
            .resource
            .id;
        let conversation = store
            .create_conversation(&key(), &workspace, &project, "Provider")
            .await?
            .resource
            .id;
        let (client, driver) = ayni_orchestrator_workflow_postgres::connect(&url).await?;
        Ok(Self {
            store,
            workspace,
            project,
            conversation,
            client,
            driver,
        })
    }
    async fn send(&self) -> Result<String, Box<dyn Error + Send + Sync>> {
        Ok(self
            .store
            .send_message(
                &key(),
                &self.workspace,
                &self.project,
                &self.conversation,
                "bounded input",
            )
            .await?
            .turn_id)
    }
    async fn claim(&self, turn: &str) -> Result<Call, Box<dyn Error + Send + Sync>> {
        let bytes: Vec<u8> = self
            .client
            .query_one(
                "SELECT exact_command_bytes FROM workflow_dispatch_intents WHERE turn_id=$1",
                &[&turn],
            )
            .await?
            .get(0);
        let (identity, metadata, command) = decode_command(&bytes)?;
        self.store.mark_outbox_stored(&identity.id, &bytes).await?;
        let mut binding = command.binding.ok_or("binding")?;
        assert_eq!(binding.execution_binding_revision, "openai-responses-v1");
        binding.command_digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        let seed = [91; 32];
        let public = ed25519_dalek::SigningKey::from_bytes(&seed)
            .verifying_key()
            .to_bytes();
        let executor = key();
        let mut report = ConversationTurnReport {
            binding: Some(binding.clone()),
            kind: ConversationTurnReportKind::Started.into(),
            report_sequence: 1,
            executor_instance_id: executor.clone(),
            signing_public_key: public.to_vec(),
            signing_key_digest: signing_key_digest(&public),
            ..Default::default()
        };
        let claim_identity = EventIdentity {
            source: format!("ayni://orchestrator/supervisors/{executor}"),
            id: key(),
        };
        sign_report(&claim_identity, &mut report, &seed)?;
        let exact = encode_report(&claim_identity, &metadata, &report)?;
        assert!(self.store.accept_claim(&exact).await?.accepted);
        let config = command.frozen_configuration.ok_or("config")?;
        let manifest = command.context_manifest.ok_or("manifest")?;
        let evidence = ProviderCallEvidence {
            call_id: identity.id,
            provider: config.provider,
            model: config.model,
            configuration_revision_id: config.revision_id,
            context_digest: manifest.digest,
            request_digest: format!("sha256:{}", "d".repeat(64)),
            reserved_input_tokens: config.max_input_tokens,
            reserved_output_tokens: config.max_output_tokens,
            reserved_total_tokens: config.max_total_tokens,
            state: "sending".into(),
            ..Default::default()
        };
        Ok(Call {
            bytes,
            binding,
            metadata,
            evidence,
            executor,
            seed,
            public,
        })
    }
    async fn cancel(
        &self,
        turn: &str,
        key: &str,
    ) -> Result<
        ayni_orchestrator_workflow_postgres::store::provider_calls::CancellationReceipt,
        StoreError,
    > {
        self.store
            .cancel_turn(
                &self.workspace,
                &self.project,
                &self.conversation,
                turn,
                key,
            )
            .await
    }
    async fn state(&self, turn: &str) -> Result<String, tokio_postgres::Error> {
        Ok(self
            .client
            .query_one(
                "SELECT execution_state FROM workflow_turns WHERE id=$1",
                &[&turn],
            )
            .await?
            .get(0))
    }
    async fn settled(&self, turn: &str) -> Result<bool, tokio_postgres::Error> {
        Ok(self
            .client
            .query_one(
                "SELECT reservation_settled FROM workflow_provider_calls WHERE turn_id=$1",
                &[&turn],
            )
            .await?
            .get(0))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
struct Call {
    bytes: Vec<u8>,
    binding: ConversationTurnBinding,
    metadata: WorkflowEventMetadata,
    evidence: ProviderCallEvidence,
    executor: String,
    seed: [u8; 32],
    public: [u8; 32],
}
impl Call {
    fn request(&self, action: &str) -> AuthorityRequest {
        AuthorityRequest {
            command_bytes: self.bytes.clone(),
            action: action.into(),
            call_id: self.evidence.call_id.clone(),
            request_digest: self.evidence.request_digest.clone(),
            context_digest: self.evidence.context_digest.clone(),
            reserved_input_tokens: self.evidence.reserved_input_tokens,
            reserved_output_tokens: self.evidence.reserved_output_tokens,
            reserved_total_tokens: self.evidence.reserved_total_tokens,
            ..Default::default()
        }
    }
    fn terminal(
        &self,
        state: &str,
        usage: Option<u64>,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        let complete = state == "completed";
        let mut evidence = self.evidence.clone();
        evidence.state = state.into();
        evidence.total_tokens = usage;
        if complete {
            evidence.response_id = "response-fixture".into();
            evidence.reported_model = "reported-fixture-model".into();
        }
        let mut report = ConversationTurnReport {
            binding: Some(self.binding.clone()),
            kind: if complete {
                ConversationTurnReportKind::Completed.into()
            } else {
                ConversationTurnReportKind::Failed.into()
            },
            report_sequence: 2,
            executor_instance_id: self.executor.clone(),
            signing_key_digest: signing_key_digest(&self.public),
            response_content: if complete {
                "Retained provider result".into()
            } else {
                String::new()
            },
            failure_reason: if state == "uncertain" {
                "provider_uncertain".into()
            } else if state == "cancelled" {
                "cancelled".into()
            } else {
                String::new()
            },
            provider_call: Some(evidence),
            ..Default::default()
        };
        let identity = EventIdentity {
            source: format!("ayni://orchestrator/supervisors/{}", self.executor),
            id: key(),
        };
        sign_report(&identity, &mut report, &self.seed)?;
        Ok(encode_report(&identity, &self.metadata, &report)?)
    }
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn cancellation_before_claim_and_queued_cancellation_preserve_fifo_receipts() -> TestResult {
    let f = Fixture::new().await?;
    let first = f.send().await?;
    let queued = f.send().await?;
    let successor = f.send().await?;
    let cancel_key = key();
    let receipt = f.cancel(&queued, &cancel_key).await?;
    assert_eq!(receipt.state, "cancelled");
    assert_eq!(
        f.cancel(&queued, &cancel_key).await?.command_id,
        receipt.command_id
    );
    assert!(matches!(
        f.cancel(&successor, &cancel_key).await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert_eq!(f.state(&queued).await?, "failed");
    assert_eq!(f.state(&first).await?, "not_started");
    f.cancel(&first, &key()).await?;
    let scheduling: String = f
        .client
        .query_one(
            "SELECT scheduling_state FROM workflow_turns WHERE id=$1",
            &[&successor],
        )
        .await?
        .get(0);
    assert_eq!(scheduling, "eligible");
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn cancellation_before_send_refuses_call_admission_and_replays_original_receipt() -> TestResult
{
    let f = Fixture::new().await?;
    let turn = f.send().await?;
    let call = f.claim(&turn).await?;
    let receipt = f.cancel(&turn, &key()).await?;
    assert_eq!(receipt.state, "cancellation_requested");
    let denied = f.store.session_authority(&call.request("admit")).await?;
    assert!(!denied.allowed);
    assert!(denied.cancellation_requested);
    assert_eq!(denied.state, "cancelled");
    let terminal = call.terminal("cancelled", None)?;
    assert!(f.store.accept_terminal_report(&terminal).await?.accepted);
    assert_eq!(f.state(&turn).await?, "failed");
    assert!(f.settled(&turn).await?);
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn uncertain_admission_replay_retains_budget_and_blocks_replacement() -> TestResult {
    let f = Fixture::new().await?;
    let turn = f.send().await?;
    let call = f.claim(&turn).await?;
    let page = f.store.session_authority(&call.request("context")).await?;
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.context_digest, call.evidence.context_digest);
    let mut changed = call.request("admit");
    changed.call_id = key();
    assert!(matches!(
        f.store.session_authority(&changed).await,
        Err(StoreError::InvalidInput("provider_call_binding"))
    ));
    assert!(
        f.store
            .session_authority(&call.request("admit"))
            .await?
            .allowed
    );
    let replay = f.store.session_authority(&call.request("admit")).await?;
    assert!(!replay.allowed);
    assert_eq!(replay.state, "uncertain");
    let queued = f.send().await?;
    f.cancel(&turn, &key()).await?;
    let poll = f.store.session_authority(&call.request("poll")).await?;
    assert!(poll.cancellation_requested);
    assert!(!poll.allowed);
    let report = call.terminal("uncertain", None)?;
    assert!(f.store.accept_terminal_report(&report).await?.accepted);
    assert!(f.store.accept_terminal_report(&report).await?.accepted);
    assert_eq!(f.state(&turn).await?, "processing");
    assert!(!f.settled(&turn).await?);
    let scheduling: String = f
        .client
        .query_one(
            "SELECT scheduling_state FROM workflow_turns WHERE id=$1",
            &[&queued],
        )
        .await?
        .get(0);
    assert_eq!(scheduling, "waiting_for_predecessor");
    let sessions = f
        .store
        .conversation_sessions(&f.workspace, &f.project, &f.conversation)
        .await?;
    assert!(!sessions[0].can_replace);
    assert!(matches!(
        f.store
            .replace_idle_session(
                &f.workspace,
                &f.project,
                &f.conversation,
                sessions[0].generation,
                &key()
            )
            .await,
        Err(StoreError::InvalidInput("session_has_retained_work"))
    ));
    let snapshot = f
        .store
        .get_snapshot(&f.workspace, &f.project, &f.conversation)
        .await?;
    assert!(
        snapshot
            .recent_messages
            .iter()
            .all(|m| m.content != "Retained provider result")
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn cancellation_and_completion_choose_one_visible_outcome_but_keep_usage() -> TestResult {
    for cancel_first in [true, false] {
        let f = Fixture::new().await?;
        let turn = f.send().await?;
        let call = f.claim(&turn).await?;
        assert!(
            f.store
                .session_authority(&call.request("admit"))
                .await?
                .allowed
        );
        if cancel_first {
            f.cancel(&turn, &key()).await?;
        }
        let report = call.terminal("completed", Some(12))?;
        assert!(f.store.accept_terminal_report(&report).await?.accepted);
        assert!(f.store.accept_terminal_report(&report).await?.accepted);
        let receipt = f.cancel(&turn, &key()).await?;
        assert!(!receipt.cancellation_requested);
        assert_eq!(
            f.state(&turn).await?,
            if cancel_first { "failed" } else { "completed" }
        );
        assert!(f.settled(&turn).await?);
        let calls = f
            .store
            .list_provider_calls(&f.workspace, &f.project, &f.conversation)
            .await?;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].evidence.total_tokens, Some(12));
        let snapshot = f
            .store
            .get_snapshot(&f.workspace, &f.project, &f.conversation)
            .await?;
        assert_eq!(
            snapshot
                .recent_messages
                .iter()
                .filter(|m| m.content == "Retained provider result")
                .count(),
            usize::from(!cancel_first)
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn accepted_result_with_unknown_usage_remains_factual_and_reservation_stays_charged()
-> TestResult {
    let f = Fixture::new().await?;
    let turn = f.send().await?;
    let call = f.claim(&turn).await?;
    assert!(
        f.store
            .session_authority(&call.request("admit"))
            .await?
            .allowed
    );
    assert!(
        f.store
            .accept_terminal_report(&call.terminal("completed", None)?)
            .await?
            .accepted
    );
    assert_eq!(f.state(&turn).await?, "completed");
    assert!(!f.settled(&turn).await?);
    let calls = f
        .store
        .list_provider_calls(&f.workspace, &f.project, &f.conversation)
        .await?;
    assert_eq!(calls[0].evidence.total_tokens, None);
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn concurrent_cancellation_and_terminal_acceptance_use_one_lock_order() -> TestResult {
    let f = Fixture::new().await?;
    let turn = f.send().await?;
    let call = f.claim(&turn).await?;
    assert!(
        f.store
            .session_authority(&call.request("admit"))
            .await?
            .allowed
    );
    let report = call.terminal("completed", Some(12))?;
    let cancel_key = key();
    let (cancel, terminal) = tokio::join!(
        f.cancel(&turn, &cancel_key),
        f.store.accept_terminal_report(&report)
    );
    let receipt = cancel?;
    assert!(terminal?.accepted);
    let snapshot = f
        .store
        .get_snapshot(&f.workspace, &f.project, &f.conversation)
        .await?;
    assert_eq!(
        snapshot
            .recent_messages
            .iter()
            .filter(|message| message.content == "Retained provider result")
            .count(),
        usize::from(!receipt.cancellation_requested)
    );
    assert!(f.settled(&turn).await?);
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn snapshot_history_boundary_retries_concurrent_conversation_progress() -> TestResult {
    let f = Fixture::new().await?;
    let (writer, driver) = ayni_orchestrator_workflow_postgres::connect(&std::env::var(
        "AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL",
    )?)
    .await?;
    let conversation = f.conversation.clone();
    let ready = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let progress_ready = ready.clone();
    let progress = tokio::spawn(async move {
        progress_ready.wait().await;
        for _ in 0..20 {
            writer
                .execute(
                    "UPDATE workflow_conversations SET revision=revision+1 WHERE id=$1",
                    &[&conversation],
                )
                .await?;
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        driver.abort();
        Ok::<(), tokio_postgres::Error>(())
    });
    ready.wait().await;
    let mut last_revision = 0;
    for _ in 0..25 {
        let snapshot = f
            .store
            .get_snapshot(&f.workspace, &f.project, &f.conversation)
            .await?;
        assert!(snapshot.conversation_revision >= last_revision);
        assert!(!snapshot.history_snapshot_id.is_empty());
        last_revision = snapshot.conversation_revision;
    }
    progress.await??;
    Ok(())
}
