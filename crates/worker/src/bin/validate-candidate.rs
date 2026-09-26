//! Independent managed validation of a frozen Git candidate bundle.

use ayni_orchestrator_domain::{
    ChangeConstraints, MAX_CHANGED_PATH_BYTES, MAX_CHANGED_PATHS, repository_path_is_valid,
};
use quote::ToTokens;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    thread,
    time::Duration,
};
use syn::{parse::Parser, spanned::Spanned};
use tracing::error;

#[derive(Serialize)]
struct Artifact {
    name: &'static str,
    sha256: String,
}

#[derive(Serialize)]
struct Manifest {
    schema_version: u8,
    start_revision: String,
    candidate_commit: String,
    lock_sha256: String,
    change_constraints: ChangeConstraints,
    outcome: &'static str,
    check_exit_code: i32,
    policy_edited: bool,
    tests_edited: bool,
    scope_violations: Vec<String>,
    changed_paths: Vec<String>,
    preparation_id: Option<String>,
    preparation_reused: bool,
    artifacts: Vec<Artifact>,
}

struct ValidationInput {
    candidate_bundle: PathBuf,
    candidate_commit: String,
    start_revision: String,
    expected_lock: String,
    workspace: PathBuf,
    evidence: PathBuf,
    change_constraints: ChangeConstraints,
    preparation: Option<PathBuf>,
    preparation_id: Option<String>,
}

#[derive(Deserialize)]
struct PreparationManifest {
    schema_version: u8,
    preparation_id: String,
    revision: String,
    lock_sha256: String,
    image_tag: String,
    image_id: String,
    image_archive_sha256: String,
    build_record_sha256: String,
}

impl ValidationInput {
    fn from_environment() -> Result<Self, io::Error> {
        Ok(Self {
            candidate_bundle: PathBuf::from(required("DEVELOPMENT_CANDIDATE_BUNDLE")?),
            candidate_commit: required("DEVELOPMENT_CANDIDATE_COMMIT")?,
            start_revision: required("DEVELOPMENT_START_REVISION")?,
            expected_lock: required("DEVELOPMENT_LOCK_SHA256")?,
            workspace: PathBuf::from(required("DEVELOPMENT_WORKSPACE")?),
            evidence: PathBuf::from(required("DEVELOPMENT_EVIDENCE_DIR")?),
            change_constraints: serde_json::from_str(&required("DEVELOPMENT_CHANGE_CONSTRAINTS")?)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
            preparation: env::var("DEVELOPMENT_PREPARATION_DIR")
                .ok()
                .map(PathBuf::from),
            preparation_id: env::var("DEVELOPMENT_PREPARATION_ID").ok(),
        })
    }
}

fn required(name: &str) -> Result<String, io::Error> {
    env::var(name).map_err(|_| io::Error::new(io::ErrorKind::NotFound, format!("missing {name}")))
}

fn command(directory: &Path, program: &str, args: &[&str], log: &Path) -> Result<i32, io::Error> {
    let output = Command::new(program)
        .args(args)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    fs::write(log.with_extension("stdout"), &output.stdout)?;
    fs::write(log.with_extension("stderr"), &output.stderr)?;
    Ok(output.status.code().unwrap_or(1))
}

fn checked(directory: &Path, program: &str, args: &[&str], log: &Path) -> Result<(), io::Error> {
    let status = command(directory, program, args, log)?;
    if status != 0 {
        return Err(io::Error::other(format!(
            "{program} step {} exited {status}",
            log.display()
        )));
    }
    Ok(())
}

fn checked_stream(
    directory: &Path,
    program: &str,
    args: &[&str],
    log: &Path,
) -> Result<(), io::Error> {
    let stdout = fs::File::create(log.with_extension("stdout"))?;
    let stderr = fs::File::create(log.with_extension("stderr"))?;
    let status = Command::new(program)
        .args(args)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "{program} step {} exited {}",
            log.display(),
            status.code().unwrap_or(1)
        )));
    }
    Ok(())
}

