use anyhow::{Result, ensure};
use std::fs;

pub fn check() -> Result<()> {
    let mut violations = Vec::new();
    for path in crate::command::files()? {
        if !std::path::Path::new(&path).is_file() {
            continue;
        }
        let script = std::path::Path::new(&path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("py"))
            || std::path::Path::new(&path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sh"))
            || path == "dev/ctl";
        let javascript = [".js", ".mjs", ".cjs", ".ts"]
            .iter()
            .any(|ext| path.ends_with(ext));
        let native = path.starts_with("apps/web/")
            || path.starts_with("clients/typescript/")
            || path.starts_with("docs/.vitepress/")
            || path == "dev/fixtures/runtime-preparation/source/index.js";
        let source = fs::read_to_string(&path).unwrap_or_default();
        let interpreter = source.starts_with("#!/usr/bin/env python")
            || source.starts_with("#!/usr/bin/env bash")
            || source.starts_with("#!/bin/sh");
        if script || (javascript && !native) || interpreter {
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
