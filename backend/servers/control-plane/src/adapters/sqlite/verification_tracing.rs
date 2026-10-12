//! Best-effort diagnostic context in the existing cursor store, outside authority.
use super::{CleanStore, OptionalExtension, params};
use ayni_orchestrator_observability::TraceContext;
impl CleanStore {
    pub(crate) fn verification_trace_context(&self, execution: &str) -> Option<TraceContext> {
        let raw: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM reconciliation_cursors WHERE name=?1",
                [format!("trace:verification:{execution}")],
                |row| row.get(0),
            )
            .optional()
            .ok()?;
        let context: TraceContext = serde_json::from_str(&raw?).ok()?;
        TraceContext::parse(&context.traceparent, context.tracestate.as_deref())
    }
    pub(crate) fn retain_verification_trace(&self, execution: &str, context: &TraceContext) {
        if !ayni_orchestrator_runtime::verification::valid_execution(execution) {
            return;
        }
        let Some(context) =
            TraceContext::parse(&context.traceparent, context.tracestate.as_deref())
        else {
            return;
        };
        let Ok(raw) = serde_json::to_string(&context) else {
            return;
        };
        // First originating context wins. Failure to retain diagnostics cannot
        // change receipt, execution accounting or runtime acceptance.
        let _ = self.connection.execute(
            "INSERT OR IGNORE INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES(?1,?2,1,?3)",
            params![format!("trace:verification:{execution}"),raw,super::unix_seconds().unwrap_or(0)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn originating_context_survives_restart_and_diagnostic_write_failure_is_optional()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("traces.sqlite");
        let store = CleanStore::open(&path, 3)?;
        let original = TraceContext::parse(
            "00-7431fc7927bb4f208986f2a4f2418dba-3f49c016c2df402b-01",
            None,
        )
        .ok_or("context")?;
        let changed = TraceContext::parse(
            "00-7431fc7927bb4f208986f2a4f2418dba-4f49c016c2df402b-01",
            None,
        )
        .ok_or("context")?;
        store.retain_verification_trace("verify-one", &original);
        store.retain_verification_trace("verify-one", &changed);
        assert_eq!(
            store.verification_trace_context("verify-one"),
            Some(original.clone())
        );
        store.connection.execute_batch("CREATE TRIGGER reject_diagnostics BEFORE INSERT ON reconciliation_cursors WHEN NEW.name LIKE 'trace:%' BEGIN SELECT RAISE(ABORT,'diagnostic failure'); END;")?;
        store.retain_verification_trace("verify-two", &changed);
        assert!(store.verification_trace_context("verify-two").is_none());
        assert_eq!(store.event_watermark()?, 0);
        drop(store);
        let store = CleanStore::open(&path, 3)?;
        assert_eq!(
            store.verification_trace_context("verify-one"),
            Some(original)
        );
        Ok(())
    }
}
