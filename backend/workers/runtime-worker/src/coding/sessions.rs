//! Coding sessions share one durable turn budget and retain bounded prior context.
use super::{activity::ActivityLog, recovery};
use ayni_orchestrator_profile_ayni::coding::WorkerSpec;
use ayni_orchestrator_runtime::coding::{PublicationOutcome, WorkerPhase, WorkerProgress};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const CHECK_RESERVE_SECONDS: u64 = 180;
pub const PUBLICATION_RESERVE_SECONDS: u64 = 90;
pub const INSTRUCTIONS: &str = "Read AGENTS.md and repository instructions. Make only the requested changes within the frozen constraints. Use ayni_verify for focused feedback and ayni_check for full checking. Never weaken .ayni.toml or .ayni.lock to silence findings. Use commit_changes for coherent feature milestones and actual quality repairs, with meaningful Conventional Commit subjects. The worker always runs a mandatory full check after you finish; complete your implementation and explain what changed. Do not access provider APIs, push branches or create PRs; the worker handles delivery. If essential information is missing respond INPUT_REQUIRED: followed by one concrete question.";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionState {
    pub progress: WorkerProgress,
    pub prior_conversation: String,
    pub last_response: String,
    pub repair_stopped: bool,
}
impl Default for SessionState {
    fn default() -> Self {
        Self {
            progress: WorkerProgress {
                phase: WorkerPhase::Admission,
                coding_turns_consumed: 0,
                quality_round: 0,
                latest_full_check: None,
                publication: PublicationOutcome::Pending,
            },
            prior_conversation: String::new(),
            last_response: String::new(),
            repair_stopped: false,
        }
    }
}
#[derive(Clone)]
pub struct Context {
    pub spec: Arc<WorkerSpec>,
    pub workspace: PathBuf,
    pub output: PathBuf,
    pub state: Arc<tokio::sync::Mutex<SessionState>>,
    pub serial: Arc<tokio::sync::Mutex<()>>,
    pub activity: ActivityLog,
}
impl Context {
    pub fn remaining(&self) -> io::Result<Duration> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs();
        let deadline = u64::try_from(self.spec.limits.deadline_at).map_err(io::Error::other)?;
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "frozen task deadline exhausted",
            ));
        }
        Ok(Duration::from_secs(deadline - now))
    }
    pub fn coding_time(&self) -> io::Result<Duration> {
        self.remaining()?
            .checked_sub(Duration::from_secs(
                CHECK_RESERVE_SECONDS + PUBLICATION_RESERVE_SECONDS,
            ))
            .filter(|v| !v.is_zero())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "time reserved for final check and publication",
                )
            })
    }
    pub async fn persist(&self) -> io::Result<()> {
        let state = self.state.lock().await;
        recovery::write_json(&self.output.join("progress.json"), &state.progress)?;
        recovery::write_json(&self.output.join("session.json"), &*state)
    }
    pub async fn phase(&self, phase: WorkerPhase) -> io::Result<()> {
        self.state.lock().await.progress.phase = phase;
        self.persist().await
    }
    pub async fn save_source(&self) -> io::Result<()> {
        let context = self.clone();
        let state = context.state.lock().await.clone();
        tokio::task::spawn_blocking(move || {
            recovery::save(&context.spec, &context.workspace, &context.output, &state)
        })
        .await
        .map_err(io::Error::other)?
    }
}
/// Execute the configured coding adapter under the same frozen session contract.
pub async fn run(context: &Context, repair: Option<&str>) -> io::Result<Option<String>> {
    use super::agent_runner::{CodingAgentRunner, RigCodingAgent};
    RigCodingAgent.run(context, repair).await
}
