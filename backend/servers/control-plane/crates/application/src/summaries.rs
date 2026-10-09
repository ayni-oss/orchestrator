//! Product read models computed from complete input sets.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeReadiness {
    pub operational_profiles: u32,
    pub ready_profiles: u32,
}

impl RuntimeReadiness {
    #[must_use]
    pub const fn ready(self) -> bool {
        self.operational_profiles > 0 && self.ready_profiles == self.operational_profiles
    }
}

/// Every repository must have at least one operational, ready runtime.
#[must_use]
pub fn project_ready(members: &[RuntimeReadiness]) -> bool {
    !members.is_empty() && members.iter().all(|member| member.ready())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_requires_every_operational_profile_and_every_member() {
        let ready = RuntimeReadiness {
            operational_profiles: 1,
            ready_profiles: 1,
        };
        let partial = RuntimeReadiness {
            operational_profiles: 2,
            ready_profiles: 1,
        };
        assert!(project_ready(&[ready, ready]));
        assert!(!project_ready(&[ready, partial]));
        assert!(!project_ready(&[ready, RuntimeReadiness::default()]));
        assert!(!project_ready(&[]));
    }
}
