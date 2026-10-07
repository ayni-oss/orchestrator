use super::*;

pub(super) fn validate_indexed_rows<M, F>(
    connection: &Connection,
    sql: &str,
    validate: F,
) -> Result<(), CleanStoreError>
where
    M: Message + Default,
    F: Fn(&rusqlite::Row<'_>, &M) -> rusqlite::Result<bool>,
{
    let mut statement = connection.prepare(sql).map_err(CleanStoreError::Database)?;
    let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
    while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
        let payload: Vec<u8> = row.get(0).map_err(CleanStoreError::Database)?;
        let value = decode::<M>(&payload)?;
        if !validate(row, &value).map_err(CleanStoreError::Database)? {
            return Err(CleanStoreError::CorruptState);
        }
    }
    Ok(())
}

pub(super) fn valid_version_and_cursor(version: u64, indexed_version: i64, cursor: i64) -> bool {
    version > 0 && i64::try_from(version) == Ok(indexed_version) && cursor > 0
}

pub(super) fn validate_resource_tables(connection: &Connection) -> Result<(), CleanStoreError> {
    validate_projects(connection)?;
    validate_indexed_rows::<v1::RepositoryConnection, _>(
        connection,
        "SELECT payload,id,status,resource_version,event_cursor FROM repository_connections",
        |row, value| {
            Ok(!value.id.is_empty()
                && value.provider.is_some()
                && value.status != v1::RepositoryConnectionStatus::Unspecified as i32
                && v1::RepositoryConnectionStatus::try_from(value.status).is_ok()
                && value.id == row.get::<_, String>(1)?
                && value.status == row.get::<_, i32>(2)?
                && valid_version_and_cursor(value.resource_version, row.get(3)?, row.get(4)?))
        },
    )?;
    validate_indexed_rows::<v1::RepositorySnapshot, _>(
        connection,
        "SELECT payload,id,repository_connection_id,revision,resource_version,event_cursor
         FROM repository_snapshots",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.repository_connection_id.is_empty()
                && !value.revision.is_empty()
                && value.id == row.get::<_, String>(1)?
                && value.repository_connection_id == row.get::<_, String>(2)?
                && value.revision == row.get::<_, String>(3)?
                && valid_version_and_cursor(value.resource_version, row.get(4)?, row.get(5)?))
        },
    )?;
    validate_indexed_rows::<v1::RuntimeProfile, _>(
        connection,
        "SELECT payload,id,repository_connection_id,kind,state,active_revision_id,
                active_runtime_artifact_id,resource_version,event_cursor FROM runtime_profiles",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.repository_connection_id.is_empty()
                && value.kind != v1::RuntimeProfileKind::Unspecified as i32
                && value.support != v1::RuntimeProfileSupport::Unspecified as i32
                && value.state != v1::RuntimeProfileState::Unspecified as i32
                && safe_metadata(&value.reconciliation_metadata, 16, 8192)
                && v1::RuntimeProfileKind::try_from(value.kind).is_ok()
                && v1::RuntimeProfileSupport::try_from(value.support).is_ok()
                && v1::RuntimeProfileState::try_from(value.state).is_ok()
                && value.id == row.get::<_, String>(1)?
                && value.repository_connection_id == row.get::<_, String>(2)?
                && value.kind == row.get::<_, i32>(3)?
                && value.state == row.get::<_, i32>(4)?
                && value.active_revision_id == row.get::<_, Option<String>>(5)?
                && value.active_runtime_artifact_id == row.get::<_, Option<String>>(6)?
                && valid_version_and_cursor(value.resource_version, row.get(7)?, row.get(8)?))
        },
    )?;
    validate_indexed_rows::<v1::RuntimeProfileRevision, _>(
        connection,
        "SELECT payload,id,runtime_profile_id,repository_snapshot_id,revision_number,
                resource_version,event_cursor FROM runtime_profile_revisions",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.runtime_profile_id.is_empty()
                && !value.repository_snapshot_id.is_empty()
                && value.revision_number > 0
                && value.configuration.is_some()
                && value.id == row.get::<_, String>(1)?
                && value.runtime_profile_id == row.get::<_, String>(2)?
                && value.repository_snapshot_id == row.get::<_, String>(3)?
                && i64::try_from(value.revision_number) == Ok(row.get::<_, i64>(4)?)
                && valid_version_and_cursor(value.resource_version, row.get(5)?, row.get(6)?))
        },
    )?;
    validate_indexed_rows::<v1::RuntimeArtifact, _>(
        connection,
        "SELECT payload,id,runtime_profile_id,runtime_profile_revision_id,status,resolved_digest,
                resource_version,event_cursor FROM runtime_artifacts",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.runtime_profile_revision_id.is_empty()
                && !value.resolved_digest.is_empty()
                && value.platform.is_some()
                && value.status != v1::RuntimeArtifactStatus::Unspecified as i32
                && v1::RuntimeArtifactStatus::try_from(value.status).is_ok()
                && valid_artifact_report(value)
                && value.id == row.get::<_, String>(1)?
                && value.runtime_profile_revision_id == row.get::<_, String>(3)?
                && value.status == row.get::<_, i32>(4)?
                && value.resolved_digest == row.get::<_, String>(5)?
                && valid_version_and_cursor(value.resource_version, row.get(6)?, row.get(7)?)
                && !row.get::<_, String>(2)?.is_empty())
        },
    )?;
    validate_indexed_rows::<v1::AutomationInstallation, _>(
        connection,
        "SELECT payload,id,runtime_profile_id,state,version,resource_version,event_cursor
         FROM automation_installations",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.runtime_profile_id.is_empty()
                && !value.version.is_empty()
                && value.state != v1::AutomationInstallationState::Unspecified as i32
                && v1::AutomationInstallationState::try_from(value.state).is_ok()
                && value.id == row.get::<_, String>(1)?
                && value.runtime_profile_id == row.get::<_, String>(2)?
                && value.state == row.get::<_, i32>(3)?
                && value.version == row.get::<_, String>(4)?
                && valid_version_and_cursor(value.resource_version, row.get(5)?, row.get(6)?))
        },
    )?;
    validate_indexed_rows::<v1::DevelopmentTask, _>(
        connection,
        "SELECT payload,id,state,active_attempt_id,deadline_at_seconds,deadline_at_nanos,
                resource_version,event_cursor,project_id FROM development_tasks",
        |row, value| {
            let deadline = value
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.deadline_at.as_ref());
            Ok(!value.id.is_empty()
                && !value.project_id.is_empty()
                && value.project_id == row.get::<_, String>(8)?
                && value.snapshot.is_some()
                && value.attempt_count > 0
                && value.state != v1::DevelopmentTaskState::Unspecified as i32
                && v1::DevelopmentTaskState::try_from(value.state).is_ok()
                && value.id == row.get::<_, String>(1)?
                && value.state == row.get::<_, i32>(2)?
                && value.active_attempt_id == row.get::<_, Option<String>>(3)?
                && deadline.map(|value| value.seconds) == row.get::<_, Option<i64>>(4)?
                && deadline.map(|value| value.nanos) == row.get::<_, Option<i32>>(5)?
                && valid_version_and_cursor(value.resource_version, row.get(6)?, row.get(7)?))
        },
    )?;
    validate_indexed_rows::<v1::DevelopmentAttempt, _>(
        connection,
        "SELECT payload,id,development_task_id,attempt_number,state,resource_version,event_cursor
         FROM development_attempts",
        |row, value| {
            Ok(!value.id.is_empty()
                && !value.development_task_id.is_empty()
                && value.attempt_number > 0
                && value.snapshot.is_some()
                && value.state != v1::DevelopmentAttemptState::Unspecified as i32
                && v1::DevelopmentAttemptState::try_from(value.state).is_ok()
                && valid_model_usage(value.model_usage.as_ref())
                && value.id == row.get::<_, String>(1)?
                && value.development_task_id == row.get::<_, String>(2)?
                && i64::from(value.attempt_number) == row.get::<_, i64>(3)?
                && value.state == row.get::<_, i32>(4)?
                && valid_version_and_cursor(value.resource_version, row.get(5)?, row.get(6)?))
        },
    )?;
    Ok(())
}

