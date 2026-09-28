use crate::{
    development::AgentProfile,
    orchestration::{Store, StoreError},
};
use ayni_orchestrator_domain::{
    ModelInteractionOutcome, ModelUsageReport, ModelUsageStage, ProviderTokenUsage, TaskId,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const PRICING_VERSION: &str = "openai-public-2025-08-07";
const PRICED_MODEL: &str = "gpt-5-2025-08-07";
const PRICING_CURRENCY: &str = "USD";
const INPUT_MICRO_USD_PER_MILLION: u64 = 1_250_000;
const CACHED_INPUT_MICRO_USD_PER_MILLION: u64 = 125_000;
const OUTPUT_MICRO_USD_PER_MILLION: u64 = 10_000_000;
const MAX_REPORT_BYTES: u64 = 16_384;
const MAX_REPORTS_PER_TASK: usize = 64;

/// Reproducible pricing inputs for an estimate, never provider billing truth.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EstimatedModelCost {
    pub label: &'static str,
    pub amount_micros: u64,
    pub currency: String,
    pub pricing_version: String,
    pub priced_input_tokens: u64,
    pub priced_cached_input_tokens: u64,
    pub priced_output_tokens: u64,
    pub input_rate_micros_per_million: u64,
    pub cached_input_rate_micros_per_million: u64,
    pub output_rate_micros_per_million: u64,
}

/// One durable provider interaction. It contains counters and correlation only.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelUsageInteraction {
    pub call_id: String,
    pub stage: ModelUsageStage,
    pub provider: String,
    /// Requested task-profile model alias or identity.
    pub model: String,
    /// Exact provider-reported model identity, when available.
    pub resolved_model: Option<String>,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub latency_ms: u64,
    pub outcome: ModelInteractionOutcome,
    pub tokens: ProviderTokenUsage,
    pub estimated_cost: Option<EstimatedModelCost>,
}

/// Aggregate usage whose token fields remain unknown if any included interaction omitted them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelUsageSummary {
    pub interaction_count: usize,
    pub unavailable_interaction_count: usize,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub estimated_cost_micros: Option<u64>,
    pub estimated_cost_currency: Option<String>,
    pub pricing_version: Option<String>,
}

/// Aggregate usage for one stage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StageModelUsage {
    pub stage: ModelUsageStage,
    #[serde(flatten)]
    pub summary: ModelUsageSummary,
}

/// Whether usage collection is supported, still active, or terminal for a task.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelUsageCollectionStatus {
    /// The task predates usage instrumentation.
    Unavailable,
    /// A model-consuming stage may still append interactions.
    Collecting,
    /// Every model-consuming stage for the task is terminal.
    Complete,
}

/// Model usage attached to each development-task API representation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskModelUsage {
    pub collection_status: ModelUsageCollectionStatus,
    pub interactions: Vec<ModelUsageInteraction>,
    pub by_stage: Vec<StageModelUsage>,
    pub total: ModelUsageSummary,
}

impl TaskModelUsage {
    fn from_interactions(
        interactions: Vec<ModelUsageInteraction>,
        collection_status: ModelUsageCollectionStatus,
    ) -> Self {
        let by_stage = [ModelUsageStage::Coding, ModelUsageStage::PrPreparation]
            .into_iter()
            .map(|stage| StageModelUsage {
                stage,
                summary: summarize(interactions.iter().filter(|item| item.stage == stage)),
            })
            .collect();
        let total = summarize(interactions.iter());
        Self {
            collection_status,
            interactions,
            by_stage,
            total,
        }
    }
}

fn complete_sum<'a>(values: impl Iterator<Item = &'a Option<u64>>) -> Option<u64> {
    let values = values.copied().collect::<Vec<_>>();
    if values.is_empty() || values.iter().any(Option::is_none) {
        return None;
    }
    values
        .into_iter()
        .flatten()
        .try_fold(0_u64, u64::checked_add)
}

