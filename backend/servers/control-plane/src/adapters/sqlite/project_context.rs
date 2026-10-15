//! Append-only safe conversation context and replaceable, provenance-bearing summaries.
use super::{
    CleanStore, CleanStoreError, Connection, Message, OptionalExtension, decode, params, v1,
};
use serde_json::{Value, json};

pub(crate) const CAPACITY: u64 = 100_000;
pub(crate) const RECENT_MESSAGES: usize = 16;
const RECORD_BYTES: usize = 65_536;
const SCHEMA: &str = "CREATE TABLE project_context_records(conversation_id TEXT NOT NULL REFERENCES project_conversations(id),sequence INTEGER NOT NULL,id TEXT NOT NULL,kind TEXT NOT NULL,source_id TEXT NOT NULL,repository_id TEXT NOT NULL,role TEXT NOT NULL,content TEXT NOT NULL,original_payload BLOB NOT NULL,PRIMARY KEY(conversation_id,id),UNIQUE(conversation_id,sequence)); CREATE TABLE project_context_summaries(conversation_id TEXT PRIMARY KEY REFERENCES project_conversations(id),version INTEGER NOT NULL,source_watermark INTEGER NOT NULL,payload BLOB NOT NULL);";

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(CleanStoreError::Database)?;
    let result = (|| {
        connection
            .execute_batch(SCHEMA)
            .map_err(CleanStoreError::Database)?;
        let payloads = connection
            .prepare("SELECT payload FROM project_conversations")
            .map_err(CleanStoreError::Database)?
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        for payload in payloads {
            let mut c: v1::ProjectConversation = decode(&payload)?;
            archive(connection, &c)?;
            for proposal in c
                .proposals
                .iter()
                .filter(|p| p.state == "approved" && !p.attempt_id.is_empty())
            {
                let bytes: Option<Vec<u8>> = connection
                    .query_row(
                        "SELECT payload FROM development_attempts WHERE id=?1",
                        [&proposal.attempt_id],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(CleanStoreError::Database)?;
                if let Some(bytes) = bytes {
                    let attempt: v1::DevelopmentAttempt = decode(&bytes)?;
                    if super::is_terminal_attempt(attempt.state)
                        || attempt.requested_input.is_some()
                    {
                        archive_outcome(
                            connection,
                            &c,
                            &attempt,
                            &safe_outcome(&proposal.task_id, &attempt),
                        )?;
                    }
                }
            }
            compact(connection, &c.id)?;
            if c.messages.len() > RECENT_MESSAGES {
                c.messages.drain(..c.messages.len() - RECENT_MESSAGES);
                c.update_cursor = c
                    .update_cursor
                    .checked_add(1)
                    .ok_or(CleanStoreError::BoundExceeded)?;
                connection
                    .execute(
                        "UPDATE project_conversations SET payload=?1,cursor=?2 WHERE id=?3",
                        params![
                            c.encode_to_vec(),
                            i64::try_from(c.update_cursor)
                                .map_err(|_| CleanStoreError::BoundExceeded)?,
                            c.id
                        ],
                    )
                    .map_err(CleanStoreError::Database)?;
            }
        }
        connection
            .execute_batch("UPDATE clean_schema SET version=22 WHERE component='control_plane'")
            .map_err(CleanStoreError::Database)
    })();
    finish(connection, result)
}

fn append(
    connection: &Connection,
    c: &v1::ProjectConversation,
    id: &str,
    kind: &str,
    source: &str,
    repository: &str,
    role: &str,
    content: &str,
) -> Result<(), CleanStoreError> {
    append_record(
        connection,
        c,
        id,
        kind,
        source,
        repository,
        role,
        content,
        &[],
    )
}