pub(super) fn validate_durable_actions(connection: &Connection) -> Result<(), CleanStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,failure
             FROM durable_actions",
        )
        .map_err(CleanStoreError::Database)?;
    let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
    while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
        let operation: String = row.get(0).map_err(CleanStoreError::Database)?;
        let key: String = row.get(1).map_err(CleanStoreError::Database)?;
        let hash: Vec<u8> = row.get(2).map_err(CleanStoreError::Database)?;
        let payload: Vec<u8> = row.get(3).map_err(CleanStoreError::Database)?;
        let state: String = row.get(4).map_err(CleanStoreError::Database)?;
        let target_type: Option<String> = row.get(5).map_err(CleanStoreError::Database)?;
        let target_id: Option<String> = row.get(6).map_err(CleanStoreError::Database)?;
        let failure: Option<String> = row.get(7).map_err(CleanStoreError::Database)?;
        let mut hasher = Sha256::new();
        hasher.update(b"ayni.orchestrator.action.v1\0");
        hasher.update(operation.as_bytes());
        hasher.update([0]);
        hasher.update(&payload);
        if operation.is_empty()
            || key.is_empty()
            || hasher.finalize().as_slice() != hash
            || !action_payload_is_valid(&operation, &payload)
            || !matches!(state.as_str(), "pending" | "completed" | "failed")
            || target_type.as_deref().is_none_or(str::is_empty)
            || target_id.as_deref().is_none_or(str::is_empty)
            || (state == "failed"
                && failure
                    .as_deref()
                    .is_none_or(|value| value.is_empty() || value.len() > 512))
            || (state != "failed" && failure.is_some())
        {
            return Err(CleanStoreError::CorruptState);
        }
    }
    Ok(())
}