fn summarize<'a>(
    interactions: impl Iterator<Item = &'a ModelUsageInteraction>,
) -> ModelUsageSummary {
    let interactions = interactions.collect::<Vec<_>>();
    let costs = interactions
        .iter()
        .map(|item| item.estimated_cost.as_ref())
        .collect::<Vec<_>>();
    let first_cost = costs.first().copied().flatten();
    let consistent_pricing = first_cost.is_some_and(|first| {
        costs.iter().all(|cost| {
            cost.is_some_and(|cost| {
                cost.currency == first.currency && cost.pricing_version == first.pricing_version
            })
        })
    });
    let estimated_cost_micros = if consistent_pricing {
        costs
            .iter()
            .flatten()
            .map(|cost| cost.amount_micros)
            .try_fold(0_u64, u64::checked_add)
    } else {
        None
    };
    ModelUsageSummary {
        interaction_count: interactions.len(),
        unavailable_interaction_count: interactions
            .iter()
            .filter(|item| !item.tokens.is_available())
            .count(),
        input_tokens: complete_sum(interactions.iter().map(|item| &item.tokens.input_tokens)),
        output_tokens: complete_sum(interactions.iter().map(|item| &item.tokens.output_tokens)),
        cached_input_tokens: complete_sum(
            interactions
                .iter()
                .map(|item| &item.tokens.cached_input_tokens),
        ),
        reasoning_tokens: complete_sum(
            interactions
                .iter()
                .map(|item| &item.tokens.reasoning_tokens),
        ),
        total_tokens: complete_sum(interactions.iter().map(|item| &item.tokens.total_tokens)),
        estimated_cost_micros,
        estimated_cost_currency: estimated_cost_micros
            .and_then(|_| first_cost.map(|cost| cost.currency.clone())),
        pricing_version: estimated_cost_micros
            .and_then(|_| first_cost.map(|cost| cost.pricing_version.clone())),
    }
}

fn estimate(
    provider: &str,
    resolved_model: Option<&str>,
    tokens: ProviderTokenUsage,
) -> Option<EstimatedModelCost> {
    if provider != "openai" || resolved_model != Some(PRICED_MODEL) {
        return None;
    }
    let input = tokens.input_tokens?;
    let output = tokens.output_tokens?;
    let cached = tokens.cached_input_tokens.unwrap_or(0);
    if cached > input {
        return None;
    }
    let regular = input - cached;
    let numerator = u128::from(regular) * u128::from(INPUT_MICRO_USD_PER_MILLION)
        + u128::from(cached) * u128::from(CACHED_INPUT_MICRO_USD_PER_MILLION)
        + u128::from(output) * u128::from(OUTPUT_MICRO_USD_PER_MILLION);
    let rounded = numerator.saturating_add(500_000) / 1_000_000;
    Some(EstimatedModelCost {
        label: "estimated",
        amount_micros: u64::try_from(rounded).ok()?,
        currency: PRICING_CURRENCY.to_owned(),
        pricing_version: PRICING_VERSION.to_owned(),
        priced_input_tokens: regular,
        priced_cached_input_tokens: cached,
        priced_output_tokens: output,
        input_rate_micros_per_million: INPUT_MICRO_USD_PER_MILLION,
        cached_input_rate_micros_per_million: CACHED_INPUT_MICRO_USD_PER_MILLION,
        output_rate_micros_per_million: OUTPUT_MICRO_USD_PER_MILLION,
    })
}

fn parse_stage(value: &str) -> Result<ModelUsageStage, StoreError> {
    match value {
        "coding" => Ok(ModelUsageStage::Coding),
        "pr_preparation" => Ok(ModelUsageStage::PrPreparation),
        _ => Err(StoreError::CorruptState(
            "invalid persisted model usage stage",
        )),
    }
}

fn parse_outcome(value: &str) -> Result<ModelInteractionOutcome, StoreError> {
    match value {
        "succeeded" => Ok(ModelInteractionOutcome::Succeeded),
        "failed" => Ok(ModelInteractionOutcome::Failed),
        "cancelled" => Ok(ModelInteractionOutcome::Cancelled),
        _ => Err(StoreError::CorruptState(
            "invalid persisted model interaction outcome",
        )),
    }
}

