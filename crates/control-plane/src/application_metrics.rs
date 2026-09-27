use crate::orchestration::{Store, StoreError};
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
    sync::atomic::{AtomicU64, Ordering},
};

const DURATION_BUCKETS_SECONDS: [i64; 10] = [1, 5, 15, 30, 60, 300, 900, 1_800, 3_600, 7_200];
const LATENCY_BUCKETS_MILLISECONDS: [i64; 7] = [100, 500, 1_000, 5_000, 10_000, 30_000, 60_000];
const DEVELOPMENT_PHASES: [&str; 11] = [
    "queued",
    "coding",
    "waiting_for_input",
    "validation_queued",
    "validating",
    "pr_queued",
    "preparing_pr",
    "completed",
    "failed",
    "cancelled",
    "unsupported",
];
const TERMINAL_OUTCOMES: [&str; 4] = ["completed", "failed", "cancelled", "unsupported"];

static ATTEMPT_RECONCILIATION_ERRORS: AtomicU64 = AtomicU64::new(0);
static DEVELOPMENT_RECONCILIATION_ERRORS: AtomicU64 = AtomicU64::new(0);
static PREPARATION_RECONCILIATION_ERRORS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub enum ReconcilerKind {
    Attempt,
    Development,
    Preparation,
}

pub fn record_reconciliation_error(kind: ReconcilerKind) {
    let counter = match kind {
        ReconcilerKind::Attempt => &ATTEMPT_RECONCILIATION_ERRORS,
        ReconcilerKind::Development => &DEVELOPMENT_RECONCILIATION_ERRORS,
        ReconcilerKind::Preparation => &PREPARATION_RECONCILIATION_ERRORS,
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

#[derive(Debug)]
struct TaskValue {
    phase: String,
    created_at: i64,
    updated_at: i64,
    lifecycle_complete: bool,
}

#[derive(Clone, Debug)]
struct LifecycleValue {
    event_time: i64,
    stage: String,
    outcome: String,
}

#[derive(Debug)]
struct UsageValue {
    stage: String,
    provider: String,
    model: String,
    outcome: String,
    latency_ms: i64,
    tokens: [Option<i64>; 5],
}

impl Store {
    pub(crate) fn application_metrics(&self) -> Result<String, StoreError> {
        let tasks = self.metric_tasks()?;
        let lifecycle = self.metric_lifecycle()?;
        let usage = self.metric_usage()?;
        Ok(render_metrics(&tasks, &lifecycle, &usage))
    }

    fn metric_tasks(&self) -> Result<Vec<TaskValue>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT phase, created_at, updated_at, lifecycle_complete
                 FROM development_tasks ORDER BY created_at, id",
            )
            .map_err(StoreError::Database)?;
        statement
            .query_map([], |row| {
                Ok(TaskValue {
                    phase: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    lifecycle_complete: row.get(3)?,
                })
            })
            .map_err(StoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Database)
    }

    fn metric_lifecycle(&self) -> Result<HashMap<String, Vec<LifecycleValue>>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT events.task_id, events.event_time, events.stage, events.outcome
                 FROM development_lifecycle_events events
                 INNER JOIN development_tasks tasks ON tasks.id=events.task_id
                 WHERE tasks.lifecycle_complete=1
                 ORDER BY events.task_id, events.event_id",
            )
            .map_err(StoreError::Database)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    LifecycleValue {
                        event_time: row.get(1)?,
                        stage: row.get(2)?,
                        outcome: row.get(3)?,
                    },
                ))
            })
            .map_err(StoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Database)?;
        let mut values: HashMap<String, Vec<LifecycleValue>> = HashMap::new();
        for (task_id, event) in rows {
            values.entry(task_id).or_default().push(event);
        }
        Ok(values)
    }

    fn metric_usage(&self) -> Result<Vec<UsageValue>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT stage, provider, model, outcome, latency_ms,
                        input_tokens, output_tokens, cached_input_tokens,
                        reasoning_tokens, total_tokens
                 FROM development_model_usage
                 ORDER BY stage, provider, model, outcome",
            )
            .map_err(StoreError::Database)?;
        statement
            .query_map([], |row| {
                Ok(UsageValue {
                    stage: row.get(0)?,
                    provider: row.get(1)?,
                    model: row.get(2)?,
                    outcome: row.get(3)?,
                    latency_ms: row.get(4)?,
                    tokens: [
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                    ],
                })
            })
            .map_err(StoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Database)
    }
}

