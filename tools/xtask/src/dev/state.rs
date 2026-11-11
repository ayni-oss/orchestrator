use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Status {
    pub pid: u32,
    pub session: String,
    pub checkout: PathBuf,
    pub branch: String,
    pub url: String,
    pub phase: String,
    pub message: String,
    pub gateway: String,
    pub active: BTreeMap<String, String>,
    pub images: BTreeMap<String, String>,
    pub updated_at_unix_ms: u128,
}

pub fn directory() -> Result<PathBuf> {
    let path =
        PathBuf::from(env::var_os("HOME").context("HOME is required for the per-user dev lock")?)
            .join(".local/state/ayni-orchestrator/dev");
    fs::create_dir_all(&path)?;
    Ok(path)
}
pub fn lock(directory: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("owner.lock"))?)
}
pub fn read(directory: &Path) -> Result<Status> {
    let mut status: Status = serde_json::from_slice(&fs::read(directory.join("status.json"))?)?;
    if let Ok(gateway) = fs::read_to_string(directory.join("gateway-status.txt")) {
        status.gateway = gateway;
    }
    Ok(status)
}
impl Status {
    pub fn save(&mut self, directory: &Path) -> Result<()> {
        self.updated_at_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let temporary = directory.join("status.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(temporary, directory.join("status.json"))?;
        Ok(())
    }
    pub fn phase(
        &mut self,
        directory: &Path,
        phase: &str,
        message: impl Into<String>,
    ) -> Result<()> {
        let message = message.into();
        if self.phase != phase || self.message != message {
            eprintln!("{phase}: {message}");
        }
        self.phase = phase.into();
        self.message = message;
        self.save(directory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_checkout_cannot_take_owned_lock() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let first = lock(directory.path())?;
        first.try_lock()?;
        let second = lock(directory.path())?;
        assert!(second.try_lock().is_err());
        drop(first);
        second.try_lock()?;
        Ok(())
    }
}
