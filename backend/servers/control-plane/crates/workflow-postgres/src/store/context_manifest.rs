use super::{StoreError, WorkflowStore, support};
use ayni_orchestrator_events::v1::{ConversationContextManifest, ConversationContextRecord};
use sha2::{Digest as _, Sha256};
use tokio_postgres::Transaction;

pub(super) const MAX_CONTEXT_RECORDS: i64 = 10_000;

/// A bounded page of immutable eligibility-time context, independent of watch retention.
#[derive(Clone, Debug)]
pub struct ContextManifestPage {
    pub manifest: ConversationContextManifest,
    pub records: Vec<ConversationContextRecord>,
    pub next_after_ordinal: u64,
}

pub(super) async fn reserve_admission(
    tx: &Transaction<'_>,
    conversation: &str,
    nonterminal: i64,
) -> Result<(), StoreError> {
    // The caller holds the conversation lock. Reserve one possible assistant
    // message for each predecessor so FIFO promotion cannot exceed the manifest
    // bound inside the predecessor's terminal-acceptance transaction.
    let messages:i64=tx.query_one("SELECT count(*) FROM (SELECT 1 FROM workflow_messages WHERE conversation_id=$1 LIMIT 10001) bounded", &[&conversation]).await?.get(0);
    if messages
        .checked_add(nonterminal)
        .and_then(|value| value.checked_add(1))
        .is_none_or(|maximum| maximum > MAX_CONTEXT_RECORDS)
    {
        return Err(StoreError::Capacity("context_manifest"));
    }
    Ok(())
}

pub(super) async fn freeze_if_configured(
    tx: &Transaction<'_>,
    command: &support::CommandMaterialization<'_>,
    configured: bool,
) -> Result<Option<ConversationContextManifest>, StoreError> {
    if !configured {
        return Ok(None);
    }
    freeze(
        tx,
        command.workspace_id,
        command.project_id,
        command.conversation_id,
        command.turn_id,
        command.context_revision,
    )
    .await
    .map(Some)
}

async fn freeze(
    tx: &Transaction<'_>,
    workspace: &str,
    project: &str,
    conversation: &str,
    turn: &str,
    revision: i64,
) -> Result<ConversationContextManifest, StoreError> {
    let mut last_turn = 0_i64;
    let mut last_role = 0_i64;
    let mut last_sequence = 0_i64;
    let mut count = 0_i64;
    let mut digest = Sha256::new();
    digest.update(b"ayni-conversation-context-v1");
    loop {
        let rows=tx.query("SELECT m.id,m.sequence,m.role,m.content,t.sequence,CASE m.role WHEN 'user' THEN 1 ELSE 2 END::bigint AS role_order FROM workflow_messages m JOIN workflow_turns t ON t.id=m.turn_id JOIN workflow_turns target ON target.id=$2 WHERE m.conversation_id=$1 AND t.sequence<=target.sequence AND (t.sequence,CASE m.role WHEN 'user' THEN 1 ELSE 2 END::bigint,m.sequence)>($3,$4,$5) ORDER BY t.sequence,role_order,m.sequence LIMIT 100", &[&conversation,&turn,&last_turn,&last_role,&last_sequence]).await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            count = count
                .checked_add(1)
                .ok_or(StoreError::Capacity("context_manifest"))?;
            if count > MAX_CONTEXT_RECORDS {
                return Err(StoreError::Capacity("context_manifest"));
            }
            let id: String = row.get(0);
            let sequence: i64 = row.get(1);
            let role: String = row.get(2);
            let content: String = row.get(3);
            for field in [
                id.as_bytes(),
                &sequence.to_be_bytes(),
                role.as_bytes(),
                content.as_bytes(),
            ] {
                digest.update(
                    u64::try_from(field.len())
                        .map_err(|_| StoreError::CorruptState)?
                        .to_be_bytes(),
                );
                digest.update(field);
            }
            tx.execute("INSERT INTO workflow_context_manifest_messages(turn_id,ordinal,conversation_id,message_id) VALUES ($1,$2,$3,$4)", &[&turn,&count,&conversation,&id]).await?;
            last_turn = row.get(4);
            last_role = row.get(5);
            last_sequence = sequence;
        }
    }
    if count == 0 {
        return Err(StoreError::CorruptState);
    }
    let digest = format!("sha256:{:x}", digest.finalize());
    tx.execute("INSERT INTO workflow_context_manifests(turn_id,workspace_id,project_id,conversation_id,context_revision,record_count,digest) VALUES ($1,$2,$3,$4,$5,$6,$7)", &[&turn,&workspace,&project,&conversation,&revision,&count,&digest]).await?;
    Ok(ConversationContextManifest {
        id: turn.to_owned(),
        digest,
        record_count: support::unsigned(count)?,
        inline_complete: false,
    })
}

impl WorkflowStore {
    /// Page the immutable context selected at eligibility, scoped to the owning turn.
    ///
    /// # Errors
    /// Returns invalid bounds, missing scoped manifest, or unavailable authority.
    pub async fn get_turn_context_inputs(
        &self,
        workspace: &str,
        project: &str,
        conversation: &str,
        turn: &str,
        after: u64,
        limit: u32,
    ) -> Result<ContextManifestPage, StoreError> {
        support::validate_scope(workspace, project, conversation)?;
        support::validate_id(turn, "turn_id")?;
        if limit > 100 {
            return Err(StoreError::InvalidInput("limit"));
        }
        let after = i64::try_from(after).map_err(|_| StoreError::InvalidInput("after_ordinal"))?;
        let limit = i64::from(if limit == 0 { 50 } else { limit });
        let (client, driver) = crate::connect(&self.database_url).await?;
        let row=client.query_opt("SELECT record_count,digest FROM workflow_context_manifests WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND turn_id=$4", &[&workspace,&project,&conversation,&turn]).await?.ok_or(StoreError::NotFound)?;
        let count: i64 = row.get(0);
        let digest: String = row.get(1);
        let rows=client.query("SELECT m.id,m.sequence,m.role,m.content,refs.ordinal FROM workflow_context_manifest_messages refs JOIN workflow_messages m ON m.conversation_id=refs.conversation_id AND m.id=refs.message_id WHERE refs.turn_id=$1 AND refs.ordinal>$2 ORDER BY refs.ordinal LIMIT $3", &[&turn,&after,&limit]).await?;
        let mut next = after;
        let mut bytes = 0_usize;
        let mut records = Vec::new();
        // Keep even escaped ProtoJSON comfortably below the canonical response bound.
        for row in rows {
            let record = ConversationContextRecord {
                message_id: row.get(0),
                sequence: support::unsigned(row.get(1))?,
                role: row.get(2),
                content: row.get(3),
            };
            let size = record.content.len() + record.message_id.len() + record.role.len() + 64;
            if bytes + size > 64 * 1024 {
                break;
            }
            bytes += size;
            next = row.get(4);
            records.push(record);
        }
        driver.abort();
        Ok(ContextManifestPage {
            manifest: ConversationContextManifest {
                id: turn.to_owned(),
                digest,
                record_count: support::unsigned(count)?,
                inline_complete: false,
            },
            records,
            next_after_ordinal: if next < count {
                support::unsigned(next)?
            } else {
                0
            },
        })
    }
}