pub(super) fn action_payload_is_valid(operation: &str, payload: &[u8]) -> bool {
    match operation {
        "CreateProject" => v1::CreateProjectRequest::decode(payload)
            .is_ok_and(|v| api_validation::create_project(&v).is_ok()),
        "UpdateProject" => v1::UpdateProjectRequest::decode(payload).is_ok_and(|v| {
            api_validation::project_mutation(
                &v.idempotency_key,
                &v.project_id,
                v.expected_resource_version,
            )
            .is_ok()
                && api_validation::project_name(&v.display_name).is_ok()
        }),
        "ArchiveProject" => v1::ArchiveProjectRequest::decode(payload).is_ok_and(|v| {
            api_validation::project_mutation(
                &v.idempotency_key,
                &v.project_id,
                v.expected_resource_version,
            )
            .is_ok()
        }),
        "CreateRepositoryConnection" => v1::CreateRepositoryConnectionRequest::decode(payload)
            .is_ok_and(|value| api_validation::create_repository_connection(&value).is_ok()),
        "SetRepositoryRegistry" => {
            v1::SetRepositoryRegistryRequest::decode(payload).is_ok_and(|v| {
                api_validation::idempotency_key(&v.idempotency_key).is_ok()
                    && api_validation::resource_id(
                        &v.repository_connection_id,
                        "repository_connection_id",
                    )
                    .is_ok()
                    && v.expected_resource_version > 0
                    && ((v.registry_connection_id.is_empty() && v.image_namespace.is_empty())
                        || (api_validation::resource_id(
                            &v.registry_connection_id,
                            "registry_connection_id",
                        )
                        .is_ok()
                            && (v.image_namespace.is_empty()
                                || crate::registry_probe::valid_namespace(&v.image_namespace))))
            })
        }
        "SetRuntimeRegistry" => v1::SetRuntimeRegistryRequest::decode(payload).is_ok_and(|v| {
            api_validation::idempotency_key(&v.idempotency_key).is_ok()
                && api_validation::resource_id(&v.runtime_profile_id, "runtime_profile_id").is_ok()
                && v.expected_resource_version > 0
                && (v.registry_connection_id.is_empty()
                    || api_validation::resource_id(
                        &v.registry_connection_id,
                        "registry_connection_id",
                    )
                    .is_ok())
        }),
        "CreateRuntimeProfile" => v1::CreateRuntimeProfileRequest::decode(payload)
            .is_ok_and(|value| api_validation::create_runtime_profile(&value).is_ok()),
        "InstallRepositoryAutomation" => v1::InstallRepositoryAutomationRequest::decode(payload)
            .is_ok_and(|value| {
                api_validation::idempotency_key(&value.idempotency_key).is_ok()
                    && api_validation::resource_id(&value.runtime_profile_id, "runtime_profile_id")
                        .is_ok()
            }),
        "ReconcileRuntimeProfile" => {
            v1::ReconcileRuntimeProfileRequest::decode(payload).is_ok_and(|value| {
                api_validation::idempotency_key(&value.idempotency_key).is_ok()
                    && api_validation::resource_id(&value.runtime_profile_id, "runtime_profile_id")
                        .is_ok()
            })
        }
        "CreateDevelopmentTask" => v1::CreateDevelopmentTaskRequest::decode(payload)
            .is_ok_and(|value| api_validation::create_development_task(&value).is_ok()),
        "CancelDevelopmentTask" => {
            v1::CancelDevelopmentTaskRequest::decode(payload).is_ok_and(|value| {
                api_validation::idempotency_key(&value.idempotency_key).is_ok()
                    && api_validation::resource_id(
                        &value.development_task_id,
                        "development_task_id",
                    )
                    .is_ok()
            })
        }
        "RetryTaskPublication" => v1::RetryTaskPublicationRequest::decode(payload)
            .is_ok_and(|v| !v.development_task_id.is_empty()),
        "RetryDevelopmentTask" => {
            v1::RetryDevelopmentTaskRequest::decode(payload).is_ok_and(|value| {
                api_validation::idempotency_key(&value.idempotency_key).is_ok()
                    && api_validation::resource_id(
                        &value.development_task_id,
                        "development_task_id",
                    )
                    .is_ok()
            })
        }
        _ => false,
    }
}