fn stored_u64(value: Option<i64>) -> Result<Option<u64>, StoreError> {
    value
        .map(u64::try_from)
        .transpose()
        .map_err(|_| StoreError::CorruptState("negative persisted model usage value"))
}

fn sqlite_u64(value: Option<u64>) -> Result<Option<i64>, StoreError> {
    value
        .map(i64::try_from)
        .transpose()
        .map_err(|_| StoreError::InvalidInput("model usage counter exceeds storage bounds"))
}

impl Store {
    pub(crate) fn record_model_usage(
        &self,
        id: TaskId,
        report: &ModelUsageReport,
    ) -> Result<bool, StoreError> {
        validate_report(report)?;
        let profile_json = self
            .connection
            .query_row(
                "SELECT profile_snapshot FROM development_tasks WHERE id=?1",
                [id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StoreError::Database)?
            .ok_or(StoreError::TaskNotFound(id))?;
        let profile: AgentProfile = serde_json::from_str(&profile_json)
            .map_err(|_| StoreError::CorruptState("invalid persisted profile"))?;
        if report.provider != profile.provider || report.model != profile.model {
            return Err(StoreError::InvalidInput(
                "model usage attribution differs from the task profile",
            ));
        }
        if let Some(existing) = self.model_usage_interaction(id, report.stage, &report.call_id)? {
            if report_matches(&existing, report) {
                return Ok(false);
            }
            return Err(StoreError::InvalidInput(
                "model usage call identity was reused with different data",
            ));
        }
        let completed_at_ms = report.completed_at_ms.ok_or(StoreError::InvalidInput(
            "terminal model usage report has no completion time",
        ))?;
        let latency_ms = report.latency_ms.ok_or(StoreError::InvalidInput(
            "terminal model usage report has no latency",
        ))?;
        let cost = estimate(
            &report.provider,
            report.resolved_model.as_deref(),
            report.tokens,
        );
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(StoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO development_model_usage(
                    task_id, stage, call_id, provider, model, resolved_model, started_at_ms,
                    completed_at_ms, latency_ms, outcome, input_tokens, output_tokens,
                    cached_input_tokens, reasoning_tokens, total_tokens, estimated_cost_micros,
                    pricing_currency, pricing_version, priced_input_tokens,
                    priced_cached_input_tokens, priced_output_tokens, input_rate,
                    cached_input_rate, output_rate)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)",
                params![
                    id.to_string(),
                    report.stage.as_str(),
                    report.call_id,
                    report.provider,
                    report.model,
                    report.resolved_model,
                    report.started_at_ms,
                    completed_at_ms,
                    i64::try_from(latency_ms).map_err(|_| StoreError::InvalidInput("model latency exceeds storage bounds"))?,
                    report.outcome.as_str(),
                    sqlite_u64(report.tokens.input_tokens)?,
                    sqlite_u64(report.tokens.output_tokens)?,
                    sqlite_u64(report.tokens.cached_input_tokens)?,
                    sqlite_u64(report.tokens.reasoning_tokens)?,
                    sqlite_u64(report.tokens.total_tokens)?,
                    cost.as_ref().map(|value| i64::try_from(value.amount_micros)).transpose().map_err(|_| StoreError::InvalidInput("estimated cost exceeds storage bounds"))?,
                    cost.as_ref().map(|value| value.currency.as_str()),
                    cost.as_ref().map(|value| value.pricing_version.as_str()),
                    cost.as_ref().map(|value| i64::try_from(value.priced_input_tokens)).transpose().map_err(|_| StoreError::InvalidInput("pricing input exceeds storage bounds"))?,
                    cost.as_ref().map(|value| i64::try_from(value.priced_cached_input_tokens)).transpose().map_err(|_| StoreError::InvalidInput("pricing input exceeds storage bounds"))?,
                    cost.as_ref().map(|value| i64::try_from(value.priced_output_tokens)).transpose().map_err(|_| StoreError::InvalidInput("pricing input exceeds storage bounds"))?,
                    cost.as_ref().map(|value| i64::try_from(value.input_rate_micros_per_million)).transpose().map_err(|_| StoreError::InvalidInput("pricing rate exceeds storage bounds"))?,
                    cost.as_ref().map(|value| i64::try_from(value.cached_input_rate_micros_per_million)).transpose().map_err(|_| StoreError::InvalidInput("pricing rate exceeds storage bounds"))?,
                    cost.as_ref().map(|value| i64::try_from(value.output_rate_micros_per_million)).transpose().map_err(|_| StoreError::InvalidInput("pricing rate exceeds storage bounds"))?,
                ],
            )
            .map_err(StoreError::Database)?;
        transaction
            .execute(
                "UPDATE development_tasks SET resource_version=resource_version+1 WHERE id=?1",
                [id.to_string()],
            )
            .map_err(StoreError::Database)?;
        transaction.commit().map_err(StoreError::Database)?;
        Ok(true)
    }