fn append_record(
    connection: &Connection,
    c: &v1::ProjectConversation,
    id: &str,
    kind: &str,
    source: &str,
    repository: &str,
    role: &str,
    content: &str,
    original_payload: &[u8],
) -> Result<(), CleanStoreError> {
    if !repository.is_empty()
        && !c
            .repository_connection_ids
            .iter()
            .any(|id| id == repository)
    {
        return Err(CleanStoreError::ParentMismatch);
    }
    if content.len() > 32_768 {
        let mut remaining = content;
        let mut index = 0;
        while !remaining.is_empty() {
            let piece = crate::truncate_utf8(remaining, 32_768);
            let chunk_id = format!("{id}:part:{index}");
            append_record(
                connection,
                c,
                &chunk_id,
                kind,
                source,
                repository,
                role,
                &piece,
                if index == 0 { original_payload } else { &[] },
            )?;
            remaining = &remaining[piece.len()..];
            index += 1;
        }
        return Ok(());
    }
    let existing: Option<(String,String,String,String,String,Vec<u8>)> = connection
        .query_row(
            "SELECT kind,source_id,repository_id,role,content,original_payload FROM project_context_records WHERE conversation_id=?1 AND id=?2",
            params![c.id, id],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        )
        .optional()
        .map_err(CleanStoreError::Database)?;
    if let Some(existing) = existing {
        if existing
            != (
                kind.into(),
                source.into(),
                repository.into(),
                role.into(),
                content.into(),
                original_payload.to_vec(),
            )
        {
            return Err(CleanStoreError::StaleVersion);
        }
        return Ok(());
    }
    let sequence: u64 = connection.query_row("SELECT COALESCE(MAX(sequence),0)+1 FROM project_context_records WHERE conversation_id=?1",[&c.id],|r|r.get::<_,u32>(0)).map_err(CleanStoreError::Database).map(u64::from)?;
    if sequence > CAPACITY {
        return Err(CleanStoreError::ConversationCapacity);
    }
    connection
        .execute(
            "INSERT INTO project_context_records VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                c.id,
                i64::try_from(sequence).map_err(|_| CleanStoreError::BoundExceeded)?,
                id,
                kind,
                source,
                repository,
                role,
                content,
                original_payload
            ],
        )
        .map_err(CleanStoreError::Database)?;
    Ok(())
}

pub(super) fn archive(
    connection: &Connection,
    c: &v1::ProjectConversation,
) -> Result<(), CleanStoreError> {
    for m in &c.messages {
        append_record(
            connection,
            c,
            &format!("message:{}", m.id),
            "message",
            &m.id,
            "",
            &m.role,
            &m.content,
            &m.encode_to_vec(),
        )?;
    }
    if let Some(plan) = &c.plan {
        for v in &plan.versions {
            let id = format!("plan:{}:{}", plan.id, v.version);
            let content=json!({"version":v.version,"findings":v.findings,"scope":v.scope,"acceptance_criteria":v.acceptance_criteria,"dependency_environment_impact":v.dependency_environment_impact,"proposal_ids":v.proposal_ids}).to_string();
            append(
                connection,
                c,
                &id,
                "plan",
                &plan.id,
                "",
                "assistant",
                &content,
            )?;
            if v.state != "pending" {
                append(
                    connection,
                    c,
                    &format!("approval:{}:{}:{}", plan.id, v.version, v.state),
                    "approval",
                    &id,
                    "",
                    "operator",
                    &json!({"plan_id":plan.id,"version":v.version,"decision":v.state}).to_string(),
                )?;
            }
        }
    }
    for p in &c.proposals {
        let content=json!({"action":p.action,"state":p.state,"task_id":p.task_id,"attempt_id":p.attempt_id,"request":p.request.as_ref().map(|r|json!({"request":r.request,"acceptance_criteria":r.acceptance_criteria,"repository_id":r.repository_connection_id,"revision":r.repository_revision}))}).to_string();
        append(
            connection,
            c,
            &format!("proposal:{}:{}", p.id, p.state),
            "proposal",
            &p.id,
            p.request
                .as_ref()
                .map_or("", |r| r.repository_connection_id.as_str()),
            "assistant",
            &content,
        )?;
    }
    compact(connection, &c.id)
}

