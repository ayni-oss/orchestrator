//! Credential-free output fixture for managed-source collection acceptance.
use std::{
    io::{self, Write as _},
    process::ExitCode,
    thread,
    time::Duration,
};

fn main() -> io::Result<ExitCode> {
    match std::env::args().nth(1).as_deref() {
        Some("bytes") => {
            io::stdout().write_all(
                b"same\nsame\n<img src=x onerror=globalThis.__rawExecuted=true>\x1b[2J\x00\xfftail",
            )?;
        }
        Some("sidecar") => println!("sidecar-output"),
        Some("init") => println!("init-output"),
        Some("fail") => {
            eprintln!("init-failure");
            return Ok(ExitCode::from(42));
        }
        Some("restart") => {
            println!("restart-output");
            thread::sleep(Duration::from_secs(1));
            return Ok(ExitCode::from(42));
        }
        Some("idle") => {
            println!("before-forced-delete");
            thread::sleep(Duration::from_secs(300));
        }
        Some("oom") => {
            let mut blocks = Vec::new();
            loop {
                // Touch every allocation so the container's memory limit, rather
                // than virtual address reservation, supplies the OOM observation.
                blocks.push(vec![42_u8; 1024 * 1024]);
                std::hint::black_box(&blocks);
            }
        }
        Some("quiet") => {}
        _ => return Ok(ExitCode::from(2)),
    }
    Ok(ExitCode::SUCCESS)
}
