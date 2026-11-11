//! Observe existing protected file captures without replacing their sandbox boundary.
use crate::{Capture, capture};
use std::{
    fs::File,
    io::{self, Read as _, Seek as _, SeekFrom},
    path::Path,
};

pub struct FileCapture {
    stdout: File,
    stderr: File,
    out_offset: u64,
    err_offset: u64,
    action: String,
    capture: Option<Capture>,
}
impl FileCapture {
    /// # Errors
    /// Return unreadable protected capture files.
    pub fn open(stdout: &Path, stderr: &Path) -> io::Result<Self> {
        let capture = capture();
        let action = uuid::Uuid::new_v4().to_string();
        if let Some(c) = &capture {
            let _ = c.record(&action, "started", "", &[], None, None);
        }
        Ok(Self {
            stdout: File::open(stdout)?,
            stderr: File::open(stderr)?,
            out_offset: 0,
            err_offset: 0,
            action,
            capture,
        })
    }
    /// # Errors
    /// Return capture read failures; diagnostic writes never stop the command.
    pub fn poll(&mut self) -> io::Result<()> {
        for (file, offset, stream) in [
            (&mut self.stdout, &mut self.out_offset, "stdout"),
            (&mut self.stderr, &mut self.err_offset, "stderr"),
        ] {
            file.seek(SeekFrom::Start(*offset))?;
            let mut buffer = [0; 8192];
            // Bound each turn so a producer cannot starve deadline/cancellation checks.
            for _ in 0..32 {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                *offset += count as u64;
                if let Some(c) = &self.capture {
                    let _ = c.record(&self.action, "output", stream, &buffer[..count], None, None);
                }
            }
        }
        Ok(())
    }
    /// # Errors
    /// Drain remaining file bytes before recording completion.
    pub fn finish(mut self, status: std::process::ExitStatus) -> io::Result<()> {
        while self.out_offset < self.stdout.metadata()?.len()
            || self.err_offset < self.stderr.metadata()?.len()
        {
            self.poll()?;
        }
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt as _;
            status.signal()
        };
        #[cfg(not(unix))]
        let signal = None;
        if let Some(c) = &self.capture {
            let _ = c.record(
                &self.action,
                if status.success() {
                    "succeeded"
                } else {
                    "failed"
                },
                "",
                &[],
                status.code(),
                signal,
            );
            let _ = c.record(
                &self.action,
                "output_drained",
                "",
                &[],
                status.code(),
                signal,
            );
        }
        Ok(())
    }
}