fn watermark(connection: &Connection, id: &str) -> Result<u64, CleanStoreError> {
    connection.query_row("SELECT COALESCE(MAX(sequence),0) FROM project_context_records WHERE conversation_id=?1",[id],|r|r.get::<_,i64>(0)).map_err(CleanStoreError::Database).and_then(|n|u64::try_from(n).map_err(|_|CleanStoreError::CorruptState))
}
fn summary(
    connection: &Connection,
    id: &str,
) -> Result<Option<v1::ProjectConversationSummary>, CleanStoreError> {
    let bytes: Option<Vec<u8>> = connection
        .query_row(
            "SELECT payload FROM project_context_summaries WHERE conversation_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(CleanStoreError::Database)?;
    bytes.map(|p| decode(&p)).transpose()
}
fn compact(connection: &Connection, id: &str) -> Result<(), CleanStoreError> {
    let source = watermark(connection, id)?;
    let previous = summary(connection, id)?;
    if previous
        .as_ref()
        .is_some_and(|s| s.source_watermark == source)
    {
        return Ok(());
    }
    let mut statement=connection.prepare("SELECT id,kind,content FROM project_context_records WHERE conversation_id=?1 AND sequence<=?2 ORDER BY CASE WHEN kind IN ('approval','plan','worker_outcome') THEN 0 ELSE 1 END,sequence DESC LIMIT 16").map_err(CleanStoreError::Database)?;
    let rows = statement
        .query_map(params![id, sqlite_number(source)?], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(CleanStoreError::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(CleanStoreError::Database)?;
    let version = previous
        .as_ref()
        .map_or(Some(1), |s| s.version.checked_add(1))
        .ok_or(CleanStoreError::BoundExceeded)?;
    let value = v1::ProjectConversationSummary {
        version,
        source_watermark: source,
        content: format!(
            "Extractive working index; excerpts are incomplete. Retrieve original records before relying on details.\n{}",
            rows.iter()
                .map(|(id, kind, content)| format!(
                    "[{id}] {kind}: {}",
                    crate::truncate_utf8(content, 512)
                ))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        source_record_ids: rows.into_iter().map(|r| r.0).collect(),
    };
    // The transaction and both predicates prevent stale or interrupted compaction replacing newer context.
    connection.execute("INSERT INTO project_context_summaries VALUES(?1,?2,?3,?4) ON CONFLICT(conversation_id) DO UPDATE SET version=excluded.version,source_watermark=excluded.source_watermark,payload=excluded.payload WHERE project_context_summaries.version=?5 AND project_context_summaries.source_watermark<excluded.source_watermark",params![id,sqlite_number(version)?,sqlite_number(source)?,value.encode_to_vec(),sqlite_number(previous.map_or(0,|s|s.version))?]).map_err(CleanStoreError::Database)?;
    Ok(())
}

impl CleanStore {
    fn context_scope(
        &self,
        project: &str,
        id: &str,
    ) -> Result<v1::ProjectConversation, CleanStoreError> {
        let c = self.project_conversation(id)?;
        if c.project_id != project {
            return Err(CleanStoreError::ParentMismatch);
        }
        self.project(project)?;
        Ok(c)
    }
    pub(crate) fn project_conversation_context(
        &self,
        project: &str,
        id: &str,
    ) -> Result<v1::GetProjectConversationContextResponse, CleanStoreError> {
        self.context_scope(project, id)?;
        let source = watermark(&self.connection, id)?;
        Ok(v1::GetProjectConversationContextResponse {
            summary: summary(&self.connection, id)?,
            source_watermark: source,
            archive_record_count: source,
            capacity_record_limit: CAPACITY,
            recent_message_limit: u32::try_from(RECENT_MESSAGES)
                .map_err(|_| CleanStoreError::CorruptState)?,
        })
    }
    pub(crate) fn project_conversation_history(
        &self,
        request: &v1::ListProjectConversationHistoryRequest,
    ) -> Result<v1::ListProjectConversationHistoryResponse, CleanStoreError> {
        let c = self.context_scope(&request.project_id, &request.conversation_id)?;
        if request.query.len() > 256 || request.limit > 100 {
            return Err(CleanStoreError::ContextBudget);
        }
        let source = watermark(&self.connection, &c.id)?;
        let snapshot = if request.snapshot_sequence == 0 {
            source
        } else {
            request.snapshot_sequence
        };
        if snapshot > source {
            return Err(CleanStoreError::StaleVersion);
        }
        let before = if request.before_sequence == 0 {
            snapshot + 1
        } else {
            request.before_sequence
        };
        let limit = if request.limit == 0 {
            50
        } else {
            request.limit
        };
        let mut statement=self.connection.prepare("SELECT sequence,id,kind,source_id,repository_id,role,content FROM project_context_records WHERE conversation_id=?1 AND sequence<=?2 AND sequence<?3 AND (?4='' OR instr(lower(content),lower(?4))>0 OR instr(lower(id),lower(?4))>0) ORDER BY sequence DESC LIMIT ?5").map_err(CleanStoreError::Database)?;
        let mut records = statement
            .query_map(
                params![
                    c.id,
                    sqlite_number(snapshot)?,
                    i64::try_from(before).map_err(|_| CleanStoreError::BoundExceeded)?,
                    request.query,
                    limit + 1
                ],
                |r| {
                    Ok(v1::ProjectContextRecord {
                        sequence: u64::from(r.get::<_, u32>(0)?),
                        id: r.get(1)?,
                        kind: r.get(2)?,
                        source_id: r.get(3)?,
                        repository_id: r.get(4)?,
                        role: r.get(5)?,
                        content: r.get(6)?,
                    })
                },
            )
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        let mut has_more = records.len() > limit as usize;
        records.truncate(limit as usize);
        let mut bytes = 0usize;
        let count = records
            .iter()
            .take_while(|r| {
                bytes += r.encoded_len() + 16;
                bytes <= 512 * 1024
            })
            .count();
        has_more |= count < records.len();
        records.truncate(count);
        Ok(v1::ListProjectConversationHistoryResponse {
            next_before_sequence: records.last().map_or(0, |r| r.sequence),
            records,
            snapshot_sequence: snapshot,
            has_more,
            archive_record_count: source,
        })
    }
    pub(crate) fn frozen_conversation_context(
        &self,
        c: &v1::ProjectConversation,
    ) -> Result<Value, CleanStoreError> {
        let context = self.project_conversation_context(&c.project_id, &c.id)?;
        let query = c
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map_or("", |m| m.content.as_str());
        let terms = query
            .split(|ch: char| !ch.is_alphanumeric() && ch != '-' && ch != '_')
            .filter(|t| t.len() > 3)
            .take(12)
            .map(|term| crate::truncate_utf8(term, 256))
            .collect::<Vec<_>>();
        let mut selected = std::collections::BTreeMap::new();
        let mut bytes = 0usize;
        for term in std::iter::once("").chain(terms.iter().map(String::as_str)) {
            let page =
                self.project_conversation_history(&v1::ListProjectConversationHistoryRequest {
                    project_id: c.project_id.clone(),
                    conversation_id: c.id.clone(),
                    query: term.into(),
                    limit: if term.is_empty() { 2 } else { 100 },
                    snapshot_sequence: context.source_watermark,
                    ..Default::default()
                })?;
            for record in page.records {
                if selected.contains_key(&record.sequence) {
                    continue;
                }
                let value = json!({"sequence":record.sequence,"id":record.id,"kind":record.kind,"source_id":record.source_id,"repository_id":record.repository_id,"role":record.role,"content":record.content});
                let size = value.to_string().len();
                if bytes + size > 90_000 || selected.len() >= 116 {
                    continue;
                }
                bytes += size;
                selected.insert(record.sequence, value);
            }
        }
        let complete = selected.len() as u64 == context.archive_record_count;
        let summary=context.summary.map(|s|json!({"version":s.version,"source_watermark":s.source_watermark,"content":s.content,"source_record_ids":s.source_record_ids}));
        Ok(
            json!({"conversation_id":c.id,"project_id":c.project_id,"repository_ids":c.repository_connection_ids,"source_watermark":context.source_watermark,"summary":summary,"records":selected.into_values().collect::<Vec<_>>(),"complete":complete,"archive_record_count":context.archive_record_count,"query":terms.join(" "),"capacity_record_limit":CAPACITY,"max_record_bytes":RECORD_BYTES}),
        )
    }
}
fn finish(
    connection: &Connection,
    result: Result<(), CleanStoreError>,
) -> Result<(), CleanStoreError> {
    match result {
        Ok(()) => connection
            .execute_batch("COMMIT")
            .map_err(CleanStoreError::Database),
        Err(e) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Deliberately excludes snapshots, provider bindings, secrets and environment data.
pub(crate) fn safe_outcome(task_id: &str, attempt: &v1::DevelopmentAttempt) -> Value {
    json!({"task_id":task_id,"attempt_id":attempt.id,"state":attempt.state,"requested_input":attempt.requested_input,"execution":attempt.result.as_ref().map(|r|r.execution),"publication":attempt.result.as_ref().map(|r|r.publication),"quality_state":attempt.result.as_ref().and_then(|r|r.quality.as_ref()).map(|q|q.outcome),"commit":attempt.result.as_ref().and_then(|r|r.commit.as_ref()),"quality_findings":attempt.result.as_ref().and_then(|r|r.quality.as_ref()).map(|q|&q.findings),"changed_paths":attempt.result.as_ref().map(|r|&r.changed_paths),"reporting_state":attempt.reporting_state,"cleanup_state":attempt.cleanup_state})
}
pub(super) fn archive_outcome(
    connection: &Connection,
    c: &v1::ProjectConversation,
    attempt: &v1::DevelopmentAttempt,
    outcome: &Value,
) -> Result<(), CleanStoreError> {
    use sha2::{Digest, Sha256};
    let (project, repository): (String, String) = connection
        .query_row(
            "SELECT project_id,repository_connection_id FROM development_tasks WHERE id=?1",
            [&attempt.development_task_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(CleanStoreError::Database)?;
    if project != c.project_id
        || !c.repository_connection_ids.contains(&repository)
        || outcome["task_id"].as_str() != Some(attempt.development_task_id.as_str())
    {
        return Err(CleanStoreError::ParentMismatch);
    }
    let content = outcome.to_string();
    append(
        connection,
        c,
        &format!(
            "outcome:{}:{:x}",
            attempt.id,
            Sha256::digest(content.as_bytes())
        ),
        "worker_outcome",
        &attempt.id,
        &repository,
        "system",
        &content,
    )
}

impl CleanStore {
    pub(super) fn validate_project_context(
        &self,
        c: &v1::ProjectConversation,
    ) -> Result<(), CleanStoreError> {
        let (count,max,oversized):(i64,i64,i64)=self.connection.query_row("SELECT COUNT(*),COALESCE(MAX(sequence),0),COALESCE(SUM(CASE WHEN length(CAST(content AS BLOB))>65536 OR sequence<1 OR source_id='' THEN 1 ELSE 0 END),0) FROM project_context_records WHERE conversation_id=?1",[&c.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(CleanStoreError::Database)?;
        let summary = summary(&self.connection, &c.id)?.ok_or(CleanStoreError::CorruptState)?;
        if count != max
            || max > sqlite_number(CAPACITY)?
            || oversized != 0
            || u64::try_from(max).ok() != Some(summary.source_watermark)
            || summary.version == 0
            || summary.version > summary.source_watermark + 1
            || summary.content.len() > 32_768
        {
            return Err(CleanStoreError::CorruptState);
        }
        for source in &summary.source_record_ids {
            let found:Option<i64>=self.connection.query_row("SELECT sequence FROM project_context_records WHERE conversation_id=?1 AND id=?2",params![c.id,source],|r|r.get(0)).optional().map_err(CleanStoreError::Database)?;
            if found.is_none_or(|s| u64::try_from(s).map_or(true, |s| s > summary.source_watermark))
            {
                return Err(CleanStoreError::CorruptState);
            }
        }
        Ok(())
    }
}

fn sqlite_number(number: u64) -> Result<i64, CleanStoreError> {
    i64::try_from(number).map_err(|_| CleanStoreError::BoundExceeded)
}