pub(super) fn validate_resource_events(connection: &Connection) -> Result<(), CleanStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT service,resource_type,resource_id,resource_version,payload
             FROM resource_events ORDER BY cursor",
        )
        .map_err(CleanStoreError::Database)?;
    let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
    while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
        let service: String = row.get(0).map_err(CleanStoreError::Database)?;
        let resource_type: String = row.get(1).map_err(CleanStoreError::Database)?;
        let resource_id: String = row.get(2).map_err(CleanStoreError::Database)?;
        let version: i64 = row.get(3).map_err(CleanStoreError::Database)?;
        let payload: Vec<u8> = row.get(4).map_err(CleanStoreError::Database)?;
        let valid =
            valid_resource_event(&service, &resource_type, &payload, &resource_id, version)?;
        if resource_id.is_empty() || version <= 0 || !valid {
            return Err(CleanStoreError::CorruptState);
        }
    }
    Ok(())
}

pub(super) fn valid_resource_event(
    service: &str,
    resource_type: &str,
    payload: &[u8],
    resource_id: &str,
    version: i64,
) -> Result<bool, CleanStoreError> {
    match (service, resource_type) {
        (PROJECT_SERVICE, "project") => Ok(v1::Project::decode(payload).is_ok_and(|v| {
            valid_project(&v)
                && v.id == resource_id
                && i64::try_from(v.resource_version) == Ok(version)
        })),
        (REPOSITORY_SERVICE, "repository_connection") => {
            valid_repository_connection_event(payload, resource_id, version)
        }
        (REPOSITORY_SERVICE, "repository_snapshot") => {
            valid_repository_snapshot_event(payload, resource_id, version)
        }
        (PROFILE_SERVICE, "runtime_profile") => {
            valid_runtime_profile_event(payload, resource_id, version)
        }
        (PROFILE_SERVICE, "runtime_profile_revision") => {
            valid_runtime_profile_revision_event(payload, resource_id, version)
        }
        (PROFILE_SERVICE, "runtime_artifact") => {
            valid_runtime_artifact_event(payload, resource_id, version)
        }
        (PROFILE_SERVICE, "automation_installation") => {
            valid_automation_installation_event(payload, resource_id, version)
        }
        (DEVELOPMENT_SERVICE, "development_task") => {
            valid_development_task_event(payload, resource_id, version)
        }
        (DEVELOPMENT_SERVICE, "development_attempt") => {
            valid_development_attempt_event(payload, resource_id, version)
        }
        _ => Ok(false),
    }
}

