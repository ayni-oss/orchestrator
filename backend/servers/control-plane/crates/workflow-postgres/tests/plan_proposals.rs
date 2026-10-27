use ayni_orchestrator_events::{
    EventIdentity,
    v1::{
        ConversationTurnBinding, ConversationTurnReport, ConversationTurnReportKind, PlanProposal,
        ProviderCallEvidence,
    },
    workflow::{
        WorkflowEventMetadata, decode_command, decode_report, encode_report, sign_report,
        signing_key_digest,
    },
};
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore,
    plans::{PlanContent, PlanScope},
    store::provider_calls::AuthorityRequest,
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
            connection_id: String::new(),
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
    fn terminal(&self, proposal: &[u8]) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        let mut evidence = self.evidence.clone();
        evidence.state = "completed".into();
        evidence.total_tokens = Some(42);
        evidence.response_id = "response-fixture".into();
        evidence.reported_model = "reported-fixture-model".into();
        let mut report = ConversationTurnReport {
            binding: Some(self.binding.clone()),
            kind: ConversationTurnReportKind::Completed.into(),
            report_sequence: 2,
            executor_instance_id: self.executor.clone(),
            signing_key_digest: signing_key_digest(&self.public),
            response_content: "Retained provider result".into(),
            failure_reason: String::new(),
            plan_proposal: Some(PlanProposal {
                invocation_id: "proposal_fixture".into(),
                proposal_json: proposal.to_vec(),
            }),
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

fn proposal(title: &str) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&serde_json::json!({
        "content": {"title":title,"objective":"Retain discussion outcome","scope":"This conversation",
            "out_of_scope":"Execution and approval","constraints":["Review required"],
            "unresolved_questions":["Which repository commit?"],"tasks":[{
                "key":"review","objective":"Review source","scope":"Declared repository",
                "out_of_scope":"Dispatch","acceptance_criteria":["Source verified before execution"],
                "repository":"declared/example","source_ref":"unverified","execution_limits":"No execution",
                "unresolved_inputs":["Verify commit"]}]},
        "assumptions":["Repository declared by the user"],"evidence":["Conversation discussion; not verified repository evidence"]
    }))
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
#[allow(
    clippy::too_many_lines,
    reason = "cumulative authority and recovery scenario keeps transaction ordering visible"
)]
async fn agent_proposals_retain_bound_origin_and_replay_without_duplicate_or_unauthorized_writes()
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
    let bytes = call.terminal(&proposal("Agent draft")?)?;
    let scope = PlanScope {
        workspace: &f.workspace,
        project: &f.project,
        conversation: &f.conversation,
    };
    let (first, replay) = tokio::join!(
        f.store.accept_terminal_report(&bytes),
        f.store.accept_terminal_report(&bytes)
    );
    let receipt = first?;
    assert!(receipt.accepted);
    assert_eq!(receipt, replay?);
    assert_eq!(f.state(&turn).await?, "completed");
    assert!(f.settled(&turn).await?);
    let heads = f.store.list_plans(scope).await?;
    assert_eq!(heads.len(), 1);
    let version = f.store.get_plan_version(scope, &heads[0].id, "").await?;
    assert_eq!(version.author, "agent");
    let source = version.provenance.as_ref().ok_or("provenance")?;
    assert_eq!(source.source_turn_id, turn);
    assert_eq!(
        source.configuration_revision_id,
        call.evidence.configuration_revision_id
    );
    assert_eq!(source.context_digest, call.evidence.context_digest);
    assert_eq!(source.provider_call_id, call.evidence.call_id);
    assert_eq!(source.provider_response_id, "response-fixture");
    assert_eq!(source.assignment_id, call.binding.assignment_id);
    assert_eq!(source.executor_generation, call.binding.executor_generation);
    assert_eq!(source.report_id, receipt.report_id);
    assert_eq!(source.invocation_id, "proposal_fixture");
    assert_eq!(source.assumptions.len(), 1);
    assert_eq!(source.evidence.len(), 1);
    let restarted = WorkflowStore::new(std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?);
    assert_eq!(restarted.accept_terminal_report(&bytes).await?, receipt);
    assert_eq!(
        restarted.get_plan_version(scope, &heads[0].id, "").await?,
        version
    );
    let other = f
        .store
        .create_conversation(&key(), &f.workspace, &f.project, "Foreign")
        .await?
        .resource;
    let foreign = PlanScope {
        conversation: &other.id,
        ..scope
    };
    assert!(f.store.list_plans(foreign).await?.is_empty());
    assert!(matches!(
        f.store.get_plan_version(foreign, &heads[0].id, "").await,
        Err(StoreError::NotFound)
    ));
    assert!(
        f.client
            .execute(
                "UPDATE workflow_plan_proposals SET invocation_id='changed' WHERE version_id=$1",
                &[&version.id]
            )
            .await
            .is_err()
    );
    assert!(
        f.client
            .execute(
                "DELETE FROM workflow_plan_proposals WHERE version_id=$1",
                &[&version.id]
            )
            .await
            .is_err()
    );
    assert!(
        f.client
            .execute(
                "DELETE FROM workflow_plan_versions WHERE id=$1",
                &[&version.id]
            )
            .await
            .is_err()
    );

    // Failed insertion rolls the signed receipt, head, version and update back together.
    let next = f.send().await?;
    let next_call = f.claim(&next).await?;
    f.store
        .session_authority(&next_call.request("admit"))
        .await?;
    let next_bytes = next_call.terminal(&proposal("Rollback draft")?)?;
    let (admin, admin_driver) = ayni_orchestrator_workflow_postgres::connect(&std::env::var(
        "AYNI_POSTGRES_FEASIBILITY_URL",
    )?)
    .await?;
    admin.batch_execute("ALTER TABLE workflow_plan_versions ADD CONSTRAINT proposal_rollback_fixture CHECK(title <> 'Rollback draft')").await?;
    let failed = f.store.accept_terminal_report(&next_bytes).await;
    admin
        .batch_execute(
            "ALTER TABLE workflow_plan_versions DROP CONSTRAINT proposal_rollback_fixture",
        )
        .await?;
    assert!(failed.is_err());
    assert_eq!(f.state(&next).await?, "processing");
    let (_, _, report) = decode_report(&next_bytes)?;
    assert_eq!(f.client.query_one("SELECT count(*) FROM workflow_report_receipts WHERE turn_id=$1 AND report_sequence=2", &[&next]).await?.get::<_, i64>(0), 0);
    assert_eq!(f.store.list_plans(scope).await?.len(), 1);
    assert!(report.plan_proposal.is_some());
    assert!(f.store.accept_terminal_report(&next_bytes).await?.accepted);
    assert_eq!(f.store.list_plans(scope).await?.len(), 2);
    admin_driver.abort();
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn invalid_cancelled_stale_and_capacity_proposals_never_materialize_a_plan() -> TestResult {
    let f = Fixture::new().await?;
    let scope = PlanScope {
        workspace: &f.workspace,
        project: &f.project,
        conversation: &f.conversation,
    };
    malformed_draft_fails(&f, scope).await?;
    fenced_and_cancelled_drafts_are_suppressed(&f, scope).await?;
    exhausted_plan_capacity_fails(&f, scope).await?;
    Ok(())
}

