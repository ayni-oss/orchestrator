//! Read presentation from Ayni's generated Markdown, without reevaluating policy.
use ayni_orchestrator_runtime::coding::QualityRow;
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn decorate(rows: &mut [QualityRow], artifact: &Value, markdown: &str) {
    let mut target = String::new();
    let mut presentations = HashMap::new();
    for line in markdown.lines() {
        if let Some(heading) = line.strip_prefix("## ")
            && let Some((label, _)) = heading.split_once(" — ")
        {
            label.clone_into(&mut target);
        }
        let columns: Vec<_> = line.split('|').map(str::trim).collect();
        if columns.len() != 6 || !columns[1].starts_with("**") {
            continue;
        }
        let kind = columns[2].trim_matches('*');
        let status = columns[4].split_whitespace().last().unwrap_or_default();
        if !matches!(status, "pass" | "warn" | "fail") {
            continue;
        }
        let summary = columns[3].trim_matches('`');
        presentations.insert(
            (target.clone(), kind.to_owned()),
            (status.to_owned(), summary.to_owned()),
        );
    }
    let Some(signals) = artifact["rows"].as_array() else {
        return;
    };
    for (row, signal) in rows.iter_mut().zip(signals) {
        let language = signal["language"].as_str().unwrap_or_default();
        let root = signal["scope"]["path"].as_str().unwrap_or(".");
        let root = if root == "." { "workspace" } else { root };
        let target = format!("{language} ({root})");
        if let Some((status, summary)) = presentations.get(&(target.clone(), row.kind.clone()))
            && (status != "fail") == row.passed
        {
            row.status.clone_from(status);
            row.summary.clone_from(summary);
            row.target = target;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn markdown_groups_preserve_warnings_and_distinct_target_summaries() {
        let mut rows = vec![
            QualityRow {
                kind: "size".into(),
                passed: true,
                status: String::new(),
                summary: String::new(),
                target: String::new(),
            },
            QualityRow {
                kind: "size".into(),
                passed: false,
                status: String::new(),
                summary: String::new(),
                target: String::new(),
            },
        ];
        let artifact = json!({"rows":[{"language":"rust","scope":{"path":"."}}, {"language":"svelte","scope":{"path":"apps/web"}}]});
        let markdown = "## svelte (apps/web) — 0/1 passing\n| **1** | **size** | `max_lines=1200 fail_count=1` | <img src=\"https://assets.ayni.build/v1/cli/fail.svg\"> fail |\n## rust (workspace) — 1/1 passing\n| **1** | **size** | `max_lines=900 fail_count=0` | <img src=\"https://assets.ayni.build/v1/cli/warn.svg\"> warn |";
        decorate(&mut rows, &artifact, markdown);
        assert_eq!(rows[0].status, "warn");
        assert_eq!(rows[0].summary, "max_lines=900 fail_count=0");
        assert_eq!(rows[1].status, "fail");
        assert_eq!(rows[1].target, "svelte (apps/web)");
        rows[1].passed = true;
        rows[1].status.clear();
        decorate(&mut rows, &artifact, markdown);
        assert!(
            rows[1].status.is_empty(),
            "presentation cannot contradict evidence"
        );
    }
}
