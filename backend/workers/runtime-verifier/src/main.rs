//! Credential-free content verification inside an immutable candidate image.
use ayni_orchestrator_runtime_verifier::{Evidence, Options, verify};
use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let evidence =
        Options::parse(env::args().skip(1)).map_or_else(Evidence::invalid, |v| verify(&v));
    if let Ok(json) = serde_json::to_string(&evidence) {
        println!("{json}");
    } else {
        println!(r#"{{"state":"invalid","reason":"evidence_unavailable"}}"#);
        return ExitCode::from(2);
    }
    if evidence.state == "content_verified" {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}
