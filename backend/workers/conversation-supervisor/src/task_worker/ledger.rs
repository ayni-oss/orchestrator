//! Call intent precedes network I/O. A sending record is never automatically replayed.
use ayni_orchestrator_events::task_execution::{TaskCommand, TaskOutcome, digest};
use ayni_orchestrator_providers::responses::ResponseEvidence;
use rusqlite::{Connection, OptionalExtension as _, params};
use std::{io, path::Path};
pub struct Ledger {
    connection: Connection,
}
impl Ledger {
    pub fn open(root: &Path, command: &TaskCommand) -> io::Result<Self> {
        let connection = Connection::open(root.join("execution.db")).map_err(error)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS execution(singleton INTEGER PRIMARY KEY CHECK(singleton=1),command_digest TEXT NOT NULL,state TEXT NOT NULL,request BLOB,response BLOB,outcome BLOB);").map_err(error)?;
        let hash = digest(&serde_json::to_vec(command).map_err(io::Error::other)?);
        connection.execute("INSERT OR IGNORE INTO execution(singleton,command_digest,state) VALUES(1,?1,'ready')",[&hash]).map_err(error)?;
        let retained: String = connection
            .query_row("SELECT command_digest FROM execution", [], |r| r.get(0))
            .map_err(error)?;
        if retained != hash {
            return Err(io::Error::other("task ledger identity mismatch"));
        }
        Ok(Self { connection })
    }
    pub fn request_digest(&self) -> io::Result<String> {
        let bytes: Vec<u8> = self
            .connection
            .query_row("SELECT request FROM execution", [], |r| r.get(0))
            .map_err(error)?;
        Ok(digest(&bytes))
    }
    pub fn state(&self) -> io::Result<String> {
        self.connection
            .query_row("SELECT state FROM execution", [], |r| r.get(0))
            .map_err(error)
    }
    pub fn sending(&mut self, request: &[u8]) -> io::Result<()> {
        let tx = self.connection.transaction().map_err(error)?;
        let changed = tx
            .execute(
                "UPDATE execution SET state='sending',request=?1 WHERE state='ready'",
                [request],
            )
            .map_err(error)?;
        if changed != 1 {
            return Err(io::Error::other("task call already started"));
        }
        tx.commit().map_err(error)
    }
    pub fn retain_response(&mut self, evidence: &ResponseEvidence) -> io::Result<()> {
        let tx = self.connection.transaction().map_err(error)?;
        tx.execute(
            "UPDATE execution SET state='received',response=?1",
            [serde_json::to_vec(evidence).map_err(io::Error::other)?],
        )
        .map_err(error)?;
        tx.commit().map_err(error)
    }
    pub fn response(&self) -> io::Result<Option<ResponseEvidence>> {
        self.read("response")
    }
    pub fn outcome(&self) -> io::Result<Option<TaskOutcome>> {
        self.read("outcome")
    }
    fn read<T: serde::de::DeserializeOwned>(&self, column: &str) -> io::Result<Option<T>> {
        let bytes: Option<Vec<u8>> = self
            .connection
            .query_row(&format!("SELECT {column} FROM execution"), [], |r| r.get(0))
            .optional()
            .map_err(error)?
            .flatten();
        bytes
            .map(|b| serde_json::from_slice(&b).map_err(io::Error::other))
            .transpose()
    }
    pub fn finish(&self, outcome: &TaskOutcome) -> io::Result<()> {
        self.connection
            .execute(
                "UPDATE execution SET state='finished',outcome=?1",
                params![serde_json::to_vec(outcome).map_err(io::Error::other)?],
            )
            .map_err(error)?;
        Ok(())
    }
}
fn error(_: rusqlite::Error) -> io::Error {
    io::Error::other("task ledger unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sending_and_accepted_results_survive_reopening_without_another_call() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let command = TaskCommand {
            attempt_id: "fixture".into(),
            generation: 1,
            ..Default::default()
        };
        let mut ledger = Ledger::open(root.path(), &command)?;
        assert_eq!(ledger.state()?, "ready");
        ledger.sending(b"exact request")?;
        drop(ledger);
        let mut ledger = Ledger::open(root.path(), &command)?;
        assert_eq!(ledger.state()?, "sending");
        assert!(ledger.sending(b"another call").is_err());
        let evidence = ResponseEvidence {
            response_id: "response".into(),
            model: "fixture".into(),
            content: String::new(),
            usage: ayni_orchestrator_providers::responses::TokenUsage::default(),
            failure_reason: String::new(),
            tool_calls: vec![],
        };
        ledger.retain_response(&evidence)?;
        assert_eq!(
            ledger.response()?.map(|v| v.response_id),
            Some("response".into())
        );
        let result = TaskOutcome {
            state: "completed".into(),
            patch: "retained patch".into(),
            ..Default::default()
        };
        ledger.finish(&result)?;
        drop(ledger);
        assert_eq!(
            Ledger::open(root.path(), &command)?.outcome()?,
            Some(result)
        );
        let changed = TaskCommand {
            generation: 2,
            ..command
        };
        assert!(Ledger::open(root.path(), &changed).is_err());
        Ok(())
    }
}
