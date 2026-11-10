//! Bounded progress for an authorized live build generation.
use ayni_orchestrator_events::runtime_preparation::{Binding, BuildMilestone};
use std::{collections::VecDeque, sync::Mutex};

const MAX_BUILDS: usize = 64;
static BUILDS: Mutex<VecDeque<Progress>> = Mutex::new(VecDeque::new());

struct Progress {
    operation_id: String,
    generation: u64,
    milestone: BuildMilestone,
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
    builds.push_back(Progress {
        operation_id: binding.operation_id.clone(),
        generation: binding.generation,
        milestone: previous.map_or(milestone, |entry| entry.milestone.max(milestone)),
    });
}

pub fn latest(operation: &str, generation: u64) -> Option<BuildMilestone> {
    BUILDS
        .lock()
        .ok()?
        .iter()
        .find(|entry| entry.operation_id == operation && entry.generation == generation)
        .map(|entry| entry.milestone)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            latest(&binding.operation_id, 7),
            Some(BuildMilestone::BuildImage)
        );
        assert_eq!(latest(&binding.operation_id, 8), None);
        assert_eq!(latest("different-operation", 7), None);
        binding.generation = 8;
        record(&binding, BuildMilestone::CopySource);
        assert_eq!(
            latest(&binding.operation_id, 8),
            Some(BuildMilestone::CopySource)
        );
    }
}
