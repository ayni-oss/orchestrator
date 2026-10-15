use super::*;
use ayni_orchestrator_runtime::project_agent::{ProjectAgentRequest, ProjectAgentResult};

fn turn(sequence: u64) -> io::Result<ProjectSessionTurn> {
    Ok(ProjectSessionTurn {
        session_id: "session".into(),
        generation: 1,
        turn_sequence: sequence,
        pod_uid: "pod-uid".into(),
        deadline_at_unix_seconds: now()? + 300,
        request: ProjectAgentRequest {
            worker_profile: None,
            profile: None,
            run_id: format!("run-{sequence}"),
            project_id: "project".into(),
            model: "model".into(),
            history: String::new(),
            conversation_context: serde_json::Value::Null,
            repositories: vec![],
            context: String::new(),
            max_model_calls: 2,
            deadline_seconds: 300,
        },
    })
}
fn identity() -> Identity {
    Identity {
        session_id: "session".into(),
        generation: 1,
        pod_uid: "pod-uid".into(),
    }
}
#[tokio::test]
async fn local_framing_is_bounded_and_survives_partial_writes() -> io::Result<()> {
    let (mut sender, mut receiver) = tokio::io::duplex(32);
    let write = tokio::spawn(async move { write_frame(&mut sender, b"safe request").await });
    assert_eq!(read_frame(&mut receiver, 128).await?, b"safe request");
    write.await.map_err(io::Error::other)??;
    let (mut sender, mut receiver) = tokio::io::duplex(32);
    sender.write_u32(129).await?;
    assert!(read_frame(&mut receiver, 128).await.is_err());
    Ok(())
}
#[tokio::test]
async fn duplicate_preparation_never_rewrites_source_or_spends_another_turn() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let results = tempfile::tempdir()?;
    let first = turn(1)?;
    prepare(
        root.path(),
        results.path(),
        &identity(),
        ProjectSessionSubmission {
            turn: first.clone(),
            source_tokens: std::collections::BTreeMap::default(),
        },
    )
    .await?;
    let sentinel = turn_root(root.path(), &first).join("original-evidence");
    fs::write(&sentinel, "frozen source")?;
    prepare(
        root.path(),
        results.path(),
        &identity(),
        ProjectSessionSubmission {
            turn: first.clone(),
            source_tokens: std::collections::BTreeMap::default(),
        },
    )
    .await?;
    assert_eq!(fs::read_to_string(sentinel)?, "frozen source");
    let mut conflict = first;
    conflict.request.history = "different frozen input".into();
    assert!(
        prepare(
            root.path(),
            results.path(),
            &identity(),
            ProjectSessionSubmission {
                turn: conflict,
                source_tokens: std::collections::BTreeMap::default()
            }
        )
        .await
        .is_err()
    );
    Ok(())
}
#[tokio::test]
async fn another_turn_cannot_mutate_workspace_until_matching_research_completed() -> io::Result<()>
{
    let root = tempfile::tempdir()?;
    let results = tempfile::tempdir()?;
    let first = turn(1)?;
    prepare(
        root.path(),
        results.path(),
        &identity(),
        ProjectSessionSubmission {
            turn: first.clone(),
            source_tokens: std::collections::BTreeMap::default(),
        },
    )
    .await?;
    let second = turn(2)?;
    assert!(
        prepare(
            root.path(),
            results.path(),
            &identity(),
            ProjectSessionSubmission {
                turn: second.clone(),
                source_tokens: std::collections::BTreeMap::default()
            }
        )
        .await
        .is_err()
    );
    assert!(turn_root(root.path(), &first).exists());
    let mut finished = receipt(&first)?;
    atomic_json(
        &results.path().join("receipt.json"),
        &finished,
        RECEIPT_BYTES,
    )?;
    assert!(
        prepare(
            root.path(),
            results.path(),
            &identity(),
            ProjectSessionSubmission {
                turn: second.clone(),
                source_tokens: std::collections::BTreeMap::default()
            }
        )
        .await
        .is_err()
    );
    assert!(turn_root(root.path(), &first).exists());
    finished.result = Some(ProjectAgentResult {
        run_id: first.request.run_id.clone(),
        message: "completed".into(),
        proposals: vec![],
        model_calls: 1,
        token_usage: ayni_orchestrator_domain::ProviderTokenUsage::default(),
    });
    atomic_json(
        &results.path().join("receipt.json"),
        &finished,
        RECEIPT_BYTES,
    )?;
    prepare(
        root.path(),
        results.path(),
        &identity(),
        ProjectSessionSubmission {
            turn: second,
            source_tokens: std::collections::BTreeMap::default(),
        },
    )
    .await?;
    assert!(!turn_root(root.path(), &first).exists());
    Ok(())
}
#[test]
fn generation_pod_request_digest_and_absolute_deadline_are_fenced() -> io::Result<()> {
    let first = turn(1)?;
    let value = receipt(&first)?;
    assert!(first.matches_receipt(&value));
    let mut stale = value.clone();
    stale.generation += 1;
    assert!(!first.matches_receipt(&stale));
    let mut stale = value.clone();
    stale.pod_uid = "replacement".into();
    assert!(!first.matches_receipt(&stale));
    let mut stale = value;
    stale.request_digest = "different".into();
    assert!(!first.matches_receipt(&stale));
    let mut extended = first.clone();
    extended.deadline_at_unix_seconds += 1;
    assert!(!extended.matches_receipt(&receipt(&first)?));
    let mut stale = first;
    stale.deadline_at_unix_seconds = now()?.saturating_sub(1);
    assert!(remaining(&stale).is_err());
    stale.generation += 1;
    assert!(identity().validate(&stale).is_err());
    Ok(())
}
#[test]
fn safe_preparation_and_result_files_exclude_source_credentials() -> io::Result<()> {
    let first = turn(1)?;
    let ready = String::from_utf8(encode(&first, TURN_BYTES)?).map_err(io::Error::other)?;
    let result =
        String::from_utf8(encode(&receipt(&first)?, RECEIPT_BYTES)?).map_err(io::Error::other)?;
    assert!(!ready.contains("source_tokens"));
    assert!(!result.contains("source_tokens"));
    Ok(())
}

#[tokio::test]
async fn unix_socket_protocol_keeps_frames_separate_after_client_disconnect() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("checkout.sock");
    let listener = UnixListener::bind(&path)?;
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await?;
        let request = read_frame(&mut first, 1024).await?;
        write_frame(&mut first, b"accepted").await?;
        drop(first);
        let (mut retry, _) = listener.accept().await?;
        assert_eq!(read_frame(&mut retry, 1024).await?, request);
        write_frame(&mut retry, b"accepted").await
    });
    for _ in 0..2 {
        let mut client = UnixStream::connect(&path).await?;
        write_frame(&mut client, b"same frozen turn").await?;
        assert_eq!(read_frame(&mut client, 1024).await?, b"accepted");
    }
    server.await.map_err(io::Error::other)??;
    Ok(())
}