async fn malformed_draft_fails(f: &Fixture, scope: PlanScope<'_>) -> TestResult {
    // A valid completed provider call with malformed draft is durably failed;
    // accounting survives and replay never resends a paid request.
    let turn = f.send().await?;
    let call = f.claim(&turn).await?;
    f.store.session_authority(&call.request("admit")).await?;
    let invalid = call
        .terminal(br#"{"content":{},"assumptions":[],"evidence":[],"conversation_id":"spoof"}"#)?;
    let accepted = f.store.accept_terminal_report(&invalid).await?;
    assert!(accepted.accepted);
    assert_eq!(f.store.accept_terminal_report(&invalid).await?, accepted);
    assert_eq!(f.state(&turn).await?, "failed");
    assert!(f.settled(&turn).await?);
    assert_eq!(
        f.client
            .query_one(
                "SELECT failure_reason FROM workflow_turns WHERE id=$1",
                &[&turn]
            )
            .await?
            .get::<_, String>(0),
        "invalid_plan_proposal"
    );
    assert!(f.store.list_plans(scope).await?.is_empty());

    Ok(())
}

async fn fenced_and_cancelled_drafts_are_suppressed(
    f: &Fixture,
    scope: PlanScope<'_>,
) -> TestResult {
    let turn = f.send().await?;
    let call = f.claim(&turn).await?;
    f.store.session_authority(&call.request("admit")).await?;
    let bytes = call.terminal(&proposal("Fenced draft")?)?;
    let (identity, metadata, mut report) = decode_report(&bytes)?;
    report
        .binding
        .as_mut()
        .ok_or("binding")?
        .executor_generation += 1;
    sign_report(&identity, &mut report, &call.seed)?;
    let stale = encode_report(&identity, &metadata, &report)?;
    let rejected = f.store.accept_terminal_report(&stale).await?;
    assert!(!rejected.accepted);
    assert_eq!(rejected.safe_reason, "stale_generation");
    assert!(f.store.list_plans(scope).await?.is_empty());
    // Wrong conversation scope cannot bind a proposal even with a valid signature.
    let other = f
        .store
        .create_conversation(&key(), &f.workspace, &f.project, "Other")
        .await?
        .resource;
    let (mut identity, metadata, mut report) = decode_report(&bytes)?;
    identity.id = key();
    report.binding.as_mut().ok_or("binding")?.conversation_id = other.id;
    sign_report(&identity, &mut report, &call.seed)?;
    let wrong = f
        .store
        .accept_terminal_report(&encode_report(&identity, &metadata, &report)?)
        .await?;
    assert!(!wrong.accepted);
    assert_eq!(wrong.safe_reason, "wrong_scope");
    // Cancellation wins before acceptance and suppresses an otherwise valid draft.
    f.cancel(&turn, &key()).await?;
    let cancelled_bytes = call.terminal(&proposal("Cancelled draft")?)?;
    assert!(
        f.store
            .accept_terminal_report(&cancelled_bytes)
            .await?
            .accepted
    );
    assert_eq!(f.state(&turn).await?, "failed");
    assert!(f.store.list_plans(scope).await?.is_empty());

    Ok(())
}

async fn exhausted_plan_capacity_fails(f: &Fixture, scope: PlanScope<'_>) -> TestResult {
    // All retained plans count toward the same bound before accepting a proposal.
    for index in 0..32 {
        f.store
            .create_plan(
                scope,
                &key(),
                "",
                &PlanContent {
                    title: format!("Operator {index}"),
                    objective: "Goal".into(),
                    scope: "Scope".into(),
                    ..Default::default()
                },
            )
            .await?;
    }
    let turn = f.send().await?;
    let call = f.claim(&turn).await?;
    f.store.session_authority(&call.request("admit")).await?;
    let bytes = call.terminal(&proposal("Excess draft")?)?;
    let receipt = f.store.accept_terminal_report(&bytes).await?;
    assert!(receipt.accepted);
    assert_eq!(f.store.accept_terminal_report(&bytes).await?, receipt);
    assert_eq!(f.state(&turn).await?, "failed");
    assert_eq!(
        f.client
            .query_one(
                "SELECT failure_reason FROM workflow_turns WHERE id=$1",
                &[&turn]
            )
            .await?
            .get::<_, String>(0),
        "plan_capacity"
    );
    assert_eq!(f.store.list_plans(scope).await?.len(), 32);
    Ok(())
}
