//! Ayni's final Markdown retained outside the coding sandbox and bound to evidence.
use ayni_orchestrator_runtime::coding::QualityEvidence;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Report {
    pub signals_digest: String,
    pub commit: String,
    pub markdown: String,
}
impl Report {
    pub fn read(path: &std::path::Path) -> std::io::Result<Self> {
        // JSON escaping can expand the bounded 1 MiB CLI output sixfold.
        serde_json::from_slice(&ayni_orchestrator_runtime::runtime_execution::read_bounded(
            path, 8_388_608,
        )?)
        .map_err(std::io::Error::other)
    }
    pub fn matches(&self, evidence: &QualityEvidence) -> bool {
        self.signals_digest == evidence.signals_digest && self.commit == evidence.commit
    }
    pub fn comment(&self, attempt: &str) -> String {
        let heading = format!(
            "<!-- ayni-quality attempt:{attempt} digest:{} -->\n## Ayni quality\n\nChecked commit: `{}`\n\n",
            self.signals_digest, self.commit,
        );
        // GitHub issue comments accept at most 65,536 characters. Bound bytes
        // conservatively, preserve UTF-8, and disclose omitted report content.
        let limit = 60_000usize.saturating_sub(heading.len());
        if self.markdown.len() <= limit {
            return heading + &self.markdown;
        }
        let mut end = limit.saturating_sub(128);
        while !self.markdown.is_char_boundary(end) {
            end -= 1;
        }
        format!(
            "{heading}{}\n\n_Report truncated to fit GitHub's comment limit; full Markdown is retained with the worker evidence._\n",
            &self.markdown[..end]
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comment_retains_generated_markdown_and_bounds_unicode_reports() {
        let mut report = Report {
            signals_digest: "sha256:abc".into(),
            commit: "123".into(),
            markdown: "# ayni check\n<img src=\"https://assets.ayni.build/v1/cli/warn.svg\"> warn"
                .into(),
        };
        assert!(report.comment("attempt:1").ends_with(&report.markdown));
        assert_ne!(report.comment("attempt:1"), report.comment("attempt:2"));
        report.markdown = "✓".repeat(30_000);
        let body = report.comment("attempt:1");
        assert!(body.len() < 60_000);
        assert!(body.contains("truncated"));
        assert!(!report.matches(&QualityEvidence {
            artifact_path: "evidence/signals.json".into(),
            signals_digest: "sha256:other".into(),
            source_fingerprint: String::new(),
            commit: "123".into()
        }));
    }
}
