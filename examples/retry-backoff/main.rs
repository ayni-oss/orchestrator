//! Deliberately incorrect, standalone coding-agent evaluation fixture.
//!
//! This is not linked into the orchestrator. The agent's task is to repair the
//! calculation and add its own regression tests without changing this contract.

use std::{env, process::ExitCode};

fn retry_delay_seconds(attempt: u32) -> u32 {
    attempt.saturating_mul(2).min(60)
}

fn main() -> ExitCode {
    let Some(attempt) = env::args().nth(1).and_then(|value| value.parse::<u32>().ok()) else {
        eprintln!("usage: retry-backoff <positive-attempt-number>");
        return ExitCode::FAILURE;
    };
    if attempt == 0 {
        eprintln!("attempt number must be positive");
        return ExitCode::FAILURE;
    }
    println!("{}", retry_delay_seconds(attempt));
    ExitCode::SUCCESS
}
