use super::*;

pub(super) fn provider_key(provider: &v1::RepositoryProvider) -> Result<String, CleanStoreError> {
    match provider.provider.as_ref() {
        Some(v1::repository_provider::Provider::Github(github)) => {
            Ok(format!("github:{}", github.repository_id))
        }
        None => Err(CleanStoreError::CorruptState),
    }
}

pub(super) fn page_items_decode(method: &str, items: &[Vec<u8>]) -> bool {
    macro_rules! all_decode {
        ($message:ty) => {
            items.iter().all(|item| {
                <$message>::decode(item.as_slice())
                    .is_ok_and(|value| !value.id.is_empty() && value.resource_version > 0)
            })
        };
    }
    match method {
        "ListRepositoryConnections" => all_decode!(v1::RepositoryConnection),
        "ListRepositorySnapshots" => all_decode!(v1::RepositorySnapshot),
        "ListProjects" => all_decode!(v1::Project),
        "ListRuntimeProfiles" => all_decode!(v1::RuntimeProfile),
        "ListRuntimeProfileRevisions" => all_decode!(v1::RuntimeProfileRevision),
        "ListRuntimeArtifacts" => all_decode!(v1::RuntimeArtifact),
        "ListDevelopmentTasks" => all_decode!(v1::DevelopmentTask),
        "ListDevelopmentAttempts" => all_decode!(v1::DevelopmentAttempt),
        _ => false,
    }
}

pub(super) fn encode_page_token(
    snapshot: &str,
    secret: &str,
    offset: usize,
) -> Result<String, CleanStoreError> {
    let token = PageToken {
        snapshot: snapshot.to_owned(),
        offset,
        proof: page_token_proof(secret, snapshot, offset),
    };
    let json = serde_json::to_vec(&token).map_err(|_| CleanStoreError::CorruptState)?;
    Ok(URL_SAFE_NO_PAD.encode(json))
}

pub(super) fn decode_page_token(token: &str) -> Result<PageToken, CleanStoreError> {
    let json = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| CleanStoreError::InvalidPageToken)?;
    serde_json::from_slice(&json).map_err(|_| CleanStoreError::InvalidPageToken)
}

pub(super) fn page_token_proof(secret: &str, snapshot: &str, offset: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"ayni.orchestrator.page.v1\0");
    hasher.update(secret.as_bytes());
    hasher.update([0]);
    hasher.update(snapshot.as_bytes());
    hasher.update([0]);
    hasher.update(offset.to_string().as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

pub(super) fn encode_filter(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

pub(super) fn page_size(page: Option<&v1::PageRequest>) -> usize {
    page.map_or(DEFAULT_PAGE_SIZE, |value| {
        if value.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            usize::try_from(value.page_size)
                .unwrap_or(MAX_PAGE_SIZE)
                .min(MAX_PAGE_SIZE)
        }
    })
}

pub(super) fn append_event<M: Message>(
    transaction: &Transaction<'_>,
    service: &str,
    resource_type: &str,
    resource_id: &str,
    resource_version: u64,
    repository_connection_id: Option<&str>,
    parent_id: Option<&str>,
    value: &M,
) -> Result<u64, CleanStoreError> {
    transaction
        .execute(
            "INSERT INTO resource_events(
                service,resource_type,resource_id,resource_version,
                repository_connection_id,parent_id,payload,created_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                service,
                resource_type,
                resource_id,
                i64::try_from(resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                repository_connection_id,
                parent_id,
                value.encode_to_vec(),
                unix_seconds()?
            ],
        )
        .map_err(CleanStoreError::Database)?;
    let cursor = transaction.last_insert_rowid();
    prune_events(transaction, service)?;
    u64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)
}

pub(super) fn prune_events(
    transaction: &Transaction<'_>,
    service: &str,
) -> Result<(), CleanStoreError> {
    let offset =
        i64::try_from(MAX_EVENTS_PER_SERVICE).map_err(|_| CleanStoreError::CorruptState)?;
    let cutoff: Option<i64> = transaction
        .query_row(
            "SELECT cursor FROM resource_events WHERE service=?1
             ORDER BY cursor DESC LIMIT 1 OFFSET ?2",
            params![service, offset],
            |row| row.get(0),
        )
        .optional()
        .map_err(CleanStoreError::Database)?;
    if let Some(cutoff) = cutoff {
        transaction
            .execute(
                "DELETE FROM resource_events WHERE service=?1 AND cursor<=?2",
                params![service, cutoff],
            )
            .map_err(CleanStoreError::Database)?;
    }
    let first: i64 = transaction
        .query_row(
            "SELECT COALESCE(MIN(cursor),0) FROM resource_events WHERE service=?1",
            [service],
            |row| row.get(0),
        )
        .map_err(CleanStoreError::Database)?;
    transaction
        .execute(
            "INSERT INTO event_watermarks(service,first_cursor) VALUES (?1,?2)
             ON CONFLICT(service) DO UPDATE SET first_cursor=excluded.first_cursor",
            params![service, first],
        )
        .map_err(CleanStoreError::Database)?;
    Ok(())
}