pub(super) fn valid_repository_connection_event(
    payload: &[u8],
    resource_id: &str,
    version: i64,
) -> Result<bool, CleanStoreError> {
    let value = decode::<v1::RepositoryConnection>(payload)?;
    Ok(value.id == resource_id
        && value.provider.is_some()
        && value.status != v1::RepositoryConnectionStatus::Unspecified as i32
        && v1::RepositoryConnectionStatus::try_from(value.status).is_ok()
        && i64::try_from(value.resource_version) == Ok(version))
}

pub(super) fn valid_repository_snapshot_event(
    payload: &[u8],
    resource_id: &str,
    version: i64,
) -> Result<bool, CleanStoreError> {
    let value = decode::<v1::RepositorySnapshot>(payload)?;
    Ok(value.id == resource_id
        && !value.repository_connection_id.is_empty()
        && !value.revision.is_empty()
        && i64::try_from(value.resource_version) == Ok(version))
}

pub(super) fn valid_runtime_profile_event(
    payload: &[u8],
    resource_id: &str,
    version: i64,
) -> Result<bool, CleanStoreError> {
    let value = decode::<v1::RuntimeProfile>(payload)?;
    Ok(value.id == resource_id
        && !value.repository_connection_id.is_empty()
        && value.kind != v1::RuntimeProfileKind::Unspecified as i32
        && value.support != v1::RuntimeProfileSupport::Unspecified as i32
        && value.state != v1::RuntimeProfileState::Unspecified as i32
        && safe_metadata(&value.reconciliation_metadata, 16, 8192)
        && v1::RuntimeProfileKind::try_from(value.kind).is_ok()
        && v1::RuntimeProfileSupport::try_from(value.support).is_ok()
        && v1::RuntimeProfileState::try_from(value.state).is_ok()
        && i64::try_from(value.resource_version) == Ok(version))
}

pub(super) fn valid_runtime_profile_revision_event(
    payload: &[u8],
    resource_id: &str,
    version: i64,
) -> Result<bool, CleanStoreError> {
    let value = decode::<v1::RuntimeProfileRevision>(payload)?;
    Ok(value.id == resource_id
        && !value.runtime_profile_id.is_empty()
        && !value.repository_snapshot_id.is_empty()
        && value.revision_number > 0
        && value.configuration.is_some()
        && i64::try_from(value.resource_version) == Ok(version))
}

pub(super) fn valid_runtime_artifact_event(
    payload: &[u8],
    resource_id: &str,
    version: i64,
) -> Result<bool, CleanStoreError> {
    let value = decode::<v1::RuntimeArtifact>(payload)?;
    Ok(value.id == resource_id
        && valid_artifact_report(&value)
        && i64::try_from(value.resource_version) == Ok(version))
}

