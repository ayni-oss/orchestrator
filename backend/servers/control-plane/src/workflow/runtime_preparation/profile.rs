//! Deployment-owned capabilities, frozen at admission rather than supplied by a browser.
use serde::{Deserialize, Serialize};
use tonic::Status;

pub(super) const PROFILE_ID: &str = "rootless-podman-vfs-userns-v1";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuntimeConfiguration {
    pub profile_id: String,
    pub qualified: bool,
    pub platform: String,
    pub worker_image: String,
    pub executor_image: String,
    pub verifier_image: String,
    pub trusted_cli_sha256: String,
    pub max_cpu_millis: u32,
    pub max_memory_mib: u32,
    pub max_storage_mib: u32,
    pub max_seconds: u32,
    pub max_builds: u32,
}

impl RuntimeConfiguration {
    pub fn from_environment() -> Result<Self, Status> {
        let value =
            std::env::var("ORCHESTRATOR_RUNTIME_PROFILE_JSON").map_err(|_| unavailable())?;
        Self::parse(&value)
    }

    fn parse(value: &str) -> Result<Self, Status> {
        if value.len() > 16_384 {
            return Err(unavailable());
        }
        let profile: Self = serde_json::from_str(value).map_err(|_| unavailable())?;
        profile.validate()?;
        Ok(profile)
    }

    fn validate(&self) -> Result<(), Status> {
        if self.profile_id != PROFILE_ID || !self.qualified {
            return Err(unavailable());
        }
        if !matches!(self.platform.as_str(), "linux/amd64" | "linux/arm64") {
            return Err(unavailable());
        }
        for image in [
            &self.worker_image,
            &self.executor_image,
            &self.verifier_image,
        ] {
            if !image_digest(image) {
                return Err(unavailable());
            }
        }
        if !digest(&self.trusted_cli_sha256) || !self.bounded_resources() {
            return Err(unavailable());
        }
        Ok(())
    }

    fn bounded_resources(&self) -> bool {
        (1..=64_000).contains(&self.max_cpu_millis)
            && (64..=131_072).contains(&self.max_memory_mib)
            && (64..=1_048_576).contains(&self.max_storage_mib)
            && (1..=86_400).contains(&self.max_seconds)
            && (1..=10).contains(&self.max_builds)
    }

    pub fn admit(&self, platform: &str, seconds: u32, builds: u32) -> Result<(), Status> {
        if platform != self.platform
            || seconds == 0
            || seconds > self.max_seconds
            || builds > self.max_builds
        {
            return Err(Status::invalid_argument(
                "runtime allowance exceeds the qualified deployment profile",
            ));
        }
        Ok(())
    }
}

fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    })
}

fn image_digest(value: &str) -> bool {
    ayni_orchestrator_events::runtime_preparation::valid_tool_image_reference(value)
}

fn unavailable() -> Status {
    Status::failed_precondition("qualified runtime preparation profile unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> RuntimeConfiguration {
        RuntimeConfiguration {
            profile_id: PROFILE_ID.into(),
            qualified: true,
            platform: "linux/amd64".into(),
            worker_image: format!("ghcr.io/ayni-oss/worker@sha256:{}", "1".repeat(64)),
            executor_image: format!("ghcr.io/ayni-oss/executor@sha256:{}", "2".repeat(64)),
            verifier_image: format!("ghcr.io/ayni-oss/verifier@sha256:{}", "3".repeat(64)),
            trusted_cli_sha256: format!("sha256:{}", "4".repeat(64)),
            max_cpu_millis: 2000,
            max_memory_mib: 4096,
            max_storage_mib: 16384,
            max_seconds: 3600,
            max_builds: 2,
        }
    }
    #[test]
    fn admission_requires_the_exact_qualified_profile_and_pinned_tooling() {
        let profile = fixture();
        assert!(profile.validate().is_ok());
        assert!(profile.admit("linux/amd64", 300, 1).is_ok());
        assert!(profile.admit("linux/arm64", 300, 1).is_err());
        assert!(profile.admit("linux/amd64", 3601, 1).is_err());
        assert!(profile.admit("linux/amd64", 300, 3).is_err());
        for field in [
            "profile_id",
            "worker_image",
            "executor_image",
            "verifier_image",
            "trusted_cli_sha256",
        ] {
            let mut value = serde_json::to_value(&profile)
                .unwrap_or_else(|error| panic!("fixture profile: {error}"));
            value[field] = "unsupported-or-mutable".into();
            assert!(RuntimeConfiguration::parse(&value.to_string()).is_err());
        }
        let mut value = serde_json::to_value(&profile)
            .unwrap_or_else(|error| panic!("fixture profile: {error}"));
        value["qualified"] = false.into();
        assert!(RuntimeConfiguration::parse(&value.to_string()).is_err());
        value["qualified"] = true.into();
        value["privileged_fallback"] = true.into();
        assert!(RuntimeConfiguration::parse(&value.to_string()).is_err());
    }
}
