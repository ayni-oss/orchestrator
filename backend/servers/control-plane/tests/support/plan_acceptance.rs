use std::{
    io::Write as _,
    process::{Command, Stdio},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// Invoke the same installed client used by dev/ctl acceptance. No broker or
// executor is started: review and admission depend only on PostgreSQL authority.
pub async fn run(endpoint: String, retained: Option<Vec<u8>>) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        let mode = if retained.is_some() {
            "--verify-plans-stdin"
        } else {
            "--prepare-plans"
        };
        let mut child = Command::new(env!("CARGO_BIN_EXE_workflow-acceptance"))
            .args(["--endpoint", &endpoint, mode])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(bytes) = retained {
            child
                .stdin
                .take()
                .ok_or("client stdin missing")?
                .write_all(&bytes)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(format!(
                "plan acceptance client failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Ok(output.stdout)
    })
    .await?
}