    fn model_usage_interaction(
        &self,
        id: TaskId,
        stage: ModelUsageStage,
        call_id: &str,
    ) -> Result<Option<ModelUsageInteraction>, StoreError> {
        let all = self.model_usage_interactions(id)?;
        Ok(all
            .into_iter()
            .find(|item| item.stage == stage && item.call_id == call_id))
    }

    pub(crate) fn task_model_usage(&self, id: TaskId) -> Result<TaskModelUsage, StoreError> {
        let complete = self
            .connection
            .query_row(
                "SELECT complete FROM development_model_usage_collection WHERE task_id=?1",
                [id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(StoreError::Database)?;
        let status = match complete {
            None => ModelUsageCollectionStatus::Unavailable,
            Some(false) => ModelUsageCollectionStatus::Collecting,
            Some(true) => ModelUsageCollectionStatus::Complete,
        };
        Ok(TaskModelUsage::from_interactions(
            self.model_usage_interactions(id)?,
            status,
        ))
    }

    pub(crate) fn complete_model_usage_collection(&self, id: TaskId) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(StoreError::Database)?;
        let changed = transaction
            .execute(
                "UPDATE development_model_usage_collection SET complete=1
                 WHERE task_id=?1 AND complete=0",
                [id.to_string()],
            )
            .map_err(StoreError::Database)?;
        if changed == 1 {
            transaction
                .execute(
                    "UPDATE development_tasks SET resource_version=resource_version+1 WHERE id=?1",
                    [id.to_string()],
                )
                .map_err(StoreError::Database)?;
        }
        transaction.commit().map_err(StoreError::Database)?;
        Ok(())
    }

    fn model_usage_interactions(
        &self,
        id: TaskId,
    ) -> Result<Vec<ModelUsageInteraction>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT stage, call_id, provider, model, resolved_model, started_at_ms,
                    completed_at_ms, latency_ms, outcome, input_tokens, output_tokens,
                    cached_input_tokens, reasoning_tokens, total_tokens, estimated_cost_micros,
                    pricing_currency, pricing_version, priced_input_tokens,
                    priced_cached_input_tokens, priced_output_tokens, input_rate,
                    cached_input_rate, output_rate
                 FROM development_model_usage WHERE task_id=?1 ORDER BY stage, call_id",
            )
            .map_err(StoreError::Database)?;
        let rows = statement
            .query_map([id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                    row.get::<_, Option<i64>>(12)?,
                    row.get::<_, Option<i64>>(13)?,
                    row.get::<_, Option<i64>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<String>>(16)?,
                    row.get::<_, Option<i64>>(17)?,
                    row.get::<_, Option<i64>>(18)?,
                    row.get::<_, Option<i64>>(19)?,
                    row.get::<_, Option<i64>>(20)?,
                    row.get::<_, Option<i64>>(21)?,
                    row.get::<_, Option<i64>>(22)?,
                ))
            })
            .map_err(StoreError::Database)?;
        rows.map(|row| {
            let row = row.map_err(StoreError::Database)?;
            let cost = match row.14 {
                Some(amount) => Some(EstimatedModelCost {
                    label: "estimated",
                    amount_micros: u64::try_from(amount).map_err(|_| {
                        StoreError::CorruptState("invalid persisted estimated cost")
                    })?,
                    currency: row
                        .15
                        .ok_or(StoreError::CorruptState("estimated cost has no currency"))?,
                    pricing_version: row.16.ok_or(StoreError::CorruptState(
                        "estimated cost has no pricing version",
                    ))?,
                    priced_input_tokens: stored_u64(row.17)?
                        .ok_or(StoreError::CorruptState("estimated cost has no input"))?,
                    priced_cached_input_tokens: stored_u64(row.18)?.ok_or(
                        StoreError::CorruptState("estimated cost has no cached input"),
                    )?,
                    priced_output_tokens: stored_u64(row.19)?
                        .ok_or(StoreError::CorruptState("estimated cost has no output"))?,
                    input_rate_micros_per_million: stored_u64(row.20)?
                        .ok_or(StoreError::CorruptState("estimated cost has no input rate"))?,
                    cached_input_rate_micros_per_million: stored_u64(row.21)?.ok_or(
                        StoreError::CorruptState("estimated cost has no cached rate"),
                    )?,
                    output_rate_micros_per_million: stored_u64(row.22)?.ok_or(
                        StoreError::CorruptState("estimated cost has no output rate"),
                    )?,
                }),
                None => None,
            };
            Ok(ModelUsageInteraction {
                stage: parse_stage(&row.0)?,
                call_id: row.1,
                provider: row.2,
                model: row.3,
                resolved_model: row.4,
                started_at_ms: row.5,
                completed_at_ms: row.6,
                latency_ms: u64::try_from(row.7)
                    .map_err(|_| StoreError::CorruptState("invalid persisted model latency"))?,
                outcome: parse_outcome(&row.8)?,
                tokens: ProviderTokenUsage {
                    input_tokens: stored_u64(row.9)?,
                    output_tokens: stored_u64(row.10)?,
                    cached_input_tokens: stored_u64(row.11)?,
                    reasoning_tokens: stored_u64(row.12)?,
                    total_tokens: stored_u64(row.13)?,
                },
                estimated_cost: cost,
            })
        })
        .collect()
    }
}

