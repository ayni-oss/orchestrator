//! Verification tracing reuses retained diagnostic context, never admission state.
use super::{ExecutionBackend, RuntimeReconciler};
impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn verification_span(
        &self,
        stage: &'static str,
        execution: &str,
    ) -> tracing::Span {
        let name = execution.to_owned();
        let context = self
            .with_database(move |store| Ok(store.verification_trace_context(&name)))
            .await
            .ok()
            .flatten();
        let span =
            ayni_orchestrator_observability::verification_span(stage, execution, context.as_ref());
        if stage == "launch" && context.is_none() {
            let context = span.in_scope(ayni_orchestrator_observability::current_context);
            if let Some(context) = context {
                let name = execution.to_owned();
                let _ = self
                    .with_database(move |store| {
                        store.retain_verification_trace(&name, &context);
                        Ok(())
                    })
                    .await;
            }
        }
        span
    }
}