fn sha256(path: &Path) -> Result<String, io::Error> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}

fn dedicated_test_path(path: &str) -> bool {
    let path = Path::new(path);
    if path.components().any(|component| {
        component.as_os_str().to_str().is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "test" | "tests" | "spec" | "specs" | "__tests__"
            )
        })
    }) {
        return true;
    }
    path.file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let name = name.to_ascii_lowercase();
            matches!(name.as_str(), "test" | "tests" | "spec" | "specs")
                || name.starts_with("test_")
                || name.starts_with("spec_")
                || name.ends_with("_test")
                || name.ends_with("_spec")
        })
}

fn cfg_mentions_test(meta: &syn::Meta, negated: bool) -> bool {
    match meta {
        syn::Meta::Path(path) => path.is_ident("test") && !negated,
        syn::Meta::List(list) => {
            let negated = if list.path.is_ident("not") {
                !negated
            } else {
                negated
            };
            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
                .is_ok_and(|nested| nested.iter().any(|meta| cfg_mentions_test(meta, negated)))
        }
        syn::Meta::NameValue(_) => false,
    }
}

fn is_cfg_test(attribute: &syn::Attribute) -> bool {
    let syn::Meta::List(cfg) = &attribute.meta else {
        return false;
    };
    cfg.path.is_ident("cfg")
        && syn::parse2::<syn::Meta>(cfg.tokens.clone())
            .is_ok_and(|meta| cfg_mentions_test(&meta, false))
}

fn is_test_function_attribute(attribute: &syn::Attribute) -> bool {
    attribute.path().segments.last().is_some_and(|segment| {
        matches!(
            segment.ident.to_string().as_str(),
            "test" | "rstest" | "test_case"
        )
    })
}

