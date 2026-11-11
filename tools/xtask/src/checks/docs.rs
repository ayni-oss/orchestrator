use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::{collections::BTreeSet, fs, path::Path};
use walkdir::WalkDir;

pub fn links() -> Result<()> {
    let diagrams = Regex::new(r"(?s)```mermaid\n(.*?)```")?;
    let fences = Regex::new(r"(?s)```.*?```")?;
    let links = Regex::new(r#"\]\((<[^>]+>|[^\s)]+)(?:\s+"[^"]*")?\)"#)?;
    let scheme = Regex::new(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")?;
    let root = std::env::current_dir()?;
    let mut errors = Vec::new();
    for filename in crate::command::files()? {
        let path = Path::new(&filename);
        if path.extension().is_none_or(|ext| ext != "md") || !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(path)?;
        for diagram in diagrams.captures_iter(&text) {
            let theme = if filename.starts_with("docs/site/") {
                ""
            } else {
                "  theme: neutral\n"
            };
            if !diagram[1].starts_with(&format!(
                "---\nconfig:\n{theme}  look: classic\n  layout: elk\n---\n"
            )) {
                errors.push(format!(
                    "{filename}: Mermaid diagram must use classic/elk and the correct theme"
                ));
            }
        }
        for link in links.captures_iter(&fences.replace_all(&text, "")) {
            let target = link[1].trim_matches(['<', '>']);
            if target.starts_with('#') || scheme.is_match(target) {
                continue;
            }
            let target = target.split('#').next().unwrap_or_default();
            let base = if target.starts_with('/') {
                root.join("docs")
            } else {
                root.join(path.parent().context("Markdown parent")?)
            };
            let decoded = percent_encoding::percent_decode_str(target).decode_utf8()?;
            let destination = base.join(decoded.trim_start_matches('/'));
            if !destination.exists() {
                errors.push(format!("{filename}: missing {target}"));
            }
        }
    }
    ensure!(errors.is_empty(), "{}", errors.join("\n"));
    println!("Markdown links and Mermaid styling passed");
    Ok(())
}

pub fn site() -> Result<()> {
    fn html_paths(root: &str, extension: &str) -> Result<BTreeSet<String>> {
        WalkDir::new(root)
            .into_iter()
            .map(|entry| -> Result<_> {
                let entry = entry?;
                Ok((entry.file_type().is_file()
                    && entry.path().extension().is_some_and(|e| e == extension))
                .then(|| {
                    entry
                        .path()
                        .strip_prefix(root)
                        .map(|p| p.with_extension("html").to_string_lossy().into_owned())
                })
                .transpose()?)
            })
            .filter_map(Result::transpose)
            .collect()
    }
    let expected = html_paths("docs/site", "md")?;
    let mut actual = html_paths("docs/.vitepress/dist", "html")?;
    actual.remove("404.html");
    ensure!(
        expected == actual,
        "site audience boundary differs: missing={:?} unexpected={:?}",
        expected.difference(&actual),
        actual.difference(&expected)
    );
    ensure!(
        fs::read_to_string("docs/.vitepress/dist/CNAME")?.trim() == "orchestrator.ayni.build",
        "unexpected published CNAME"
    );
    let home = fs::read_to_string("docs/.vitepress/dist/index.html")?;
    for required in [
        "ayni-brand-mark",
        "<svg",
        "favicon.svg",
        "From intent to reviewed delivery",
        "Project setup",
    ] {
        ensure!(home.contains(required), "homepage is missing {required}");
    }
    ensure!(
        Path::new("docs/.vitepress/dist/favicon.svg").is_file(),
        "missing favicon"
    );
    Ok(())
}
