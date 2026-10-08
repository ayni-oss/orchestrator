use std::{error::Error, fs};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=ayni-release.json");
    let release: serde_json::Value = serde_json::from_slice(&fs::read("ayni-release.json")?)?;
    for (key, name) in [
        ("cli_version", "AYNI_CLI_VERSION"),
        ("linux_amd64_archive_sha256", "AYNI_ARCHIVE_SHA256"),
        ("executor_image", "AYNI_EXECUTOR_IMAGE"),
        ("docker_client_image", "AYNI_DOCKER_CLIENT_IMAGE"),
        ("signals_schema_version", "AYNI_SIGNALS_SCHEMA_VERSION"),
    ] {
        let value = release[key].as_str().ok_or("missing Ayni release field")?;
        if value.is_empty() || value.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err("invalid Ayni release field".into());
        }
        println!("cargo:rustc-env={name}={value}");
    }
    Ok(())
}
