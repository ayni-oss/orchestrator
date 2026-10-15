use super::{
    CleanStoreError, Connection, api_validation, v1, valid_version_and_cursor,
    validate_indexed_rows,
};

pub(super) fn valid_project(value: &v1::Project) -> bool {
    api_validation::resource_id(&value.id, "project_id").is_ok()
        && api_validation::project_name(&value.display_name).is_ok()
        && matches!(
            v1::ProjectState::try_from(value.state),
            Ok(v1::ProjectState::Active | v1::ProjectState::Archived)
        )
        && api_validation::project_members(&value.repository_connection_ids).is_ok()
        && value.agent_defaults.as_ref().is_none_or(|defaults| {
            if crate::agent_profiles::selection(
                defaults.planner_profile.as_ref(),
                &ayni_orchestrator_runtime::agent_profile::Role::Planner,
            )
            .is_err()
                || crate::agent_profiles::selection(
                    defaults.worker_profile.as_ref(),
                    &ayni_orchestrator_runtime::agent_profile::Role::Worker,
                )
                .is_err()
            {
                return false;
            }
            [
                (&defaults.project_connection_id, &defaults.project_model),
                (&defaults.coding_connection_id, &defaults.coding_model),
            ]
            .into_iter()
            .all(|(connection, model)| {
                (connection.is_empty() && model.is_empty())
                    || (api_validation::resource_id(connection, "connection_id").is_ok()
                        && !model.is_empty()
                        && model.len() <= 128
                        && !model.chars().any(char::is_control))
            })
        })
        && value.created_at.is_some()
        && value.updated_at.is_some()
        && value.resource_version > 0
}

pub(super) fn validate_projects(connection: &Connection) -> Result<(), CleanStoreError> {
    validate_indexed_rows::<v1::Project, _>(
        connection,
        "SELECT payload,id,state,resource_version,event_cursor FROM projects",
        |row, value| {
            if !valid_project(value)
                || value.id != row.get::<_, String>(1)?
                || value.state != row.get::<_, i32>(2)?
                || !valid_version_and_cursor(value.resource_version, row.get(3)?, row.get(4)?)
            {
                return Ok(false);
            }
            let mut statement = connection.prepare("SELECT repository_connection_id,active FROM project_repositories WHERE project_id=?1")?;
            let members = statement
                .query_map([&value.id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(value.repository_connection_ids.iter().all(|id| {
                members.iter().any(|(member, active)| {
                    member == id
                        && *active == i32::from(value.state == v1::ProjectState::Active as i32)
                })
            }) && members
                .iter()
                .all(|(id, active)| *active == 0 || value.repository_connection_ids.contains(id)))
        },
    )?;
    // Removed memberships remain in the index to validate retained task history.
    let mismatch: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM development_tasks t WHERE NOT EXISTS(SELECT 1 FROM project_repositories m WHERE m.project_id=t.project_id AND m.repository_connection_id=t.repository_connection_id))", [], |r| r.get(0)).map_err(CleanStoreError::Database)?;
    if mismatch {
        return Err(CleanStoreError::CorruptState);
    }
    Ok(())
}
