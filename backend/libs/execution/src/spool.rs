//! Transactional, bounded delivery journal. Acknowledgements never rerun actions.
#![allow(
    clippy::significant_drop_tightening,
    reason = "SQLite statements and transactions borrow the locked connection through their final operation"
)]
use ayni_orchestrator_telemetry::v1::TelemetryRecord;
use rusqlite::{Connection, OptionalExtension as _, params};
use std::{io, path::Path, sync::Mutex};

pub struct Spool {
    db: Mutex<Connection>,
    maximum: u64,
    rotation: Mutex<String>,
}

impl Spool {
    /// # Errors
    /// The parent must be trusted, private storage; reject an unusable journal.
    pub fn open(path: &Path, maximum: u64) -> io::Result<Self> {
        let db = Connection::open(path).map_err(io::Error::other)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS state(source TEXT PRIMARY KEY, next INTEGER NOT NULL, ack INTEGER NOT NULL, lost INTEGER NOT NULL, metadata TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS records(source TEXT NOT NULL, seq INTEGER NOT NULL, body TEXT NOT NULL, PRIMARY KEY(source,seq));
            CREATE TABLE IF NOT EXISTS checkpoints(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            UPDATE state SET metadata=json_remove(metadata,'$.data');").map_err(io::Error::other)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
        }
        Ok(Self {
            db: Mutex::new(db),
            maximum,
            rotation: Mutex::new(String::new()),
        })
    }

    /// Append exact bytes and identity atomically, reserving metadata space for gaps.
    /// # Errors
    /// Returns journal failures. A quota loss is retained as a gap, not an error.
    pub fn append(&self, mut record: TelemetryRecord) -> io::Result<()> {
        let source = record
            .source
            .as_ref()
            .ok_or_else(|| io::Error::other("missing source"))?
            .id
            .clone();
        let mut db = self
            .db
            .lock()
            .map_err(|_| io::Error::other("journal lock"))?;
        let tx = db.transaction().map_err(io::Error::other)?;
        tx.execute(
            "INSERT OR IGNORE INTO state VALUES(?1,1,0,0,?2)",
            params![
                source,
                serde_json::to_string(&TelemetryRecord {
                    source: record.source.clone(),
                    command_digest: record.command_digest.clone(),
                    ..Default::default()
                })
                .map_err(io::Error::other)?
            ],
        )
        .map_err(io::Error::other)?;
        let (mut seq, lost): (i64, i64) = tx
            .query_row(
                "SELECT next,lost FROM state WHERE source=?1",
                [&source],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(io::Error::other)?;
        let used: i64 = tx
            .query_row(
                "SELECT coalesce(sum(length(body)),0) FROM records",
                [],
                |r| r.get(0),
            )
            .map_err(io::Error::other)?;
        // Bound serialised bytes too (binary becomes base64 in protobuf JSON).
        let size = serde_json::to_vec(&record).map_err(io::Error::other)?.len() as u64;
        if used.unsigned_abs().saturating_add(size) > self.maximum {
            tx.execute(
                "INSERT INTO checkpoints(key,value) VALUES(?1,'true') ON CONFLICT(key) DO NOTHING",
                [format!("loss:{source}")],
            )
            .map_err(io::Error::other)?;
            tx.execute(
                "UPDATE state SET lost=lost+?2 WHERE source=?1",
                params![
                    source,
                    i64::try_from(record.data.len().max(1)).map_err(io::Error::other)?
                ],
            )
            .map_err(io::Error::other)?;
        } else {
            if lost > 0 {
                let gap = TelemetryRecord {
                    source: record.source.clone(),
                    sequence: seq.unsigned_abs(),
                    event: "capture_gap".into(),
                    lost_bytes: lost.unsigned_abs(),
                    observed_at_unix_ms: crate::now_ms(),
                    command_digest: record.command_digest.clone(),
                    ..Default::default()
                };
                tx.execute(
                    "INSERT INTO records VALUES(?1,?2,?3)",
                    params![
                        source,
                        seq,
                        serde_json::to_string(&gap).map_err(io::Error::other)?
                    ],
                )
                .map_err(io::Error::other)?;
                seq += 1;
            }
            record.sequence = seq.unsigned_abs();
            tx.execute(
                "INSERT INTO records VALUES(?1,?2,?3)",
                params![
                    source,
                    seq,
                    serde_json::to_string(&record).map_err(io::Error::other)?
                ],
            )
            .map_err(io::Error::other)?;
            tx.execute(
                "UPDATE state SET next=?2,lost=0 WHERE source=?1",
                params![source, seq + 1],
            )
            .map_err(io::Error::other)?;
        }
        tx.commit().map_err(io::Error::other)
    }

    fn flush_losses(&self) -> io::Result<()> {
        let mut db = self
            .db
            .lock()
            .map_err(|_| io::Error::other("journal lock"))?;
        let tx = db.transaction().map_err(io::Error::other)?;
        let losses = {
            let mut query = tx
                .prepare("SELECT source,next,lost,metadata FROM state WHERE lost>0 LIMIT 32")
                .map_err(io::Error::other)?;
            query
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })
                .map_err(io::Error::other)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(io::Error::other)?
        };
        for (source, seq, lost, metadata) in losses {
            // At most one reserved gap record per source waits outside the payload quota.
            let pending: i64 = tx
                .query_row(
                    "SELECT count(*) FROM records WHERE source=?1 AND body LIKE '%capture_gap%'",
                    [&source],
                    |r| r.get(0),
                )
                .map_err(io::Error::other)?;
            if pending > 0 {
                continue;
            }
            let gap = TelemetryRecord {
                source: serde_json::from_str::<TelemetryRecord>(&metadata)
                    .map_err(io::Error::other)?
                    .source,
                command_digest: serde_json::from_str::<TelemetryRecord>(&metadata)
                    .map_err(io::Error::other)?
                    .command_digest,
                sequence: seq.unsigned_abs(),
                event: "capture_gap".into(),
                lost_bytes: lost.unsigned_abs(),
                observed_at_unix_ms: crate::now_ms(),
                ..Default::default()
            };
            tx.execute(
                "INSERT INTO records VALUES(?1,?2,?3)",
                params![
                    source,
                    seq,
                    serde_json::to_string(&gap).map_err(io::Error::other)?
                ],
            )
            .map_err(io::Error::other)?;
            tx.execute(
                "UPDATE state SET next=?2,lost=0 WHERE source=?1",
                params![source, seq + 1],
            )
            .map_err(io::Error::other)?;
        }
        tx.commit().map_err(io::Error::other)
    }

