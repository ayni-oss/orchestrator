//! Complete product projections, frozen by the same bounded pagination store as resources.
use std::collections::HashMap;

use ayni_orchestrator_application::summaries::{RuntimeReadiness, project_ready};

use super::{
    CleanStore, CleanStoreError, Connection, Message, OptionalExtension, PROJECT_SERVICE, Page,
    REPOSITORY_SERVICE, collect_bounded_payloads, v1,
};

fn resources<T: Message + Default>(
    connection: &Connection,
    table: &str,
) -> Result<Vec<T>, CleanStoreError> {
    let mut statement = connection
        .prepare(&format!("SELECT payload FROM {table} ORDER BY id"))
        .map_err(CleanStoreError::Database)?;
    let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
    collect_bounded_payloads(&mut rows)?
        .into_iter()
        .map(|payload| T::decode(payload.as_slice()).map_err(|_| CleanStoreError::CorruptState))
        .collect()
}

fn readiness(
    connection: &Connection,
) -> Result<HashMap<String, RuntimeReadiness>, CleanStoreError> {
    let mut result = HashMap::<String, RuntimeReadiness>::new();
    for profile in resources::<v1::RuntimeProfile>(connection, "runtime_profiles")? {
        if profile.support != v1::RuntimeProfileSupport::Operational as i32 {
            continue;
        }
        let summary = result.entry(profile.repository_connection_id).or_default();
        summary.operational_profiles += 1;
        summary.ready_profiles += u32::from(profile.state == v1::RuntimeProfileState::Ready as i32);
    }
    Ok(result)
}

const fn runtime(summary: RuntimeReadiness, ready: bool) -> v1::RuntimeReadinessSummary {
    v1::RuntimeReadinessSummary {
        complete: true,
        ready,
        operational_profiles: summary.operational_profiles,
        ready_profiles: summary.ready_profiles,
    }
}

impl CleanStore {
    pub(crate) fn repository_summaries(
        &mut self,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::RepositorySummary>, CleanStoreError> {
        self.page_messages(
            "ListRepositorySummaries",
            "",
            page,
            |connection| {
                let profiles = readiness(connection)?;
                Ok(
                    resources::<v1::RepositoryConnection>(connection, "repository_connections")?
                        .into_iter()
                        .map(|repository| {
                            let summary = profiles.get(&repository.id).copied().unwrap_or_default();
                            v1::RepositorySummary {
                                repository_connection: Some(repository),
                                runtime: Some(runtime(summary, summary.ready())),
                            }
                            .encode_to_vec()
                        })
                        .collect(),
                )
            },
            REPOSITORY_SERVICE,
        )
    }

    pub(crate) fn project_summaries(
        &mut self,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::ProjectSummary>, CleanStoreError> {
        self.page_messages("ListProjectSummaries", "", page, |connection| {
            let profiles = readiness(connection)?;
            let repositories: HashMap<_, _> = resources::<v1::RepositoryConnection>(connection, "repository_connections")?.into_iter().map(|repository| (repository.id, repository.display_name)).collect();
            let mut latest = connection.prepare("SELECT payload FROM development_tasks WHERE project_id=?1 ORDER BY created_at_seconds DESC,created_at_nanos DESC,id DESC LIMIT 1").map_err(CleanStoreError::Database)?;
            resources::<v1::Project>(connection, "projects")?.into_iter().map(|project| {
                let members: Vec<_> = project.repository_connection_ids.iter().map(|id| profiles.get(id).copied().unwrap_or_default()).collect();
                let aggregate = members.iter().fold(RuntimeReadiness::default(), |mut total, item| {
                    total.operational_profiles += item.operational_profiles;
                    total.ready_profiles += item.ready_profiles;
                    total
                });
                let repository_names = project.repository_connection_ids.iter().map(|id| repositories.get(id).cloned().ok_or(CleanStoreError::CorruptState)).collect::<Result<Vec<_>, _>>()?;
                let latest_task = latest.query_row([&project.id], |row| row.get::<_, Vec<u8>>(0)).optional().map_err(CleanStoreError::Database)?
                    .map(|payload| v1::DevelopmentTask::decode(payload.as_slice()).map_err(|_| CleanStoreError::CorruptState)).transpose()?
                    .map(|task| Ok(v1::ProjectTaskSummary {id:task.id,state:task.state,request:task.snapshot.ok_or(CleanStoreError::CorruptState)?.request,created_at:task.created_at})).transpose()?;
                Ok(v1::ProjectSummary { project: Some(project), repository_names, runtime: Some(runtime(aggregate, project_ready(&members))), latest_task, tasks_complete: true }.encode_to_vec())
            }).collect()
        }, PROJECT_SERVICE)
    }
}