pub(super) fn valid_automation_installation_event(
    payload: &[u8],
    resource_id: &str,
    version: i64,
) -> Result<bool, CleanStoreError> {
    let value = decode::<v1::AutomationInstallation>(payload)?;
    Ok(value.id == resource_id
        && !value.runtime_profile_id.is_empty()
        && !value.version.is_empty()
        && value.state != v1::AutomationInstallationState::Unspecified as i32
        && v1::AutomationInstallationState::try_from(value.state).is_ok()
        && i64::try_from(value.resource_version) == Ok(version))
}

pub(super) fn valid_development_task_event(
    payload: &[u8],
    resource_id: &str,
    version: i64,
) -> Result<bool, CleanStoreError> {
    let value = decode::<v1::DevelopmentTask>(payload)?;
    Ok(value.id == resource_id
        && value.snapshot.is_some()
        && value.attempt_count > 0
        && value.state != v1::DevelopmentTaskState::Unspecified as i32
        && v1::DevelopmentTaskState::try_from(value.state).is_ok()
        && i64::try_from(value.resource_version) == Ok(version))
}

pub(super) fn valid_development_attempt_event(
    payload: &[u8],
    resource_id: &str,
    version: i64,
) -> Result<bool, CleanStoreError> {
    let value = decode::<v1::DevelopmentAttempt>(payload)?;
    Ok(value.id == resource_id
        && !value.development_task_id.is_empty()
        && value.attempt_number > 0
        && value.snapshot.is_some()
        && value.state != v1::DevelopmentAttemptState::Unspecified as i32
        && v1::DevelopmentAttemptState::try_from(value.state).is_ok()
        && valid_model_usage(value.model_usage.as_ref())
        && i64::try_from(value.resource_version) == Ok(version))
}

pub(super) fn validate_page_snapshot_rows(connection: &Connection) -> Result<(), CleanStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT method,filter_hash,cursor,items,token_secret,created_at FROM page_snapshots",
        )
        .map_err(CleanStoreError::Database)?;
    let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
    while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
        let method: String = row.get(0).map_err(CleanStoreError::Database)?;
        let filter_hash: String = row.get(1).map_err(CleanStoreError::Database)?;
        let cursor: i64 = row.get(2).map_err(CleanStoreError::Database)?;
        let encoded: String = row.get(3).map_err(CleanStoreError::Database)?;
        let token_secret: String = row.get(4).map_err(CleanStoreError::Database)?;
        let created_at: i64 = row.get(5).map_err(CleanStoreError::Database)?;
        let items: Vec<String> =
            serde_json::from_str(&encoded).map_err(|_| CleanStoreError::CorruptState)?;
        let decoded = items
            .iter()
            .map(|item| {
                URL_SAFE_NO_PAD
                    .decode(item)
                    .map_err(|_| CleanStoreError::CorruptState)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if method.is_empty()
            || filter_hash.len() != 64
            || cursor < 0
            || token_secret.is_empty()
            || created_at < 0
            || items.len() > MAX_PAGE_SNAPSHOT_ITEMS
            || encoded.len() > MAX_PAGE_SNAPSHOT_BYTES.saturating_mul(2)
            || decoded.iter().map(Vec::len).sum::<usize>() > MAX_PAGE_SNAPSHOT_BYTES
            || !page_items_decode(&method, &decoded)
        {
            return Err(CleanStoreError::CorruptState);
        }
    }
    Ok(())
}

pub(super) fn get_payload<M: Message + Default>(
    connection: &Connection,
    sql: &str,
    id: &str,
    resource_type: &'static str,
) -> Result<M, CleanStoreError> {
    let payload = connection
        .query_row(sql, [id], |row| row.get::<_, Vec<u8>>(0))
        .optional()
        .map_err(CleanStoreError::Database)?
        .ok_or_else(|| CleanStoreError::NotFound {
            resource_type,
            resource_id: id.to_owned(),
        })?;
    decode(&payload)
}

