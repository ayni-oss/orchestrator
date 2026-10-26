//! One paid exchange with bounded provisional text and a linked diagnostic span.
use super::*;
pub(super) async fn paid_request(
    state: &WorkerState,
    call_id: &str,
    command_id: &str,
    bytes: &[u8],
    context: Option<&TraceContext>,
    turn_id: &str,
) -> Result<ResponseEvidence, ()> {
    let client = state
        .provider
        .lock()
        .ok()
        .and_then(|client| client.clone())
        .ok_or(())?;
    let command_digest: String = state
        .connection
        .lock()
        .map_err(|_| ())?
        .query_row(
            "SELECT command_digest FROM provider_calls WHERE command_id=?1",
            [command_id],
            |row| row.get(0),
        )
        .map_err(|_| ())?;
    let round = call_id
        .strip_prefix(&format!("{command_id}-"))
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(0);
    let observe = |text: &str| {
        let _ = super::super::super::progress::record(
            state,
            command_id,
            &command_digest,
            ayni_orchestrator_events::progress::ExecutionProgress {
                stage: "model".into(),
                round,
                call_id: call_id.into(),
                provisional_text: text.into(),
                ..Default::default()
            },
        );
        Ok(())
    };
    let _ = observe("");
    let span = Workflow {
        project_id: state.project_id.clone(),
        conversation_id: state.conversation_id.clone(),
        run_id: command_id.into(),
        turn_id: turn_id.into(),
        provider_call_id: call_id.into(),
        session_id: state.session_id.clone(),
        generation: state.generation,
        ..Default::default()
    }
    .span_with_context("provider.model.execute", context, false);
    let response = client
        .send_streaming(call_id, bytes, observe)
        .instrument(span.clone());
    tokio::pin!(response);
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            result=&mut response => { finish(&span, result.is_ok()); return result.map_err(|_|()); },
            _=interval.tick()=>if cancelled(state,command_id) { span.record("outcome", "cancelled"); return Err(()); },
        }
    }
}