fn render_metrics(
    tasks: &[TaskValue],
    lifecycle: &HashMap<String, Vec<LifecycleValue>>,
    usage: &[UsageValue],
) -> String {
    let mut output = String::new();
    render_task_metrics(&mut output, tasks);
    render_lifecycle_metrics(&mut output, lifecycle);
    render_usage_metrics(&mut output, usage);
    render_reconciliation_metrics(&mut output);
    output
}

fn render_task_metrics(output: &mut String, tasks: &[TaskValue]) {
    let _ = output.write_str(
        "# HELP orchestrator_development_tasks Current development tasks by phase.\n\
# TYPE orchestrator_development_tasks gauge\n",
    );
    for phase in DEVELOPMENT_PHASES {
        let count = tasks.iter().filter(|task| task.phase == phase).count();
        let _ = writeln!(
            output,
            "orchestrator_development_tasks{{phase=\"{phase}\"}} {count}"
        );
    }

    let _ = output.write_str(
        "# HELP orchestrator_development_task_outcomes_total Durable terminal development task outcomes.\n\
# TYPE orchestrator_development_task_outcomes_total counter\n",
    );
    for outcome in TERMINAL_OUTCOMES {
        let count = tasks.iter().filter(|task| task.phase == outcome).count();
        let _ = writeln!(
            output,
            "orchestrator_development_task_outcomes_total{{outcome=\"{outcome}\"}} {count}"
        );
    }

    let _ = output.write_str(
        "# HELP orchestrator_development_task_duration_seconds Total duration of terminal development tasks with complete lifecycle telemetry.\n\
# TYPE orchestrator_development_task_duration_seconds histogram\n",
    );
    for outcome in TERMINAL_OUTCOMES {
        let durations = tasks
            .iter()
            .filter(|task| task.phase == outcome && task.lifecycle_complete)
            .map(|task| task.updated_at.saturating_sub(task.created_at).max(0))
            .collect::<Vec<_>>();
        write_histogram(
            output,
            "orchestrator_development_task_duration_seconds",
            &format!("outcome=\"{outcome}\""),
            &durations,
            &DURATION_BUCKETS_SECONDS,
        );
    }
}

fn render_lifecycle_metrics(output: &mut String, lifecycle: &HashMap<String, Vec<LifecycleValue>>) {
    let mut durations: BTreeMap<(String, String), Vec<i64>> = BTreeMap::new();
    for events in lifecycle.values() {
        collect_stage_durations(events, &mut durations);
    }
    let _ = output.write_str(
        "# HELP orchestrator_development_stage_outcomes_total Durable terminal stage outcomes.\n\
# TYPE orchestrator_development_stage_outcomes_total counter\n\
# HELP orchestrator_development_stage_duration_seconds Development stage duration from queued or started to a terminal stage outcome.\n\
# TYPE orchestrator_development_stage_duration_seconds histogram\n",
    );
    for ((stage, outcome), values) in durations {
        let labels = format!(
            "stage=\"{}\",outcome=\"{}\"",
            escape_label(&stage),
            escape_label(&outcome)
        );
        let _ = writeln!(
            output,
            "orchestrator_development_stage_outcomes_total{{{labels}}} {}",
            values.len()
        );
        write_histogram(
            output,
            "orchestrator_development_stage_duration_seconds",
            &labels,
            &values,
            &DURATION_BUCKETS_SECONDS,
        );
    }
}

fn collect_stage_durations(
    events: &[LifecycleValue],
    durations: &mut BTreeMap<(String, String), Vec<i64>>,
) {
    if let Some((queue_index, queue)) = events
        .iter()
        .enumerate()
        .find(|(_, event)| event.stage == "queue" && event.outcome == "queued")
        && let Some(end) = events.get(queue_index + 1)
    {
        let outcome = if end.stage == "queue" {
            end.outcome.as_str()
        } else {
            "succeeded"
        };
        durations
            .entry(("queue".into(), outcome.into()))
            .or_default()
            .push(end.event_time.saturating_sub(queue.event_time).max(0));
    } else if let Some(terminal) = events.iter().find(|event| {
        event.stage == "queue"
            && matches!(
                event.outcome.as_str(),
                "failed" | "cancelled" | "unsupported"
            )
    }) {
        durations
            .entry(("queue".into(), terminal.outcome.clone()))
            .or_default()
            .push(0);
    }

    for (index, terminal) in events.iter().enumerate() {
        if terminal.stage == "queue"
            || !matches!(
                terminal.outcome.as_str(),
                "succeeded" | "blocked" | "failed" | "cancelled" | "unsupported"
            )
        {
            continue;
        }
        let start = events[..index].iter().rev().find(|event| {
            event.stage == terminal.stage && matches!(event.outcome.as_str(), "started" | "queued")
        });
        if let Some(start) = start {
            durations
                .entry((terminal.stage.clone(), terminal.outcome.clone()))
                .or_default()
                .push(terminal.event_time.saturating_sub(start.event_time).max(0));
        }
    }
}