fn report_matches(existing: &ModelUsageInteraction, report: &ModelUsageReport) -> bool {
    existing.call_id == report.call_id
        && existing.stage == report.stage
        && existing.provider == report.provider
        && existing.model == report.model
        && existing.resolved_model == report.resolved_model
        && existing.started_at_ms == report.started_at_ms
        && Some(existing.completed_at_ms) == report.completed_at_ms
        && Some(existing.latency_ms) == report.latency_ms
        && existing.outcome == report.outcome
        && existing.tokens == report.tokens
}

fn validate_report(report: &ModelUsageReport) -> Result<(), StoreError> {
    if report.schema_version != ModelUsageReport::SCHEMA_VERSION
        || report.call_id.is_empty()
        || report.call_id.len() > 32
        || !report
            .call_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || report.provider.is_empty()
        || report.provider.len() > 64
        || report.model.is_empty()
        || report.model.len() > 128
        || report
            .resolved_model
            .as_ref()
            .is_some_and(|model| model.is_empty() || model.len() > 128)
        || report.started_at_ms < 0
        || report
            .completed_at_ms
            .is_some_and(|end| end < report.started_at_ms)
        || report
            .latency_ms
            .is_some_and(|latency| latency > 86_400_000)
        || report.outcome == ModelInteractionOutcome::InProgress
    {
        return Err(StoreError::InvalidInput("invalid model usage report"));
    }
    Ok(())
}

