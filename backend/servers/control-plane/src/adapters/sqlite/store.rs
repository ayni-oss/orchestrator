use super::*;

impl CleanStore {
    pub(super) fn event_watermark(&self) -> Result<i64, CleanStoreError> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(cursor),0) FROM resource_events",
                [],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)
    }

    pub(crate) fn open(path: impl AsRef<Path>, max_attempts: u32) -> Result<Self, CleanStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(CleanStoreError::CreateDirectory)?;
        }
        let connection = Connection::open(path).map_err(CleanStoreError::Database)?;
        Self::from_connection(connection, max_attempts)
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Result<Self, CleanStoreError> {
        Self::from_connection(
            Connection::open_in_memory().map_err(CleanStoreError::Database)?,
            3,
        )
    }

    pub(super) fn from_connection(
        connection: Connection,
        max_attempts: u32,
    ) -> Result<Self, CleanStoreError> {
        if max_attempts == 0 {
            return Err(CleanStoreError::InvalidTransition);
        }
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;",
            )
            .map_err(CleanStoreError::Database)?;
        initialize_schema(&connection)?;
        let store = Self {
            connection,
            max_attempts,
            coding_turns: 24,
            quality_repair_limit: 3,
        };
        store.integrity_check()?;
        store.foreign_key_check()?;
        store.validate_durable_state()?;
        Ok(store)
    }

    #[cfg(test)]
    pub(crate) fn schema_version(&self) -> Result<i64, CleanStoreError> {
        self.connection
            .query_row(
                "SELECT version FROM clean_schema WHERE component='control_plane'",
                [],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)
    }

    pub(crate) fn integrity_check(&self) -> Result<(), CleanStoreError> {
        let result: String = self
            .connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(CleanStoreError::Database)?;
        if result == "ok" {
            Ok(())
        } else {
            Err(CleanStoreError::CorruptState)
        }
    }

    pub(super) fn foreign_key_check(&self) -> Result<(), CleanStoreError> {
        let mut statement = self
            .connection
            .prepare("PRAGMA foreign_key_check")
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        if rows.next().map_err(CleanStoreError::Database)?.is_some() {
            Err(CleanStoreError::CorruptState)
        } else {
            Ok(())
        }
    }

    pub(super) fn validate_durable_state(&self) -> Result<(), CleanStoreError> {
        self.validate_verification_inbox()?;
        validate_resource_tables(&self.connection)?;
        validate_durable_actions(&self.connection)?;
        validate_resource_events(&self.connection)?;
        validate_page_snapshot_rows(&self.connection)?;
        Ok(())
    }

    pub(crate) fn current_cursor(&self, service: &str) -> Result<u64, CleanStoreError> {
        let value: i64 = self
            .connection
            .query_row(
                "SELECT COALESCE(MAX(cursor), 0) FROM resource_events WHERE service=?1",
                [service],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        u64::try_from(value).map_err(|_| CleanStoreError::CorruptState)
    }

    pub(crate) fn initial_events(
        &self,
        service: &str,
        repository_connection_id: Option<&str>,
        parent_id: Option<&str>,
    ) -> Result<(Vec<StoredEvent>, u64), CleanStoreError> {
        let (sql, parameters): (&str, Vec<Option<&str>>) = match service {
            REPOSITORY_SERVICE => (
                "SELECT event_cursor,'repository_connection',payload FROM repository_connections
                 WHERE (?1 IS NULL OR id=?1)
                 UNION ALL
                 SELECT event_cursor,'repository_snapshot',payload FROM repository_snapshots
                 WHERE (?1 IS NULL OR repository_connection_id=?1)
                 ORDER BY 1",
                vec![repository_connection_id],
            ),
            PROFILE_SERVICE => (
                "SELECT event_cursor,'runtime_profile',payload FROM runtime_profiles
                 WHERE (?1 IS NULL OR repository_connection_id=?1)
                   AND (?2 IS NULL OR id=?2)
                 UNION ALL
                 SELECT r.event_cursor,'runtime_profile_revision',r.payload
                 FROM runtime_profile_revisions r JOIN runtime_profiles p ON p.id=r.runtime_profile_id
                 WHERE (?1 IS NULL OR p.repository_connection_id=?1)
                   AND (?2 IS NULL OR p.id=?2)
                 UNION ALL
                 SELECT a.event_cursor,'runtime_artifact',a.payload
                 FROM runtime_artifacts a JOIN runtime_profiles p ON p.id=a.runtime_profile_id
                 WHERE (?1 IS NULL OR p.repository_connection_id=?1)
                   AND (?2 IS NULL OR p.id=?2)
                 ORDER BY 1",
                vec![repository_connection_id, parent_id],
            ),
            DEVELOPMENT_SERVICE => (
                "SELECT event_cursor,'development_task',payload FROM development_tasks WHERE id=?1
                 UNION ALL
                 SELECT event_cursor,'development_attempt',payload FROM development_attempts
                 WHERE development_task_id=?1
                 ORDER BY 1",
                vec![parent_id],
            ),
            _ => return Err(CleanStoreError::CorruptState),
        };
        let mut statement = self
            .connection
            .prepare(sql)
            .map_err(CleanStoreError::Database)?;
        let mut rows = match parameters.as_slice() {
            [first] => statement
                .query(params![first])
                .map_err(CleanStoreError::Database)?,
            [first, second] => statement
                .query(params![first, second])
                .map_err(CleanStoreError::Database)?,
            _ => return Err(CleanStoreError::CorruptState),
        };
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
            let cursor: i64 = row.get(0).map_err(CleanStoreError::Database)?;
            events.push(StoredEvent {
                cursor: u64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                resource_type: row.get(1).map_err(CleanStoreError::Database)?,
                payload: row.get(2).map_err(CleanStoreError::Database)?,
            });
        }
        let high_water = self.current_cursor(service)?;
        Ok((events, high_water))
    }

    pub(crate) fn events_after(
        &self,
        service: &str,
        after_cursor: u64,
        repository_connection_id: Option<&str>,
        parent_id: Option<&str>,
    ) -> Result<Vec<StoredEvent>, CleanStoreError> {
        let after = i64::try_from(after_cursor).map_err(|_| CleanStoreError::WatchCursorExpired)?;
        let watermark: i64 = self
            .connection
            .query_row(
                "SELECT first_cursor FROM event_watermarks WHERE service=?1",
                [service],
                |row| row.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?
            .unwrap_or(0);
        let high_water: i64 = self
            .connection
            .query_row(
                "SELECT COALESCE(MAX(cursor),0) FROM resource_events WHERE service=?1",
                [service],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        if after < watermark.saturating_sub(1) || after > high_water {
            return Err(CleanStoreError::WatchCursorExpired);
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT cursor, resource_type, payload
                 FROM resource_events
                 WHERE service=?1 AND cursor>?2
                   AND (?3 IS NULL OR repository_connection_id=?3)
                   AND (?4 IS NULL OR parent_id=?4)
                 ORDER BY cursor ASC LIMIT 256",
            )
            .map_err(CleanStoreError::Database)?;
        statement
            .query_map(
                params![service, after, repository_connection_id, parent_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .map_err(CleanStoreError::Database)?
            .map(|row| {
                let (cursor, resource_type, payload) = row.map_err(CleanStoreError::Database)?;
                Ok(StoredEvent {
                    cursor: u64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                    resource_type,
                    payload,
                })
            })
            .collect()
    }

    pub(crate) fn durable_action(
        &self,
        operation: &str,
        key: &str,
    ) -> Result<Option<DurableAction>, CleanStoreError> {
        self.connection
            .query_row(
                "SELECT state, target_id, request_payload FROM durable_actions
                 WHERE operation=?1 AND idempotency_key=?2",
                params![operation, key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(CleanStoreError::Database)?
            .map(|(state, _target_id, request_payload)| {
                let state = match state.as_str() {
                    "pending" => DurableActionState::Pending,
                    "completed" => DurableActionState::Completed,
                    "failed" => DurableActionState::Failed,
                    _ => return Err(CleanStoreError::CorruptState),
                };
                Ok(DurableAction {
                    idempotency_key: key.to_owned(),
                    state,
                    request_payload,
                })
            })
            .transpose()
    }

    pub(crate) fn pending_external_actions(
        &self,
        operation: &str,
    ) -> Result<Vec<DurableAction>, CleanStoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT idempotency_key,target_id,request_payload
                 FROM durable_actions WHERE operation=?1 AND state='pending'
                 ORDER BY created_at,idempotency_key LIMIT ?2",
            )
            .map_err(CleanStoreError::Database)?;
        let actions = statement
            .query_map(
                params![
                    operation,
                    i64::try_from(MAX_PENDING_ACTION_BATCH)
                        .map_err(|_| CleanStoreError::CorruptState)?
                ],
                |row| {
                    Ok(DurableAction {
                        idempotency_key: row.get(0)?,
                        state: DurableActionState::Pending,
                        request_payload: row.get(2)?,
                    })
                },
            )
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        Ok(actions)
    }

    pub(crate) fn begin_external_action<M: Message>(
        &mut self,
        operation: &str,
        key: &str,
        request: &M,
        target_type: &str,
        target_id: &str,
    ) -> Result<DurableAction, CleanStoreError> {
        let digest = request_digest(operation, request);
        if let Some(existing) = action_row(&self.connection, operation, key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return Ok(existing.into_action(key));
        }
        let now = unix_seconds()?;
        let request_payload = request.encode_to_vec();
        self.connection
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,
                    created_at,updated_at
                 ) VALUES (?1,?2,?3,?4,'pending',?5,?6,?7,?7)",
                params![
                    operation,
                    key,
                    digest,
                    request_payload,
                    target_type,
                    target_id,
                    now
                ],
            )
            .map_err(CleanStoreError::Database)?;
        Ok(DurableAction {
            idempotency_key: key.to_owned(),
            state: DurableActionState::Pending,
            request_payload,
        })
    }

    pub(crate) fn complete_external_action(
        &mut self,
        operation: &str,
        key: &str,
    ) -> Result<(), CleanStoreError> {
        let changed = self
            .connection
            .execute(
                "UPDATE durable_actions SET state='completed', failure=NULL, updated_at=?3
                 WHERE operation=?1 AND idempotency_key=?2 AND state='pending'",
                params![operation, key, unix_seconds()?],
            )
            .map_err(CleanStoreError::Database)?;
        if changed == 0 {
            let action = self.durable_action(operation, key)?;
            if action.is_some_and(|value| value.state == DurableActionState::Completed) {
                Ok(())
            } else {
                Err(CleanStoreError::InvalidTransition)
            }
        } else {
            Ok(())
        }
    }

    pub(crate) fn fail_external_action(
        &mut self,
        operation: &str,
        key: &str,
        failure: &str,
    ) -> Result<(), CleanStoreError> {
        let failure = super::payloads::truncate_utf8(failure, 512);
        let changed = self
            .connection
            .execute(
                "UPDATE durable_actions SET state='failed', failure=?3, updated_at=?4
                 WHERE operation=?1 AND idempotency_key=?2 AND state='pending'",
                params![operation, key, failure, unix_seconds()?],
            )
            .map_err(CleanStoreError::Database)?;
        if changed == 1 {
            return Ok(());
        }
        let recorded_failure: Option<String> = self.connection.query_row(
            "SELECT failure FROM durable_actions WHERE operation=?1 AND idempotency_key=?2 AND state='failed'",
            params![operation, key], |row| row.get(0),
        ).optional().map_err(CleanStoreError::Database)?.flatten();
        if recorded_failure.as_deref() == Some(failure.as_str()) {
            Ok(())
        } else {
            Err(CleanStoreError::InvalidTransition)
        }
    }

    #[cfg(test)]
    pub(crate) fn requeue_external_action(
        &mut self,
        operation: &str,
        key: &str,
    ) -> Result<(), CleanStoreError> {
        let changed = self
            .connection
            .execute(
                "UPDATE durable_actions SET state='pending', failure=NULL, updated_at=?3
                 WHERE operation=?1 AND idempotency_key=?2 AND state='failed'",
                params![operation, key, unix_seconds()?],
            )
            .map_err(CleanStoreError::Database)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(CleanStoreError::InvalidTransition)
        }
    }

    pub(super) fn page_messages<M: Message + Default>(
        &mut self,
        method: &str,
        filter_hash: &str,
        page: Option<&v1::PageRequest>,
        fresh_items: impl FnOnce(&Connection) -> Result<Vec<Vec<u8>>, CleanStoreError>,
        service: &str,
    ) -> Result<Page<M>, CleanStoreError> {
        let size = page_size(page);
        let token = page.map_or("", |value| value.page_token.as_str());
        if token.is_empty() {
            let items = fresh_items(&self.connection)?;
            let cursor = self.current_cursor(service)?;
            return self.materialize_page(method, filter_hash, cursor, items, 0, size);
        }
        let page_token = decode_page_token(token)?;
        let row = self
            .connection
            .query_row(
                "SELECT method,filter_hash,cursor,items,token_secret,created_at
                 FROM page_snapshots WHERE token=?1",
                [&page_token.snapshot],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(CleanStoreError::Database)?
            .ok_or(CleanStoreError::InvalidPageToken)?;
        let now = unix_seconds()?;
        if row.0 != method
            || row.1 != filter_hash
            || now.saturating_sub(row.5) > PAGE_LIFETIME_SECONDS
            || page_token.proof != page_token_proof(&row.4, &page_token.snapshot, page_token.offset)
        {
            return Err(CleanStoreError::InvalidPageToken);
        }
        let encoded: Vec<String> =
            serde_json::from_str(&row.3).map_err(|_| CleanStoreError::CorruptState)?;
        let items = encoded
            .into_iter()
            .map(|item| {
                URL_SAFE_NO_PAD
                    .decode(item)
                    .map_err(|_| CleanStoreError::CorruptState)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.materialize_existing_page(
            &page_token.snapshot,
            &row.4,
            u64::try_from(row.2).map_err(|_| CleanStoreError::CorruptState)?,
            items,
            page_token.offset,
            size,
        )
    }

    pub(super) fn materialize_page<M: Message + Default>(
        &mut self,
        method: &str,
        filter_hash: &str,
        cursor: u64,
        items: Vec<Vec<u8>>,
        offset: usize,
        size: usize,
    ) -> Result<Page<M>, CleanStoreError> {
        if items.len() > MAX_PAGE_SNAPSHOT_ITEMS
            || items
                .iter()
                .try_fold(0_usize, |total, item| {
                    total.checked_add(item.len()).ok_or(())
                })
                .map_or(true, |total| total > MAX_PAGE_SNAPSHOT_BYTES)
        {
            return Err(CleanStoreError::BoundExceeded);
        }
        let fits_one_page = items.len() <= size
            && items.iter().map(Vec::len).sum::<usize>() <= MAX_PAGE_RESPONSE_BYTES;
        if fits_one_page {
            let values = items
                .into_iter()
                .map(|value| decode(&value))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Page {
                items: values,
                next_page_token: String::new(),
                snapshot_cursor: cursor,
            });
        }
        self.prune_page_snapshots()?;
        let token = Uuid::now_v7().to_string();
        let secret = Uuid::now_v7().to_string();
        let encoded = items
            .iter()
            .map(|value| URL_SAFE_NO_PAD.encode(value))
            .collect::<Vec<_>>();
        let encoded = serde_json::to_string(&encoded).map_err(|_| CleanStoreError::CorruptState)?;
        if encoded.len() > MAX_PAGE_SNAPSHOT_BYTES.saturating_mul(2) {
            return Err(CleanStoreError::BoundExceeded);
        }
        self.connection
            .execute(
                "INSERT INTO page_snapshots(token,method,filter_hash,cursor,items,token_secret,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    token,
                    method,
                    filter_hash,
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                    encoded,
                    secret,
                    unix_seconds()?
                ],
            )
            .map_err(CleanStoreError::Database)?;
        self.materialize_existing_page(&token, &secret, cursor, items, offset, size)
    }

    pub(super) fn materialize_existing_page<M: Message + Default>(
        &self,
        token: &str,
        secret: &str,
        cursor: u64,
        items: Vec<Vec<u8>>,
        offset: usize,
        size: usize,
    ) -> Result<Page<M>, CleanStoreError> {
        if offset > items.len() {
            return Err(CleanStoreError::InvalidPageToken);
        }
        let mut end = offset;
        let mut bytes = 0_usize;
        while end < items.len() && end.saturating_sub(offset) < size {
            let next = bytes
                .checked_add(items[end].len())
                .ok_or(CleanStoreError::BoundExceeded)?;
            if next > MAX_PAGE_RESPONSE_BYTES {
                break;
            }
            bytes = next;
            end = end.saturating_add(1);
        }
        if end == offset && offset < items.len() {
            return Err(CleanStoreError::BoundExceeded);
        }
        let values = items[offset..end]
            .iter()
            .map(|value| decode(value))
            .collect::<Result<Vec<_>, _>>()?;
        let next_page_token = if end < items.len() {
            encode_page_token(token, secret, end)?
        } else {
            String::new()
        };
        Ok(Page {
            items: values,
            next_page_token,
            snapshot_cursor: cursor,
        })
    }

    pub(super) fn prune_page_snapshots(&mut self) -> Result<(), CleanStoreError> {
        let now = unix_seconds()?;
        self.connection
            .execute(
                "DELETE FROM page_snapshots WHERE created_at<?1",
                [now.saturating_sub(PAGE_LIFETIME_SECONDS)],
            )
            .map_err(CleanStoreError::Database)?;
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM page_snapshots", [], |row| row.get(0))
            .map_err(CleanStoreError::Database)?;
        let excess = usize::try_from(count)
            .unwrap_or(usize::MAX)
            .saturating_sub(MAX_PAGE_SNAPSHOTS.saturating_sub(1));
        if excess > 0 {
            self.connection
                .execute(
                    "DELETE FROM page_snapshots WHERE token IN (
                        SELECT token FROM page_snapshots ORDER BY created_at ASC LIMIT ?1
                     )",
                    [i64::try_from(excess).map_err(|_| CleanStoreError::CorruptState)?],
                )
                .map_err(CleanStoreError::Database)?;
        }
        Ok(())
    }
}
