fn valid_project(value: &v1::Project) -> bool {
    api_validation::resource_id(&value.id, "project_id").is_ok()
        && api_validation::project_name(&value.display_name).is_ok()
        && matches!(
            v1::ProjectState::try_from(value.state),
            Ok(v1::ProjectState::Active | v1::ProjectState::Archived)
        )
        && value.repository_connection_ids.len() == 1
        && api_validation::resource_id(
            &value.repository_connection_ids[0],
            "repository_connection_id",
        )
        .is_ok()
        && value.created_at.is_some()
        && value.updated_at.is_some()
        && value.resource_version > 0
}

fn validate_projects(connection: &Connection) -> Result<(), CleanStoreError> {
    validate_indexed_rows::<v1::Project, _>(
        connection,
        "SELECT p.payload,p.id,p.state,p.resource_version,p.event_cursor,m.repository_connection_id,m.active FROM projects p LEFT JOIN project_repositories m ON m.project_id=p.id",
        |row, value| {
            Ok(valid_project(value)
                && value.id == row.get::<_, String>(1)?
                && value.state == row.get::<_, i32>(2)?
                && valid_version_and_cursor(value.resource_version, row.get(3)?, row.get(4)?)
                && row.get::<_, Option<String>>(5)?.as_ref()
                    == value.repository_connection_ids.first()
                && row.get::<_, Option<i32>>(6)?
                    == Some(i32::from(value.state == v1::ProjectState::Active as i32)))
        },
    )?;
    let mismatch: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM development_tasks t JOIN project_repositories m ON m.project_id=t.project_id WHERE t.repository_connection_id != m.repository_connection_id)", [], |r| r.get(0)).map_err(CleanStoreError::Database)?;
    if mismatch {
        return Err(CleanStoreError::CorruptState);
    }
    Ok(())
}