/// Imports bounded atomic spool files. Replays are harmless because call identities are unique.
pub fn ingest_usage_reports(
    store: &Store,
    checkpoint_root: &Path,
    id: TaskId,
    terminal_outcome: Option<ModelInteractionOutcome>,
) -> Result<(), StoreError> {
    let directory = checkpoint_root.join("model-usage").join(id.to_string());
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(StoreError::InvalidInput(
                "model usage directory is unreadable",
            ));
        }
    };
    let mut paths = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| StoreError::InvalidInput("model usage directory is unreadable"))?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    if paths.len() > MAX_REPORTS_PER_TASK {
        return Err(StoreError::InvalidInput("too many model usage reports"));
    }
    for path in paths {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| StoreError::InvalidInput("model usage report is unreadable"))?;
        if !metadata.file_type().is_file() {
            return Err(StoreError::InvalidInput(
                "model usage report must be a regular file",
            ));
        }
        let file = fs::File::open(&path)
            .map_err(|_| StoreError::InvalidInput("model usage report is unreadable"))?;
        let mut bytes = Vec::new();
        file.take(MAX_REPORT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| StoreError::InvalidInput("model usage report is unreadable"))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_REPORT_BYTES {
            return Err(StoreError::InvalidInput("model usage report is too large"));
        }
        let mut report: ModelUsageReport = serde_json::from_slice(&bytes)
            .map_err(|_| StoreError::InvalidInput("model usage report is malformed"))?;
        if report.outcome == ModelInteractionOutcome::InProgress {
            if store
                .model_usage_interaction(id, report.stage, &report.call_id)?
                .is_some()
            {
                continue;
            }
            let Some(outcome) = terminal_outcome else {
                continue;
            };
            let completed_at_ms = unix_millis()?;
            report.outcome = outcome;
            report.completed_at_ms = Some(completed_at_ms);
            report.latency_ms = Some(
                u64::try_from(completed_at_ms.saturating_sub(report.started_at_ms))
                    .unwrap_or(u64::MAX)
                    .min(86_400_000),
            );
        }
        store.record_model_usage(id, &report)?;
    }
    Ok(())
}

fn unix_millis() -> Result<i64, StoreError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(StoreError::ClockBeforeEpoch)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| StoreError::ClockOverflow)
}

#[cfg(test)]
mod tests {
    use super::{PRICED_MODEL, estimate, ingest_usage_reports, summarize};
    use crate::{
        development::{AgentProfile, RepositoryBinding},
        model_usage::ModelUsageInteraction,
        orchestration::{Store, StoreError},
    };
    use ayni_orchestrator_domain::{
        ModelInteractionOutcome, ModelUsageReport, ModelUsageStage, ProviderTokenUsage, TaskId,
    };
    use rusqlite::params;
    use std::path::Path;

