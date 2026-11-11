use anyhow::{Result, ensure};
use std::fs;

fn violation(path: &str, source: &str) -> bool {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let native = path.starts_with("apps/web/")
        || path.starts_with("clients/typescript/")
        || path.starts_with("docs/.vitepress/")
        || path == "dev/fixtures/runtime-preparation/source/index.js";
    if ["py", "sh", "bash", "zsh"].contains(&extension.as_str()) || path == "dev/ctl" {
        return true;
    }
    if ["js", "mjs", "cjs", "ts"].contains(&extension.as_str()) && !native {
        return true;
    }
    if source.starts_with("#!") && !source.starts_with("#![") && !native {
        return true;
    }
    if path.starts_with("apps/web/tests/") && source.contains("node:child_process") {
        return true;
    }
    let configuration =
        ["yaml", "yml"].contains(&extension.as_str()) || path.contains("Dockerfile");
    if !configuration || path.contains("/source/") {
        return false;
    }
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .any(|line| {
            [
                "python3 ",
                "python ",
                "node -e",
                "node, /fixture",
                "/bin/sh",
                "/bin/bash",
                "if [",
                "for bin in",
                "for attempt in",
                "case \"",
                "<<'PY'",
            ]
            .iter()
            .any(|pattern| line.contains(pattern))
        })
}

pub fn check() -> Result<()> {
    let mut violations = Vec::new();
    for path in crate::command::files()? {
        if !std::path::Path::new(&path).is_file() {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap_or_default();
        if violation(&path, &source) {
            violations.push(path);
        }
    }
    ensure!(
        violations.is_empty(),
        "supporting-language tooling remains:\n{}",
        violations.join("\n")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catches_extensionless_and_embedded_runners_but_keeps_application_inputs() {
        assert!(violation("dev/run", "#!/usr/bin/env python3\n"));
        assert!(violation(
            "dev/Dockerfile",
            "RUN python3 /fixture/server.py"
        ));
        assert!(violation(
            ".github/workflows/test.yml",
            "run: python3 -m unittest"
        ));
        assert!(violation(
            "apps/web/tests/bad.mjs",
            "import {spawn} from 'node:child_process'"
        ));
        assert!(!violation(
            "apps/web/tests/ui.mjs",
            "import test from 'node:test'"
        ));
        assert!(!violation(
            "dev/fixtures/runtime-preparation/source/index.js",
            "console.log('workload')"
        ));
        assert!(!violation(
            ".github/workflows/test.yml",
            "run: cargo xtask check tooling"
        ));
    }
}