fn render_usage_metrics(output: &mut String, usage: &[UsageValue]) {
    const TOKEN_TYPES: [&str; 5] = ["input", "output", "cached_input", "reasoning", "total"];
    let mut interactions: BTreeMap<(String, String, String), Vec<i64>> = BTreeMap::new();
    let mut errors: BTreeMap<(String, String, String), u64> = BTreeMap::new();
    let mut tokens: BTreeMap<(String, String, String, &'static str), i64> = BTreeMap::new();

    for value in usage {
        interactions
            .entry((
                value.stage.clone(),
                value.provider.clone(),
                value.outcome.clone(),
            ))
            .or_default()
            .push(value.latency_ms);
        if value.outcome == "failed" {
            *errors
                .entry((
                    value.stage.clone(),
                    value.provider.clone(),
                    value.model.clone(),
                ))
                .or_default() += 1;
        }
        for (token_type, count) in TOKEN_TYPES.into_iter().zip(value.tokens) {
            if let Some(count) = count {
                *tokens
                    .entry((
                        value.stage.clone(),
                        value.provider.clone(),
                        value.model.clone(),
                        token_type,
                    ))
                    .or_default() += count;
            }
        }
    }

    let _ = output.write_str(
        "# HELP orchestrator_model_interactions_total Provider model interactions.\n\
# TYPE orchestrator_model_interactions_total counter\n\
# HELP orchestrator_model_interaction_latency_milliseconds Provider model interaction latency.\n\
# TYPE orchestrator_model_interaction_latency_milliseconds histogram\n",
    );
    for ((stage, provider, outcome), latencies) in interactions {
        let labels = format!(
            "stage=\"{}\",provider=\"{}\",outcome=\"{}\"",
            escape_label(&stage),
            escape_label(&provider),
            escape_label(&outcome)
        );
        let _ = writeln!(
            output,
            "orchestrator_model_interactions_total{{{labels}}} {}",
            latencies.len()
        );
        write_histogram(
            output,
            "orchestrator_model_interaction_latency_milliseconds",
            &labels,
            &latencies,
            &LATENCY_BUCKETS_MILLISECONDS,
        );
    }

    let _ = output.write_str(
        "# HELP orchestrator_provider_errors_total Failed provider model interactions.\n\
# TYPE orchestrator_provider_errors_total counter\n",
    );
    for ((stage, provider, model), count) in errors {
        let _ = writeln!(
            output,
            "orchestrator_provider_errors_total{{stage=\"{}\",provider=\"{}\",model=\"{}\"}} {count}",
            escape_label(&stage),
            escape_label(&provider),
            escape_label(&model)
        );
    }

    let _ = output.write_str(
        "# HELP orchestrator_model_tokens_total Provider-reported tokens; cached input and reasoning are subsets of input and output.\n\
# TYPE orchestrator_model_tokens_total counter\n",
    );
    for ((stage, provider, model, token_type), count) in tokens {
        let _ = writeln!(
            output,
            "orchestrator_model_tokens_total{{stage=\"{}\",provider=\"{}\",model=\"{}\",type=\"{token_type}\"}} {count}",
            escape_label(&stage),
            escape_label(&provider),
            escape_label(&model)
        );
    }
}

fn render_reconciliation_metrics(output: &mut String) {
    let _ = output.write_str(
        "# HELP orchestrator_reconciliation_errors_total Reconciliation operations that returned an error.\n\
# TYPE orchestrator_reconciliation_errors_total counter\n",
    );
    for (reconciler, counter) in [
        ("attempt", &ATTEMPT_RECONCILIATION_ERRORS),
        ("development", &DEVELOPMENT_RECONCILIATION_ERRORS),
        ("preparation", &PREPARATION_RECONCILIATION_ERRORS),
    ] {
        let _ = writeln!(
            output,
            "orchestrator_reconciliation_errors_total{{reconciler=\"{reconciler}\"}} {}",
            counter.load(Ordering::Relaxed)
        );
    }
}

fn write_histogram(output: &mut String, name: &str, labels: &str, values: &[i64], buckets: &[i64]) {
    for bucket in buckets {
        let count = values.iter().filter(|value| **value <= *bucket).count();
        let _ = writeln!(output, "{name}_bucket{{{labels},le=\"{bucket}\"}} {count}");
    }
    let _ = writeln!(
        output,
        "{name}_bucket{{{labels},le=\"+Inf\"}} {}",
        values.len()
    );
    let _ = writeln!(output, "{name}_count{{{labels}}} {}", values.len());
    let _ = writeln!(
        output,
        "{name}_sum{{{labels}}} {}",
        values.iter().sum::<i64>()
    );
}

fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::{LifecycleValue, TaskValue, UsageValue, collect_stage_durations, render_metrics};
    use std::collections::{BTreeMap, HashMap};

    #[test]
    fn metrics_cover_lifecycle_usage_and_only_bounded_labels() {
        let tasks = vec![
            TaskValue {
                phase: "completed".into(),
                created_at: 10,
                updated_at: 42,
                lifecycle_complete: true,
            },
            TaskValue {
                phase: "cancelled".into(),
                created_at: 20,
                updated_at: 25,
                lifecycle_complete: true,
            },
        ];
        let lifecycle = HashMap::from([(
            "task-secret-id".into(),
            vec![
                LifecycleValue {
                    event_time: 10,
                    stage: "queue".into(),
                    outcome: "queued".into(),
                },
                LifecycleValue {
                    event_time: 12,
                    stage: "coding".into(),
                    outcome: "started".into(),
                },
                LifecycleValue {
                    event_time: 30,
                    stage: "coding".into(),
                    outcome: "succeeded".into(),
                },
            ],
        )]);
        let usage = vec![UsageValue {
            stage: "coding".into(),
            provider: "openai".into(),
            model: "gpt-5\"preview".into(),
            outcome: "failed".into(),
            latency_ms: 42,
            tokens: [Some(10), Some(2), None, Some(1), Some(12)],
        }];

        let metrics = render_metrics(&tasks, &lifecycle, &usage);
        assert!(metrics.contains("orchestrator_development_tasks{phase=\"completed\"} 1"));
        assert!(metrics.contains("orchestrator_development_stage_outcomes_total{stage=\"coding\",outcome=\"succeeded\"} 1"));
        assert!(metrics.contains("orchestrator_model_tokens_total{stage=\"coding\",provider=\"openai\",model=\"gpt-5\\\"preview\",type=\"total\"} 12"));
        assert!(metrics.contains("orchestrator_provider_errors_total"));
        assert!(
            metrics
                .contains("orchestrator_reconciliation_errors_total{reconciler=\"development\"}")
        );
        assert!(!metrics.contains("task-secret-id"));
        assert!(!metrics.contains("task_id="));
        assert!(!metrics.contains("repository="));
    }

    #[test]
    fn stage_durations_use_queue_and_started_boundaries() {
        let events = vec![
            LifecycleValue {
                event_time: 10,
                stage: "queue".into(),
                outcome: "queued".into(),
            },
            LifecycleValue {
                event_time: 12,
                stage: "coding".into(),
                outcome: "started".into(),
            },
            LifecycleValue {
                event_time: 17,
                stage: "coding".into(),
                outcome: "failed".into(),
            },
        ];
        let mut values = BTreeMap::new();
        collect_stage_durations(&events, &mut values);
        assert_eq!(
            values.get(&("queue".into(), "succeeded".into())),
            Some(&vec![2])
        );
        assert_eq!(
            values.get(&("coding".into(), "failed".into())),
            Some(&vec![5])
        );

        let unsupported = [LifecycleValue {
            event_time: 20,
            stage: "queue".into(),
            outcome: "unsupported".into(),
        }];
        collect_stage_durations(&unsupported, &mut values);
        assert_eq!(
            values.get(&("queue".into(), "unsupported".into())),
            Some(&vec![0])
        );
    }
}
