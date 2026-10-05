//! Atomic recovery snapshots, independent of model completion and Ayni success.
use super::{activity::ActivityLog, freeze_with_advisory};
use ayni_orchestrator_profile_ayni::coding::CodingRequest;
use ayni_orchestrator_runtime::activity::Stage;
use std::{fs, io, path::Path};

pub fn retain(request: &CodingRequest, workspace: &Path, output: &Path, activity: &ActivityLog) {
    activity.record(Stage::Checkpoint);
    if let Err(error) = save(request, workspace, output) {
        // No source changes is normal for inspection tools. Keep the last valid
        // checkpoint when a later edit violates constraints or saving fails.
        tracing::warn!(failure = ?crate::coding::activity::io_failure(&error), "recovery checkpoint not updated");
    }
}

pub fn save(request: &CodingRequest, workspace: &Path, output: &Path) -> io::Result<()> {
    let root = output.join("checkpoints");
    fs::create_dir_all(&root)?;
    let pending = tempfile::Builder::new()
        .prefix("pending-")
        .tempdir_in(&root)?;
    let report = freeze_with_advisory(
        request,
        workspace,
        pending.path(),
        "Recovered partial work from an interrupted coding attempt. Ayni completion is not verified; review and managed PR assessment are required.",
    )?;
    fs::write(
        pending.path().join("report.json"),
        serde_json::to_vec(&report)?,
    )?;
    let commit = report
        .commit
        .as_deref()
        .ok_or_else(|| io::Error::other("checkpoint commit missing"))?;
    let destination = root.join(commit);
    if !destination.exists() {
        fs::rename(pending.path(), &destination)?;
    }
    // Readers see either complete generation; never a report/bundle mismatch.
    fs::write(output.join("checkpoint.pending"), commit)?;
    fs::rename(output.join("checkpoint.pending"), output.join("checkpoint"))?;
    // Only the newest complete checkpoint is needed; it includes the whole diff.
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        if entry.path() != destination && entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}
