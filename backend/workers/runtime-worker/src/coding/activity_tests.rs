use super::*;

#[test]
fn retains_consecutive_errors_and_redacted_interactions() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempfile::tempdir()?;
    let path = root.path().join("activity.json");
    let log = ActivityLog::new(Some(path.clone()));
    log.register_secret("test-api-credential");
    log.detail(Stage::ToolStarted, "run_command [call-1]\ncargo test");
    log.error(&std::io::Error::other(
        "protected generated lock contract is invalid",
    ));
    log.error(&std::io::Error::other(
        "unclassified context test-api-credential",
    ));
    let events: Vec<Activity> = serde_json::from_slice(&fs::read(path)?)?;
    assert_eq!(events.len(), 3);
    assert!(matches!(events[1].failure, Some(Failure::LockContract)));
    assert!(events[2].failed);
    assert!(
        events[2]
            .details
            .contains("unclassified context [REDACTED]")
    );
    let transcript = fs::read_to_string(root.path().join("diagnostics.jsonl"))?;
    assert_eq!(transcript.lines().count(), 3);
    assert!(!transcript.contains("test-api-credential"));
    assert!(ayni_orchestrator_runtime::activity::valid(&events));
    Ok(())
}

#[test]
fn snapshot_bounds_do_not_drop_the_retained_diagnostic_history()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("activity.json");
    let log = ActivityLog::new(Some(path.clone()));
    let text = "é".repeat(MAX_DETAIL_BYTES);
    for _ in 0..30 {
        log.detail(Stage::AgentMessage, &text);
    }
    log.error(&std::io::Error::other("terminal cause"));
    let bytes = fs::read(&path)?;
    assert!(bytes.len() <= MAX_ACTIVITY_BYTES);
    let events: Vec<Activity> = serde_json::from_slice(&bytes)?;
    assert!(events[0].sequence > 1);
    assert!(events[0].truncated);
    assert!(events.last().is_some_and(|event| event.failed));
    assert!(ayni_orchestrator_runtime::activity::valid(&events));
    let transcript = fs::read_to_string(root.path().join("diagnostics.jsonl"))?;
    assert_eq!(transcript.lines().count(), 31);
    assert!(transcript.contains(&text));
    Ok(())
}

#[test]
fn command_exit_status_is_distinct_from_tool_transport_success() {
    assert!(command_failed(
        "run_command",
        "exit=Some(1)\nstdout:\nfailed"
    ));
    assert!(command_failed("run_command", "exit=None\nstdout:\n"));
    assert!(!command_failed("run_command", "exit=Some(0)\nstdout:\nok"));
    assert!(!command_failed("commit_changes", "some-commit"));
}

#[test]
fn lock_and_policy_failures_keep_specific_classifications() -> Result<(), serde_json::Error> {
    for (error, expected) in [
        ("protected policy changed", "policy_changed"),
        (
            "protected generated lock contract is invalid",
            "lock_contract",
        ),
        (
            "protected proposed lock lacks worker regeneration evidence",
            "lock_evidence_missing",
        ),
        (
            "protected proposed lock regeneration evidence does not match final inputs",
            "lock_evidence_mismatch",
        ),
        (
            "task changes a protected dependency path outside reviewed environment scope",
            "dependency_scope",
        ),
    ] {
        assert_eq!(
            serde_json::to_value(io_failure(&std::io::Error::other(error)))?,
            expected
        );
    }
    Ok(())
}

#[test]
fn reopening_diagnostics_preserves_sequence_and_prior_errors()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("activity.json");
    let log = ActivityLog::new(Some(path.clone()));
    log.error(&std::io::Error::other("first cause"));
    drop(log);
    let log = ActivityLog::new(Some(path.clone()));
    log.record(Stage::Finalization);
    let events: Vec<Activity> = serde_json::from_slice(&fs::read(path)?)?;
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].sequence, 2);
    assert!(events[0].failed);
    Ok(())
}

#[test]
fn generated_lock_contract_errors_do_not_accuse_the_agent_of_unsafe_edits() {
    assert!(!unsafe_change(&std::io::Error::other(
        "protected generated lock contract is invalid"
    )));
    assert!(unsafe_change(&std::io::Error::other(
        "task changes a protected path"
    )));
    assert!(unsafe_change(&std::io::Error::other(
        "protected proposed lock lacks worker regeneration evidence"
    )));
}

#[tokio::test]
async fn completion_hook_retains_public_output_and_provider_summary_only()
-> Result<(), Box<dyn std::error::Error>> {
    use rig_agent::{client::AgentModelExt, completion::Prompt};
    use rig_core::{
        completion::{
            CompletionError, CompletionModel, CompletionRequest, CompletionResponse, Usage,
        },
        message::{AssistantContent, Reasoning},
        streaming::StreamingCompletionResponse,
    };
    struct Model;
    impl CompletionModel for Model {
        async fn completion(
            &self,
            _: CompletionRequest,
        ) -> Result<CompletionResponse, CompletionError> {
            Ok(CompletionResponse::new(
                vec![
                    AssistantContent::Reasoning(Reasoning {
                        id: None,
                        content: vec![
                            rig_core::message::ReasoningContent::Encrypted("opaque-state".into()),
                            rig_core::message::ReasoningContent::Summary(
                                "Reviewing dependency inputs".into(),
                            ),
                        ],
                    }),
                    AssistantContent::text("I will validate the lock"),
                ],
                Usage::default(),
                "test",
            ))
        }
        async fn stream(
            &self,
            _: CompletionRequest,
        ) -> Result<StreamingCompletionResponse, CompletionError> {
            Err(CompletionError::ProviderError("unused streaming".into()))
        }
    }
    let root = tempfile::tempdir()?;
    let path = root.path().join("activity.json");
    let log = ActivityLog::new(Some(path.clone()));
    Model
        .into_agent_builder()
        .add_hook(log)
        .build()
        .prompt("Original task request")
        .await?;
    let text = fs::read_to_string(path)?;
    assert!(text.contains("I will validate the lock"));
    assert!(text.contains("Reviewing dependency inputs"));
    assert!(!text.contains("opaque-state"));
    Ok(())
}

#[test]
fn provider_failure_retains_actionable_fields_without_dumping_raw_body()
-> Result<(), Box<dyn std::error::Error>> {
    use rig_core::{ProviderResponseError, completion::CompletionError};
    let root = tempfile::tempdir()?;
    let path = root.path().join("activity.json");
    let log = ActivityLog::new(Some(path.clone()));
    log.register_secret("registered-credential");
    let error = rig_agent::completion::PromptError::CompletionError(
        CompletionError::ProviderResponse(ProviderResponseError::new(
            "400".parse()?,
            r#"{"error":{"code":"unsupported_parameter","message":"summary invalid registered-credential"},"opaque":"do not log"}"#,
        )),
    );
    log.failure_detail(
        prompt_failure(&error),
        &super::super::diagnostics::prompt_diagnostic(&error),
    );
    let text = fs::read_to_string(path)?;
    assert!(text.contains("unsupported_parameter"));
    assert!(text.contains("summary invalid [REDACTED]"));
    assert!(!text.contains("registered-credential"));
    assert!(!text.contains("do not log"));
    Ok(())
}