    fn store_with_task(path: &Path, id: TaskId) -> Result<Store, Box<dyn std::error::Error>> {
        let store = Store::open(path, 3)?;
        let repository = RepositoryBinding {
            id: "sandbox".into(),
            owner: "ayni-oss".into(),
            repository: "sandbox".into(),
            installation_id: 1,
            base_branch: "main".into(),
            lock_sha256: "d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7".into(),
        };
        let profile = AgentProfile {
            id: "openai-gpt5".into(),
            provider: "openai".into(),
            model: "gpt-5".into(),
            secret_ref: "openai-api".into(),
        };
        store.connection.execute(
            "INSERT INTO repository_bindings(id, owner, repository, installation_id, base_branch, lock_sha256)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![repository.id, repository.owner, repository.repository, repository.installation_id, repository.base_branch, repository.lock_sha256],
        )?;
        store.connection.execute(
            "INSERT INTO agent_profiles(id, provider, model, secret_ref) VALUES (?1,?2,?3,?4)",
            params![
                profile.id,
                profile.provider,
                profile.model,
                profile.secret_ref
            ],
        )?;
        store.connection.execute(
            "INSERT INTO development_tasks(id, repository_id, profile_id, repository_snapshot,
             profile_snapshot, start_revision, request, acceptance_criteria, phase, created_at,
             updated_at) VALUES (?1,'sandbox','openai-gpt5',?2,?3,
             '8a555ada2f6e3ee41de2485ed13da508beaebaeb','change','[\"done\"]','coding',1,1)",
            params![
                id.to_string(),
                serde_json::to_string(&repository)?,
                serde_json::to_string(&profile)?
            ],
        )?;
        Ok(store)
    }

    fn report(call_id: &str, outcome: ModelInteractionOutcome) -> ModelUsageReport {
        ModelUsageReport {
            schema_version: ModelUsageReport::SCHEMA_VERSION,
            call_id: call_id.into(),
            stage: ModelUsageStage::Coding,
            provider: "openai".into(),
            model: "gpt-5".into(),
            resolved_model: Some(PRICED_MODEL.into()),
            started_at_ms: 1_000,
            completed_at_ms: Some(1_042),
            latency_ms: Some(42),
            outcome,
            tokens: ProviderTokenUsage::default(),
        }
    }

    fn interaction(call_id: &str, tokens: ProviderTokenUsage) -> ModelUsageInteraction {
        ModelUsageInteraction {
            call_id: call_id.into(),
            stage: ModelUsageStage::Coding,
            provider: "openai".into(),
            model: "gpt-5".into(),
            resolved_model: Some(PRICED_MODEL.into()),
            started_at_ms: 1,
            completed_at_ms: 2,
            latency_ms: 1,
            outcome: ModelInteractionOutcome::Succeeded,
            estimated_cost: estimate("openai", Some(PRICED_MODEL), tokens),
            tokens,
        }
    }

    #[test]
    fn aggregates_multiturn_usage_without_inventing_optional_values() {
        let interactions = [
            interaction(
                "call-1",
                ProviderTokenUsage {
                    input_tokens: Some(10),
                    output_tokens: Some(2),
                    total_tokens: Some(12),
                    ..ProviderTokenUsage::default()
                },
            ),
            interaction(
                "call-2",
                ProviderTokenUsage {
                    input_tokens: Some(20),
                    output_tokens: Some(3),
                    total_tokens: Some(23),
                    cached_input_tokens: Some(5),
                    ..ProviderTokenUsage::default()
                },
            ),
        ];
        let summary = summarize(interactions.iter());
        assert_eq!(summary.input_tokens, Some(30));
        assert_eq!(summary.output_tokens, Some(5));
        assert_eq!(summary.cached_input_tokens, None);
        assert_eq!(summary.reasoning_tokens, None);
    }

    #[test]
    fn fixed_pricing_version_produces_reproducible_estimate()
    -> Result<(), Box<dyn std::error::Error>> {
        let cost = estimate(
            "openai",
            Some(PRICED_MODEL),
            ProviderTokenUsage {
                input_tokens: Some(1_000_000),
                output_tokens: Some(100_000),
                cached_input_tokens: Some(200_000),
                reasoning_tokens: Some(50_000),
                total_tokens: Some(1_100_000),
            },
        )
        .ok_or("known pricing should have an estimate")?;
        assert_eq!(cost.amount_micros, 2_025_000);
        assert_eq!(cost.pricing_version, "openai-public-2025-08-07");
        assert_eq!(cost.label, "estimated");
        assert!(
            estimate(
                "openai",
                Some("gpt-5"),
                ProviderTokenUsage {
                    input_tokens: Some(1),
                    output_tokens: Some(1),
                    ..ProviderTokenUsage::default()
                }
            )
            .is_none()
        );
        assert!(
            estimate(
                "openai",
                None,
                ProviderTokenUsage {
                    input_tokens: Some(1),
                    output_tokens: Some(1),
                    ..ProviderTokenUsage::default()
                }
            )
            .is_none()
        );
        Ok(())
    }

    #[test]
    fn mixed_pricing_versions_do_not_produce_a_relabeled_aggregate() {
        let tokens = ProviderTokenUsage {
            input_tokens: Some(10),
            output_tokens: Some(2),
            total_tokens: Some(12),
            ..ProviderTokenUsage::default()
        };
        let first = interaction("call-1", tokens);
        let mut second = interaction("call-2", tokens);
        if let Some(cost) = &mut second.estimated_cost {
            cost.pricing_version = "future-table".into();
        }
        let summary = summarize([first, second].iter());
        assert_eq!(summary.estimated_cost_micros, None);
        assert_eq!(summary.estimated_cost_currency, None);
        assert_eq!(summary.pricing_version, None);
    }

    #[test]
    fn duplicate_and_restart_replay_create_one_usage_record()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let database = directory.path().join("state.sqlite");
        let id = TaskId::generate();
        let store = store_with_task(&database, id)?;
        let report = report("call-1", ModelInteractionOutcome::Failed);
        assert!(store.record_model_usage(id, &report)?);
        assert!(!store.record_model_usage(id, &report)?);
        drop(store);

        let reopened = Store::open(&database, 3)?;
        assert!(!reopened.record_model_usage(id, &report)?);
        let usage = reopened.task_model_usage(id)?;
        assert_eq!(usage.interactions.len(), 1);
        assert_eq!(
            usage.interactions[0].outcome,
            ModelInteractionOutcome::Failed
        );
        assert_eq!(usage.interactions[0].tokens.input_tokens, None);
        assert_eq!(
            usage.interactions[0].resolved_model.as_deref(),
            Some(PRICED_MODEL)
        );
        assert_eq!(usage.total.unavailable_interaction_count, 1);
        Ok(())
    }

    #[test]
    fn reused_call_identity_with_different_usage_is_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let id = TaskId::generate();
        let store = store_with_task(&directory.path().join("state.sqlite"), id)?;
        let first = report("call-1", ModelInteractionOutcome::Succeeded);
        store.record_model_usage(id, &first)?;
        let mut conflicting = first;
        conflicting.tokens.input_tokens = Some(1);
        assert!(matches!(
            store.record_model_usage(id, &conflicting),
            Err(StoreError::InvalidInput(_))
        ));
        Ok(())
    }

    #[test]
    fn unreadable_spool_is_retried_instead_of_treated_as_empty()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let id = TaskId::generate();
        let store = store_with_task(&directory.path().join("state.sqlite"), id)?;
        let usage_directory = directory.path().join("model-usage").join(id.to_string());
        std::fs::create_dir_all(usage_directory.parent().ok_or("usage root missing")?)?;
        std::fs::write(&usage_directory, b"not a directory")?;
        assert!(ingest_usage_reports(&store, directory.path(), id, None).is_err());

        std::fs::remove_file(&usage_directory)?;
        std::fs::create_dir_all(&usage_directory)?;
        std::fs::write(
            usage_directory.join("coding-call-1.json"),
            serde_json::to_vec(&report("call-1", ModelInteractionOutcome::Succeeded))?,
        )?;
        ingest_usage_reports(&store, directory.path(), id, None)?;
        assert_eq!(store.task_model_usage(id)?.interactions.len(), 1);
        Ok(())
    }

    #[test]
    fn terminal_reconciliation_retains_cancelled_interaction_with_unknown_usage()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let id = TaskId::generate();
        let database = directory.path().join("state.sqlite");
        let store = store_with_task(&database, id)?;
        let usage_directory = directory.path().join("model-usage").join(id.to_string());
        std::fs::create_dir_all(&usage_directory)?;
        let mut started = report("call-1", ModelInteractionOutcome::InProgress);
        started.completed_at_ms = None;
        started.latency_ms = None;
        std::fs::write(
            usage_directory.join("coding-call-1.json"),
            serde_json::to_vec(&started)?,
        )?;
        ingest_usage_reports(
            &store,
            directory.path(),
            id,
            Some(ModelInteractionOutcome::Cancelled),
        )?;
        ingest_usage_reports(
            &store,
            directory.path(),
            id,
            Some(ModelInteractionOutcome::Cancelled),
        )?;
        drop(store);
        let reopened = Store::open(&database, 3)?;
        ingest_usage_reports(
            &reopened,
            directory.path(),
            id,
            Some(ModelInteractionOutcome::Cancelled),
        )?;
        let usage = reopened.task_model_usage(id)?;
        assert_eq!(usage.interactions.len(), 1);
        assert_eq!(
            usage.interactions[0].outcome,
            ModelInteractionOutcome::Cancelled
        );
        assert_eq!(usage.interactions[0].tokens.total_tokens, None);
        Ok(())
    }
}
