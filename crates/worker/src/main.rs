use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!(
        "the retired managed-check worker is not executable; use a controller-admitted runtime Job"
    );
    ExitCode::FAILURE
}