fn item_attributes(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        syn::Item::ExternCrate(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::ForeignMod(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Macro(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Static(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::TraitAlias(item) => &item.attrs,
        syn::Item::Type(item) => &item.attrs,
        syn::Item::Union(item) => &item.attrs,
        syn::Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

fn test_item_fingerprint(
    item: &syn::Item,
    attributes: &[syn::Attribute],
    source: &str,
    line_starts: &[usize],
) -> String {
    let span = item.span();
    let start = span.start();
    let end = span.end();
    let exact = line_starts
        .get(start.line.saturating_sub(1))
        .zip(line_starts.get(end.line.saturating_sub(1)))
        .and_then(|(line_start, line_end)| {
            source.get(line_start + start.column..line_end + end.column)
        })
        .map_or_else(|| item.to_token_stream().to_string(), str::to_owned);
    format!(
        "{}|{exact}",
        attributes
            .iter()
            .map(ToTokens::to_token_stream)
            .map(|tokens| tokens.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn collect_test_items(
    items: &[syn::Item],
    source: &str,
    line_starts: &[usize],
    tests: &mut Vec<String>,
) {
    for item in items {
        let attributes = item_attributes(item);
        if attributes.iter().any(is_cfg_test)
            || matches!(item, syn::Item::Fn(_)) && attributes.iter().any(is_test_function_attribute)
        {
            tests.push(test_item_fingerprint(item, attributes, source, line_starts));
            continue;
        }
        if let syn::Item::Mod(module) = item
            && let Some((_, items)) = &module.content
        {
            collect_test_items(items, source, line_starts, tests);
        }
    }
}

fn rust_test_fingerprint(source: &str) -> Result<Vec<String>, syn::Error> {
    let file = syn::parse_file(source)?;
    let mut line_starts = vec![0];
    line_starts.extend(
        source
            .bytes()
            .enumerate()
            .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
    );
    let mut tests = Vec::new();
    collect_test_items(&file.items, source, &line_starts, &mut tests);
    Ok(tests)
}

fn file_at_revision(
    workspace: &Path,
    revision: &str,
    path: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["show", &format!("{revision}:{path}")])
        .current_dir(workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if !output.status.success() {
        return Ok(String::new());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn candidate_file(workspace: &Path, path: &str) -> Result<String, io::Error> {
    match fs::read_to_string(workspace.join(path)) {
        Ok(source) => Ok(source),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

fn edited_test_paths(
    workspace: &Path,
    start_revision: &str,
    changed_paths: &[String],
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut edited = Vec::new();
    for path in changed_paths {
        if dedicated_test_path(path) {
            edited.push(path.clone());
            continue;
        }
        if Path::new(path).extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let baseline = file_at_revision(workspace, start_revision, path)?;
        let candidate = candidate_file(workspace, path)?;
        let baseline_tests = rust_test_fingerprint(&baseline).map_err(|error| {
            format!("could not classify baseline Rust tests in {path}: {error}")
        })?;
        let candidate_tests = rust_test_fingerprint(&candidate).map_err(|error| {
            format!("could not classify candidate Rust tests in {path}: {error}")
        })?;
        if baseline_tests != candidate_tests {
            edited.push(path.clone());
        }
    }
    edited.sort();
    edited.dedup();
    Ok(edited)
}

fn scope_violations(
    constraints: &ChangeConstraints,
    changed_paths: &[String],
    edited_tests: &[String],
) -> Vec<String> {
    let mut violations = changed_paths
        .iter()
        .filter_map(|path| {
            constraints.protecting_path(path).map(|protected| {
                format!("protected path {protected:?} covers changed path {path:?}")
            })
        })
        .collect::<Vec<_>>();
    if !constraints.allow_test_edits {
        violations.extend(
            edited_tests
                .iter()
                .map(|path| format!("test edits are not allowed in {path:?}")),
        );
    }
    violations.sort();
    violations.dedup();
    violations
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let input = ValidationInput::from_environment()?;
    input.change_constraints.validate()?;
    let (lock_digest, changed_paths) = checkout_candidate(&input)?;
    let policy_edited = changed_paths.iter().any(|path| path == ".ayni.toml");
    let dependency_edited = changed_paths.iter().any(|path| {
        matches!(
            path.as_str(),
            ".ayni.lock" | "Cargo.toml" | "Cargo.lock" | "rust-toolchain.toml"
        )
    });
    let edited_tests = edited_test_paths(&input.workspace, &input.start_revision, &changed_paths)?;
    let tests_edited = !edited_tests.is_empty();
    let scope_violations =
        scope_violations(&input.change_constraints, &changed_paths, &edited_tests);
    if policy_edited {
        write_manifest(
            &input.evidence,
            &Manifest {
                schema_version: 2,
                start_revision: input.start_revision,
                candidate_commit: input.candidate_commit,
                lock_sha256: lock_digest,
                change_constraints: input.change_constraints.clone(),
                outcome: "policy_rejected",
                check_exit_code: -1,
                policy_edited,
                tests_edited,
                scope_violations,
                changed_paths,
                preparation_id: input.preparation_id.clone(),
                preparation_reused: false,
                artifacts: Vec::new(),
            },
        )?;
        return Err("candidate edited required Ayni policy".into());
    }
    if dependency_edited {
        write_manifest(
            &input.evidence,
            &Manifest {
                schema_version: 2,
                start_revision: input.start_revision.clone(),
                candidate_commit: input.candidate_commit.clone(),
                lock_sha256: lock_digest,
                change_constraints: input.change_constraints.clone(),
                outcome: "dependency_change_unsupported",
                check_exit_code: -1,
                policy_edited: false,
                tests_edited,
                scope_violations,
                changed_paths,
                preparation_id: input.preparation_id.clone(),
                preparation_reused: false,
                artifacts: Vec::new(),
            },
        )?;
        return Err("dependency evolution is unsupported by the pilot profile".into());
    }
    if !scope_violations.is_empty() {
        write_manifest(
            &input.evidence,
            &Manifest {
                schema_version: 2,
                start_revision: input.start_revision.clone(),
                candidate_commit: input.candidate_commit.clone(),
                lock_sha256: lock_digest,
                change_constraints: input.change_constraints.clone(),
                outcome: "scope_rejected",
                check_exit_code: -1,
                policy_edited: false,
                tests_edited,
                scope_violations,
                changed_paths,
                preparation_id: input.preparation_id.clone(),
                preparation_reused: false,
                artifacts: Vec::new(),
            },
        )?;
        return Err("candidate violated frozen task change constraints".into());
    }
    let (exit, preparation_reused) = run_managed_check(&input)?;
    retain_result(
        &input,
        lock_digest,
        changed_paths,
        tests_edited,
        exit,
        preparation_reused,
    )
}

fn checkout_candidate(
    input: &ValidationInput,
) -> Result<(String, Vec<String>), Box<dyn std::error::Error>> {
    let workspace = &input.workspace;
    let evidence = &input.evidence;
    fs::create_dir_all(workspace)?;
    fs::create_dir_all(evidence)?;
    checked(
        workspace,
        "git",
        &["init", "--quiet"],
        &evidence.join("git-init"),
    )?;
    checked(
        workspace,
        "git",
        &[
            "fetch",
            input
                .candidate_bundle
                .to_str()
                .ok_or("invalid bundle path")?,
            "HEAD",
        ],
        &evidence.join("git-fetch"),
    )?;
    checked(
        workspace,
        "git",
        &["checkout", "--detach", &input.candidate_commit],
        &evidence.join("git-checkout"),
    )?;
    checked(
        workspace,
        "git",
        &[
            "merge-base",
            "--is-ancestor",
            &input.start_revision,
            &input.candidate_commit,
        ],
        &evidence.join("git-ancestry"),
    )?;
    checked(
        workspace,
        "git",
        &["rev-parse", "HEAD"],
        &evidence.join("git-revision"),
    )?;
    ensure_revision(evidence, &input.candidate_commit)?;
    let lock_digest = sha256(&workspace.join(".ayni.lock"))?;
    if lock_digest != input.expected_lock {
        return Err("candidate changed the committed Ayni lock".into());
    }
    checked_stream(
        workspace,
        "git",
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            &input.start_revision,
            &input.candidate_commit,
        ],
        &evidence.join("git-diff-paths"),
    )?;
    let changed_paths = read_changed_paths(evidence)?;
    checked_stream(
        workspace,
        "git",
        &[
            "diff",
            "--name-status",
            "--no-renames",
            &input.start_revision,
            &input.candidate_commit,
        ],
        &evidence.join("git-diff"),
    )?;
    checked_stream(
        workspace,
        "git",
        &[
            "diff",
            "--function-context",
            "--no-ext-diff",
            &input.start_revision,
            &input.candidate_commit,
        ],
        &evidence.join("git-diff-patch"),
    )?;
    Ok((lock_digest, changed_paths))
}

fn read_changed_paths(evidence: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let path = evidence.join("git-diff-paths.stdout");
    let bytes = fs::read(&path)?;
    if bytes.len() > MAX_CHANGED_PATH_BYTES + MAX_CHANGED_PATHS {
        fs::remove_file(path)?;
        return Err("candidate changed paths exceed the supported byte limit".into());
    }
    let mut changed_paths = bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8(path.to_vec()))
        .collect::<Result<Vec<_>, _>>()?;
    if changed_paths.len() > MAX_CHANGED_PATHS
        || changed_paths.iter().map(String::len).sum::<usize>() > MAX_CHANGED_PATH_BYTES
    {
        fs::remove_file(path)?;
        return Err("candidate changed paths exceed supported scope accounting limits".into());
    }
    if let Some(path) = changed_paths
        .iter()
        .find(|path| !repository_path_is_valid(path))
    {
        return Err(format!("candidate changed an unsupported repository path: {path:?}").into());
    }
    changed_paths.sort();
    changed_paths.dedup();
    Ok(changed_paths)
}

fn ensure_revision(evidence: &Path, expected: &str) -> Result<(), Box<dyn std::error::Error>> {
    let actual = fs::read_to_string(evidence.join("git-revision.stdout"))?;
    if actual.trim() != expected {
        return Err("bundle did not produce the frozen candidate commit".into());
    }
    Ok(())
}

fn run_managed_check(input: &ValidationInput) -> Result<(i32, bool), Box<dyn std::error::Error>> {
    let workspace = &input.workspace;
    let evidence = &input.evidence;
    let mut engine_ready = false;
    for _ in 0..60 {
        if command(
            workspace,
            "docker",
            &["version"],
            &evidence.join("docker-ready"),
        )? == 0
        {
            engine_ready = true;
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }
    if !engine_ready {
        return Err("isolated Docker engine did not become ready".into());
    }
    let reused = if let Some(preparation) = input.preparation.as_deref() {
        restore_preparation(input, preparation)?;
        true
    } else {
        checked(
            workspace,
            "ayni",
            &[
                "env",
                "build",
                "--repo-root",
                workspace.to_str().ok_or("invalid workspace")?,
            ],
            &evidence.join("ayni-build"),
        )?;
        false
    };
    checked(
        workspace,
        "ayni",
        &[
            "env",
            "doctor",
            "--repo-root",
            workspace.to_str().ok_or("invalid workspace")?,
        ],
        &evidence.join("ayni-doctor"),
    )?;
    let config = workspace.join(".ayni.toml");
    let exit = command(
        workspace,
        "ayni",
        &[
            "check",
            "--config",
            config.to_str().ok_or("invalid config")?,
        ],
        &evidence.join("ayni-check"),
    )?;
    Ok((exit, reused))
}

fn restore_preparation(
    input: &ValidationInput,
    directory: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read(directory.join("manifest.json"))?;
    let manifest: PreparationManifest = serde_json::from_slice(&bytes)?;
    let expected_id = input
        .preparation_id
        .as_deref()
        .ok_or("preparation ID is absent")?;
    if manifest.schema_version != 1
        || manifest.preparation_id != expected_id
        || manifest.revision != input.start_revision
        || manifest.lock_sha256 != input.expected_lock
    {
        return Err("prepared environment does not match the frozen task inputs".into());
    }
    let archive = directory.join("environment-image.tar");
    let build = directory.join("build.json");
    if stream_sha256(&archive)? != manifest.image_archive_sha256
        || sha256(&build)? != manifest.build_record_sha256
    {
        return Err("prepared environment artifact digest differs".into());
    }
    checked(
        &input.workspace,
        "docker",
        &[
            "image",
            "load",
            "--input",
            archive.to_str().ok_or("invalid archive path")?,
        ],
        &input.evidence.join("preparation-load"),
    )?;
    let inspect = Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{.Id}}",
            &manifest.image_tag,
        ])
        .current_dir(&input.workspace)
        .output()?;
    if !inspect.status.success()
        || String::from_utf8_lossy(&inspect.stdout).trim() != manifest.image_id
    {
        return Err("loaded preparation image identity differs".into());
    }
    let state = input.workspace.join(".ayni/environment");
    fs::create_dir_all(&state)?;
    fs::copy(build, state.join("build.json"))?;
    Ok(())
}

fn stream_sha256(path: &Path) -> Result<String, io::Error> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn retain_result(
    input: &ValidationInput,
    lock_digest: String,
    changed_paths: Vec<String>,
    tests_edited: bool,
    exit: i32,
    preparation_reused: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = input.workspace.join(".ayni");
    let signals = root.join("last/signals.json");
    let execution = root.join("last/execution.json");
    let build = root.join("environment/build.json");
    let signals_digest = sha256(&signals)?;
    let execution_data: Value = serde_json::from_slice(&fs::read(&execution)?)?;
    if execution_data
        .get("artifact_digest")
        .and_then(Value::as_str)
        != Some(format!("sha256:{signals_digest}").as_str())
    {
        return Err("execution artifact does not bind the signals digest".into());
    }
    let mut artifacts = Vec::new();
    for (source, name) in [
        (signals, "signals.json"),
        (execution, "execution.json"),
        (build, "build.json"),
    ] {
        let destination = input.evidence.join(name);
        fs::copy(source, &destination)?;
        artifacts.push(Artifact {
            name,
            sha256: sha256(&destination)?,
        });
    }
    let manifest = Manifest {
        schema_version: 2,
        start_revision: input.start_revision.clone(),
        candidate_commit: input.candidate_commit.clone(),
        lock_sha256: lock_digest,
        change_constraints: input.change_constraints.clone(),
        outcome: if exit == 0 {
            "success"
        } else {
            "quality_failure"
        },
        check_exit_code: exit,
        policy_edited: false,
        tests_edited,
        scope_violations: Vec::new(),
        changed_paths,
        preparation_id: input.preparation_id.clone(),
        preparation_reused,
        artifacts,
    };
    write_manifest(&input.evidence, &manifest)?;
    if exit != 0 {
        return Err(format!("Ayni quality check failed with exit {exit}").into());
    }
    Ok(())
}

fn write_manifest(evidence: &Path, manifest: &Manifest) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = evidence.join("manifest.json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(manifest)?)?;
    fs::rename(temporary, evidence.join("manifest.json"))?;
    Ok(())
}

fn main() -> ExitCode {
    ayni_orchestrator_observability::init("validate-candidate");
    let process_span = ayni_orchestrator_observability::process_span("validate-candidate");
    let _process = process_span.enter();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(
                reason_code = "candidate_validation_failed",
                error = %ayni_orchestrator_observability::redact(&error.to_string()),
                "candidate validation failed"
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChangeConstraints, Command, MAX_CHANGED_PATHS, dedicated_test_path, fs, read_changed_paths,
        rust_test_fingerprint, scope_violations,
    };

    #[test]
    fn dedicated_test_paths_use_components_and_file_conventions() {
        for path in [
            "tests/retry.rs",
            "src/__tests__/api.rs",
            "src/retry_test.rs",
            "src/spec_parser.rs",
        ] {
            assert!(dedicated_test_path(path), "missed {path}");
        }
        for path in ["src/latest.rs", "src/contest.rs", "src/spectrum.rs"] {
            assert!(!dedicated_test_path(path), "misclassified {path}");
        }
    }

    #[test]
    fn diff_paths_include_add_modify_delete_and_both_rename_sides()
    -> Result<(), Box<dyn std::error::Error>> {
        fn git(directory: &std::path::Path, args: &[&str]) -> Result<String, std::io::Error> {
            let output = Command::new("git")
                .args(args)
                .current_dir(directory)
                .output()?;
            if !output.status.success() {
                return Err(std::io::Error::other(
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                ));
            }
            Ok(String::from_utf8(output.stdout)
                .map_err(std::io::Error::other)?
                .trim()
                .to_owned())
        }

        let repository = tempfile::tempdir()?;
        git(repository.path(), &["init", "--quiet"])?;
        git(repository.path(), &["config", "user.name", "scope-test"])?;
        git(
            repository.path(),
            &["config", "user.email", "scope-test@localhost"],
        )?;
        git(repository.path(), &["config", "commit.gpgsign", "false"])?;
        for (path, contents) in [
            ("modified.rs", "before"),
            ("deleted.rs", "deleted"),
            ("renamed-old.rs", "renamed"),
        ] {
            fs::write(repository.path().join(path), contents)?;
        }
        git(repository.path(), &["add", "."])?;
        git(repository.path(), &["commit", "--quiet", "-m", "baseline"])?;
        let baseline = git(repository.path(), &["rev-parse", "HEAD"])?;

        fs::write(repository.path().join("modified.rs"), "after")?;
        fs::remove_file(repository.path().join("deleted.rs"))?;
        fs::rename(
            repository.path().join("renamed-old.rs"),
            repository.path().join("renamed-new.rs"),
        )?;
        fs::write(repository.path().join("added.rs"), "added")?;
        git(repository.path(), &["add", "--all"])?;
        git(repository.path(), &["commit", "--quiet", "-m", "candidate"])?;
        let candidate = git(repository.path(), &["rev-parse", "HEAD"])?;
        let output = Command::new("git")
            .args([
                "diff",
                "--name-only",
                "-z",
                "--no-renames",
                &baseline,
                &candidate,
            ])
            .current_dir(repository.path())
            .output()?;
        assert!(output.status.success());
        fs::write(
            repository.path().join("git-diff-paths.stdout"),
            output.stdout,
        )?;
        assert_eq!(
            read_changed_paths(repository.path())?,
            vec![
                "added.rs",
                "deleted.rs",
                "modified.rs",
                "renamed-new.rs",
                "renamed-old.rs"
            ]
        );
        Ok(())
    }

    #[test]
    fn changed_path_accounting_is_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let evidence = tempfile::tempdir()?;
        let mut paths = Vec::new();
        for index in 0..=MAX_CHANGED_PATHS {
            paths.extend_from_slice(format!("src/{index}.rs\0").as_bytes());
        }
        let path = evidence.path().join("git-diff-paths.stdout");
        fs::write(&path, paths)?;
        assert!(read_changed_paths(evidence.path()).is_err());
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn inline_test_classification_ignores_implementation_only_edits() -> Result<(), syn::Error> {
        let baseline = r"
            fn retry_delay(attempt: u32) -> u32 { attempt * 2 }
            #[cfg(test)]
            mod tests {
                use super::*;
                #[test]
                fn verifies_backoff() { assert_eq!(retry_delay(2), 4); }
            }
        ";
        let implementation_only = baseline.replace("attempt * 2", "1 << attempt");
        assert_eq!(
            rust_test_fingerprint(baseline)?,
            rust_test_fingerprint(&implementation_only)?
        );

        let test_edit = baseline.replace("retry_delay(2), 4", "retry_delay(3), 6");
        assert_ne!(
            rust_test_fingerprint(baseline)?,
            rust_test_fingerprint(&test_edit)?
        );
        let comment_edit = baseline.replace(
            "fn verifies_backoff()",
            "// exact test text is frozen\n                fn verifies_backoff()",
        );
        assert_ne!(
            rust_test_fingerprint(baseline)?,
            rust_test_fingerprint(&comment_edit)?
        );

        let support = "#[cfg(test)]\nconst EXPECTED_RESULT: usize = 4;";
        let support_edit = support.replace('4', "5");
        assert_ne!(
            rust_test_fingerprint(support)?,
            rust_test_fingerprint(&support_edit)?
        );
        assert!(rust_test_fingerprint("#[latest]\nfn helper() {}")?.is_empty());
        Ok(())
    }

    #[test]
    fn scope_checks_protected_trees_and_test_authorization() {
        let constraints = ChangeConstraints {
            protected_paths: vec!["src/generated".into(), "scripts/oracle.sh".into()],
            allow_test_edits: false,
        };
        let changed = vec![
            "src/generated/new.rs".into(),
            "scripts/oracle.sh".into(),
            "tests/deleted.rs".into(),
            "src/renamed-old.rs".into(),
            "src/renamed-new.rs".into(),
        ];
        let violations = scope_violations(&constraints, &changed, &["tests/deleted.rs".into()]);
        assert_eq!(violations.len(), 3);
        assert!(
            violations
                .iter()
                .any(|value| value.contains("src/generated"))
        );
        assert!(
            violations
                .iter()
                .any(|value| value.contains("scripts/oracle.sh"))
        );
        assert!(violations.iter().any(|value| value.contains("test edits")));

        let allowed = ChangeConstraints {
            protected_paths: Vec::new(),
            allow_test_edits: true,
        };
        assert!(scope_violations(&allowed, &changed, &["tests/deleted.rs".into()]).is_empty());
    }
}
