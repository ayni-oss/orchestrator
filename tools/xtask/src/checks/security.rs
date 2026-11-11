use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde_json::Value;
use std::fs;

pub fn check() -> Result<()> {
    let action = Regex::new(r"@([a-f0-9]{40})\s+#\s+(v[0-9]+\.[0-9]+\.[0-9]+|shared CI)$")?;
    for file in crate::command::files()?.iter().filter(|p| {
        p.starts_with(".github/workflows/")
            && (std::path::Path::new(&p)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("yml"))
                || std::path::Path::new(&p)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml")))
    }) {
        let source = fs::read_to_string(file)?;
        ensure!(
            !source.contains("pull_request_target"),
            "{file}: pull_request_target is forbidden"
        );
        for line in source.lines() {
            if let Some((_, value)) = line
                .trim_start()
                .trim_start_matches("- ")
                .split_once("uses:")
            {
                let value = value.trim();
                ensure!(
                    value.starts_with("./") || action.is_match(value),
                    "{file}: action must have immutable SHA and release annotation: {value}"
                );
                if value.ends_with("shared CI") {
                    ensure!(
                        value.starts_with("ayni-oss/ayni/.github/actions/"),
                        "unexpected shared action owner"
                    );
                }
            }
        }
    }
    let pr = fs::read_to_string(".github/workflows/pr-validation.yml")?;
    ensure!(
        pr.contains("permissions: {}") && pr.contains("persist-credentials: false"),
        "PR permissions/checkout credentials are unsafe"
    );
    ensure!(
        !Regex::new(r"packages:\s*write")?.is_match(&pr),
        "PR validation may not write packages"
    );
    let release: Value = serde_json::from_slice(&fs::read("tools/xtask/ayni-tool.json")?)?;
    for (key, length) in [
        ("source_revision", 40),
        ("linux_arm64_cli_sha256", 64),
        ("linux_amd64_archive_sha256", 64),
        ("linux_arm64_archive_sha256", 64),
    ] {
        let value = release[key].as_str().context("missing release pin")?;
        ensure!(
            value.len() == length
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid {key}"
        );
    }
    ensure!(
        !release["cli_version"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "CLI version is not pinned"
    );
    ensure!(
        Regex::new(r"^ghcr.io/ayni-oss/ayni-builder@sha256:[a-f0-9]{64}$")?.is_match(
            release["linux_arm64_executor_image"]
                .as_str()
                .unwrap_or_default()
        ),
        "executor is not pinned"
    );
    for file in [
        "backend/servers/control-plane/Cargo.toml",
        "backend/workers/runtime-preparation/Cargo.toml",
        "backend/workers/runtime-verifier/Cargo.toml",
    ] {
        let manifest: toml::Value = toml::from_str(&fs::read_to_string(file)?)?;
        for name in ["ayni-core", "ayni-environment"] {
            let dep = &manifest["dependencies"][name];
            ensure!(
                dep["git"].as_str() == Some("https://github.com/ayni-oss/ayni.git")
                    && dep["rev"].as_str() == release["source_revision"].as_str(),
                "{file}: {name} differs from admitted Ayni source"
            );
        }
    }
    println!("GitHub action permissions and admitted source pins passed");
    Ok(())
}
