//! Activate only protected image tooling; dependency preparation belongs to init.
use super::Failure;
use std::collections::BTreeMap;
const MAX_ENVIRONMENT_BYTES: usize = 64 * 1024;
fn parse(bytes: &[u8]) -> Result<BTreeMap<String, String>, Failure> {
    if bytes.len() > MAX_ENVIRONMENT_BYTES {
        return Err(Failure::ExecutionOutputExceeded);
    }
    let values: BTreeMap<String, String> =
        serde_json::from_slice(bytes).map_err(|_| Failure::CompatibilityFailed)?;
    if values.len() > 256
        || values.get("PATH").is_none_or(String::is_empty)
        || values.iter().any(|(key, value)| {
            key.is_empty()
                || key.len() > 128
                || key.as_bytes()[0].is_ascii_digit()
                || !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                || value.len() > 16 * 1024
                || value.contains('\0')
        })
    {
        return Err(Failure::CompatibilityFailed);
    }
    Ok(values)
}

#[cfg(target_os = "linux")]
pub(super) fn quality_command() -> Result<std::process::Command, Failure> {
    use std::{
        io::Read as _,
        process::{Command, Stdio},
    };
    // This is the public Mise JSON interface. The protected configuration owns
    // tooling activation; no repository shell or Ayni preparation marker is read.
    let mut child = Command::new("/usr/local/bin/mise")
        .args(["-C", "/etc/ayni", "env", "--json"])
        .env("CARGO_HOME", "/home/ayni/.cache/cargo")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Failure::CompatibilityFailed)?;
    let result = (|| {
        let stdout = child.stdout.take().ok_or(Failure::CompatibilityFailed)?;
        let mut bytes = Vec::new();
        stdout
            .take(MAX_ENVIRONMENT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Failure::CompatibilityFailed)?;
        let environment = parse(&bytes)?;
        if !child
            .wait()
            .map_err(|_| Failure::CompatibilityFailed)?
            .success()
        {
            return Err(Failure::CompatibilityFailed);
        }
        let mut command = Command::new("/usr/local/bin/ayni");
        command
            .args(["check", "--output", "json"])
            .envs(environment)
            // Preserve image tool resolution while restoring writable caller caches.
            .env("CARGO_HOME", "/tmp/cargo");
        Ok(command)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protected_environment_is_data_and_requires_bounded_valid_exports() {
        let values = parse(br#"{"PATH":"/opt/ayni/mise/bin:/usr/bin","VALUE":"$(unexecuted)"}"#)
            .unwrap_or_else(|error| panic!("valid environment: {error:?}"));
        assert_eq!(
            values.get("VALUE").map(String::as_str),
            Some("$(unexecuted)")
        );
        for bytes in [
            br"{}".as_slice(),
            br#"{"PATH":"/usr/bin","X":"\u0000"}"#,
            br#"{"PATH":"/usr/bin","BAD=KEY":"x"}"#,
            br#"{"PATH":false}"#,
        ] {
            assert!(parse(bytes).is_err());
        }
        assert!(parse(&vec![b' '; MAX_ENVIRONMENT_BYTES + 1]).is_err());
    }
}