    /// # Errors
    /// Return journal read errors.
    pub fn pending_bytes(&self) -> io::Result<u64> {
        let db = self
            .db
            .lock()
            .map_err(|_| io::Error::other("journal lock"))?;
        db.query_row(
            "SELECT coalesce(sum(length(body)),0) FROM records",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(i64::unsigned_abs)
        .map_err(io::Error::other)
    }

    /// # Errors
    /// Returns unavailable/corrupt journal errors.
    pub fn pending(&self) -> io::Result<Vec<TelemetryRecord>> {
        self.flush_losses()?;
        let db = self
            .db
            .lock()
            .map_err(|_| io::Error::other("journal lock"))?;
        let mut rotation = self
            .rotation
            .lock()
            .map_err(|_| io::Error::other("journal rotation"))?;
        let sources = {
            let mut query = db.prepare("SELECT DISTINCT source FROM records ORDER BY CASE WHEN source>?1 THEN 0 ELSE 1 END,source LIMIT 32").map_err(io::Error::other)?;
            query
                .query_map([&*rotation], |r| r.get::<_, String>(0))
                .map_err(io::Error::other)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(io::Error::other)?
        };
        let count = sources.len().max(1);
        let limit = i64::try_from(32 / count).map_err(io::Error::other)?;
        let mut records = Vec::new();
        for source in &sources {
            let mut query = db
                .prepare("SELECT body FROM records WHERE source=?1 ORDER BY seq LIMIT ?2")
                .map_err(io::Error::other)?;
            let rows = query
                .query_map(params![source, limit], |r| r.get::<_, String>(0))
                .map_err(io::Error::other)?;
            for row in rows {
                records.push(
                    serde_json::from_str(&row.map_err(io::Error::other)?)
                        .map_err(io::Error::other)?,
                );
            }
        }
        if let Some(last) = sources.last() {
            rotation.clone_from(last);
        }
        Ok(records)
    }

    /// # Errors
    /// Reject acknowledgements beyond the persisted sequence.
    pub fn acknowledge(&self, source: &str, sequence: u64) -> io::Result<()> {
        let mut db = self
            .db
            .lock()
            .map_err(|_| io::Error::other("journal lock"))?;
        let tx = db.transaction().map_err(io::Error::other)?;
        let next: i64 = tx
            .query_row("SELECT next FROM state WHERE source=?1", [source], |r| {
                r.get(0)
            })
            .map_err(io::Error::other)?;
        let sequence = i64::try_from(sequence).map_err(io::Error::other)?;
        if sequence >= next {
            return Err(io::Error::other("acknowledgement beyond journal"));
        }
        tx.execute(
            "DELETE FROM records WHERE source=?1 AND seq<=?2",
            params![source, sequence],
        )
        .map_err(io::Error::other)?;
        tx.execute(
            "UPDATE state SET ack=max(ack,?2) WHERE source=?1",
            params![source, sequence],
        )
        .map_err(io::Error::other)?;
        tx.commit().map_err(io::Error::other)
    }

    /// # Errors
    /// Returns unavailable journal errors.
    pub fn checkpoint(&self, key: &str) -> io::Result<Option<String>> {
        let db = self
            .db
            .lock()
            .map_err(|_| io::Error::other("journal lock"))?;
        db.query_row("SELECT value FROM checkpoints WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(io::Error::other)
    }

    /// Advance only after the corresponding records have been journaled.
    /// # Errors
    /// Returns unavailable journal errors.
    pub fn set_checkpoint(&self, key: &str, value: &str) -> io::Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|_| io::Error::other("journal lock"))?;
        db.execute("INSERT INTO checkpoints VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value]).map_err(io::Error::other)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_orchestrator_telemetry::v1::LogSource;
    fn record(data: &[u8]) -> TelemetryRecord {
        TelemetryRecord {
            source: Some(LogSource {
                id: "test".into(),
                ..Default::default()
            }),
            data: data.into(),
            ..Default::default()
        }
    }
    #[test]
    fn restart_replays_exact_bytes_and_does_not_deduplicate_equal_lines() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("journal");
        let spool = Spool::open(&path, 65536)?;
        spool.append(record(b"same\n"))?;
        spool.append(record(b"same\n"))?;
        spool.append(record(&[0, 255, 1]))?;
        drop(spool);
        let spool = Spool::open(&path, 65536)?;
        let records = spool.pending()?;
        assert_eq!(records.len(), 3);
        assert_eq!(records[2].data, [0, 255, 1]);
        assert!(spool.acknowledge("test", 4).is_err());
        spool.acknowledge("test", 2)?;
        assert_eq!(spool.pending()?.len(), 1);
        Ok(())
    }
    #[test]
    fn busy_sources_cannot_starve_inventory_or_other_producers() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let spool = Spool::open(&dir.path().join("journal"), 1024 * 1024)?;
        for index in 0..40 {
            for _ in 0..40 {
                let mut value = record(b"same\n");
                value
                    .source
                    .as_mut()
                    .ok_or_else(|| io::Error::other("source"))?
                    .id = format!("source-{index:02}");
                spool.append(value)?;
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..2 {
            let batch = spool.pending()?;
            assert!(batch.len() <= 32);
            seen.extend(batch.into_iter().filter_map(|r| r.source.map(|s| s.id)));
        }
        assert_eq!(seen.len(), 40);
        Ok(())
    }
    #[test]
    fn quota_loss_is_reported_after_delivery_recovers() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let spool = Spool::open(&dir.path().join("journal"), 1024)?;
        spool.append(record(b"first"))?;
        spool.append(record(&[42; 2048]))?;
        spool.acknowledge("test", 1)?;
        spool.append(record(b"recovered"))?;
        let records = spool.pending()?;
        assert_eq!(records[0].event, "capture_gap");
        assert_eq!(records[0].lost_bytes, 2048);
        assert_eq!(records[1].sequence, 3);
        Ok(())
    }
}
