//! Bounded progress for an authorized live build generation.
use ayni_orchestrator_events::runtime_preparation::{BUILD_STEPS, Binding, BuildMilestone};
use std::{collections::VecDeque, sync::Mutex, time::Instant};

const MAX_BUILDS: usize = 64;
static BUILDS: Mutex<VecDeque<Progress>> = Mutex::new(VecDeque::new());

struct Progress {
    operation_id: String,
    generation: u64,
    milestones: Vec<(BuildMilestone, Instant)>,
}

pub struct Snapshot {
    pub milestone: BuildMilestone,
    pub elapsed_ms: [Option<u64>; BUILD_STEPS.len()],
}

impl Progress {
    fn snapshot(&self, now: Instant) -> Option<Snapshot> {
        let &(milestone, _) = self.milestones.last()?;
        let elapsed_ms = std::array::from_fn(|index| {
            let step = BUILD_STEPS[index];
            let position = self
                .milestones
                .iter()
                .position(|&(value, _)| value == step)?;
            let started = self.milestones[position].1;
            let next_step = BUILD_STEPS
                .get(index + 1)
                .copied()
                .unwrap_or(BuildMilestone::ImageReady);
            let end = match self.milestones.get(position + 1) {
                Some(&(value, at)) if value == next_step => at,
                Some(_) => return None,
                None => now,
            };
            u64::try_from(end.saturating_duration_since(started).as_millis()).ok()
        });
        Some(Snapshot {
            milestone,
            elapsed_ms,
        })
    }
}

pub(super) fn record(binding: &Binding, milestone: BuildMilestone) {
    let Ok(mut builds) = BUILDS.lock() else {
        return;
    };
    let previous = builds
        .iter()
        .position(|entry| {
            entry.operation_id == binding.operation_id && entry.generation == binding.generation
        })
        .and_then(|index| builds.remove(index));
    if builds.len() == MAX_BUILDS {
        builds.pop_front();
    }
    let mut progress = previous.unwrap_or_else(|| Progress {
        operation_id: binding.operation_id.clone(),
        generation: binding.generation,
        milestones: Vec::with_capacity(BUILD_STEPS.len() + 1),
    });
    if progress
        .milestones
        .last()
        .is_none_or(|&(previous, _)| previous < milestone)
    {
        progress.milestones.push((milestone, Instant::now()));
    }
    builds.push_back(progress);
}

pub fn snapshot(operation: &str, generation: u64) -> Option<Snapshot> {
    BUILDS
        .lock()
        .ok()?
        .iter()
        .find(|entry| entry.operation_id == operation && entry.generation == generation)
        .and_then(|entry| entry.snapshot(Instant::now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reports_elapsed_time_only_for_observed_step_boundaries() {
        let now = Instant::now();
        let progress = Progress {
            operation_id: String::new(),
            generation: 1,
            milestones: vec![
                (BuildMilestone::CopySource, now),
                (BuildMilestone::VerifyInputs, now + Duration::from_secs(2)),
                (BuildMilestone::ExportImage, now + Duration::from_secs(5)),
            ],
        };
        assert_eq!(
            progress
                .snapshot(now + Duration::from_secs(8))
                .map(|value| value.elapsed_ms),
            Some([Some(2000), None, None, Some(3000), None])
        );
    }

    #[test]
    fn progress_is_monotonic_and_scoped_to_the_build_generation() {
        let mut binding = Binding {
            operation_id: "build-report-fixture".into(),
            stage: "build".into(),
            generation: 7,
            pod_uid: String::new(),
            command_digest: String::new(),
        };
        record(&binding, BuildMilestone::BuildImage);
        record(&binding, BuildMilestone::VerifyInputs);
        assert_eq!(
            snapshot(&binding.operation_id, 7).map(|value| value.milestone),
            Some(BuildMilestone::BuildImage)
        );
        assert!(snapshot(&binding.operation_id, 8).is_none());
        assert!(snapshot("different-operation", 7).is_none());
        binding.generation = 8;
        record(&binding, BuildMilestone::CopySource);
        assert_eq!(
            snapshot(&binding.operation_id, 8).map(|value| value.milestone),
            Some(BuildMilestone::CopySource)
        );
    }
}