pub(super) fn collect_bounded_payloads(
    rows: &mut rusqlite::Rows<'_>,
) -> Result<Vec<Vec<u8>>, CleanStoreError> {
    let mut payloads = Vec::new();
    let mut bytes = 0_usize;
    while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
        let value = row.get_ref(0).map_err(CleanStoreError::Database)?;
        let ValueRef::Blob(payload) = value else {
            return Err(CleanStoreError::CorruptState);
        };
        let next_bytes = bytes
            .checked_add(payload.len())
            .ok_or(CleanStoreError::BoundExceeded)?;
        if payloads.len() >= MAX_PAGE_SNAPSHOT_ITEMS || next_bytes > MAX_PAGE_SNAPSHOT_BYTES {
            return Err(CleanStoreError::BoundExceeded);
        }
        payloads.push(payload.to_vec());
        bytes = next_bytes;
    }
    Ok(payloads)
}

pub(super) fn query_payloads(
    connection: &Connection,
    table: &str,
    parent_column: &str,
    status: Option<i32>,
    parent: Option<&str>,
) -> Result<Vec<Vec<u8>>, CleanStoreError> {
    let sql = if parent_column.is_empty() {
        format!("SELECT payload FROM {table} WHERE (?1 IS NULL OR status=?1) ORDER BY id")
    } else {
        format!("SELECT payload FROM {table} WHERE {parent_column}=?1 ORDER BY id")
    };
    let mut statement = connection
        .prepare(&sql)
        .map_err(CleanStoreError::Database)?;
    if parent_column.is_empty() {
        let mut rows = statement
            .query([status])
            .map_err(CleanStoreError::Database)?;
        collect_bounded_payloads(&mut rows)
    } else {
        let mut rows = statement
            .query([parent])
            .map_err(CleanStoreError::Database)?;
        collect_bounded_payloads(&mut rows)
    }
}

pub(super) fn persist_task_update(
    transaction: &Transaction<'_>,
    task: &v1::DevelopmentTask,
    expected_version: u64,
) -> Result<(), CleanStoreError> {
    let repository_id = task
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.repository.as_ref())
        .map(|repository| repository.repository_connection_id.as_str())
        .ok_or(CleanStoreError::CorruptState)?;
    let cursor = append_event(
        transaction,
        DEVELOPMENT_SERVICE,
        "development_task",
        &task.id,
        task.resource_version,
        Some(repository_id),
        Some(&task.id),
        task,
    )?;
    let changed = transaction
        .execute(
            "UPDATE development_tasks SET state=?1,active_attempt_id=?2,payload=?3,
                resource_version=?4,event_cursor=?5 WHERE id=?6 AND resource_version=?7",
            params![
                task.state,
                task.active_attempt_id,
                task.encode_to_vec(),
                i64::try_from(task.resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                task.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?
            ],
        )
        .map_err(CleanStoreError::Database)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(CleanStoreError::StaleVersion)
    }
}

pub(super) fn persist_attempt_update(
    transaction: &Transaction<'_>,
    attempt: &v1::DevelopmentAttempt,
    expected_version: u64,
    task: &v1::DevelopmentTask,
) -> Result<(), CleanStoreError> {
    let repository_id = task
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.repository.as_ref())
        .map(|repository| repository.repository_connection_id.as_str())
        .ok_or(CleanStoreError::CorruptState)?;
    let cursor = append_event(
        transaction,
        DEVELOPMENT_SERVICE,
        "development_attempt",
        &attempt.id,
        attempt.resource_version,
        Some(repository_id),
        Some(&task.id),
        attempt,
    )?;
    let changed = transaction
        .execute(
            "UPDATE development_attempts SET state=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
            params![
                attempt.state,
                attempt.encode_to_vec(),
                i64::try_from(attempt.resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                attempt.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?
            ],
        )
        .map_err(CleanStoreError::Database)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(CleanStoreError::StaleVersion)
    }
}
