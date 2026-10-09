use super::super::{
    activity::ActivityLog,
    sessions::{SessionState, run},
};
use super::*;
use rig_core::{
    completion::{CompletionError, CompletionModel, CompletionRequest, CompletionResponse},
    streaming::StreamingCompletionResponse,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingFailure(Arc<AtomicUsize>);
impl CompletionModel for CountingFailure {
    async fn completion(
        &self,
        _: CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(CompletionError::ProviderError(
            "test provider unavailable".into(),
        ))
    }
    async fn stream(
        &self,
        _: CompletionRequest,
    ) -> Result<StreamingCompletionResponse, CompletionError> {
        Err(CompletionError::ProviderError("unused streaming".into()))
    }
}

#[tokio::test]
async fn shared_hook_budget_is_durable_across_sessions_and_stops_provider_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let workspace = directory.path().join("workspace");
    let output = directory.path().join("output");
    let spec = super::super::tests::fixture(&workspace)?;
    let context = Context {
        spec: Arc::new(spec),
        workspace,
        output: output.clone(),
        state: Arc::new(tokio::sync::Mutex::new(SessionState::default())),
        serial: Arc::new(tokio::sync::Mutex::new(())),
        activity: ActivityLog::default(),
    };
    let calls = Arc::new(AtomicUsize::new(0));
    for _ in 0..3 {
        let agent = CountingFailure(calls.clone())
            .into_agent_builder()
            .add_hook(BudgetHook(context.clone()))
            .build();
        assert!(
            agent
                .prompt("Original request retained across repair sessions")
                .await
                .is_err()
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(context.state.lock().await.progress.coding_turns_consumed, 2);
    let durable: SessionState = recovery::read_json(&output.join("session.json"))?;
    assert_eq!(durable.progress.coding_turns_consumed, 2);
    // Exhaustion returns before model credentials are opened or a client constructed.
    assert!(run(&context, Some("Repair findings")).await?.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    Ok(())
}
