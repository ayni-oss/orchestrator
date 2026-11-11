use crate::command;
use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::{collections::BTreeMap, fs, path::Path, process::Command, time::Duration};
use walkdir::WalkDir;

const POSTGRES: &str =
    "postgres@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3";

pub fn tree_bytes(root: &str) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(root) {
        let entry = entry?;
        if entry.file_type().is_file() {
            files.insert(
                entry
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .into_owned(),
                fs::read(entry.path())?,
            );
        }
    }
    Ok(files)
}

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("protocol") => command::run("buf", &["generate"]),
        Some("fixtures") => crate::fixtures::generate(),
        Some("schema") => schema(args.get(1).is_some_and(|arg| arg == "--write")),
        Some("contract-bundle") => bundle(
            args.get(1)
                .context("contract-bundle requires an output directory")?,
        ),
        _ => bail!("unknown generator; use protocol, schema [--write], contract-bundle OUTPUT"),
    }
}

struct Container(String);
impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "--force", &self.0])
            .output();
    }
}

fn schema(write: bool) -> Result<()> {
    let path =
        Path::new("backend/servers/control-plane/crates/workflow-postgres/src/migrations.rs");
    let source = fs::read_to_string(path)?;
    let query =
        Regex::new(r#"(?s)const fn schema_manifest_query\(\) -> &'static str \{\s*r"(.*?)"\s*\}"#)?;
    let query = query
        .captures(&source)
        .context("canonical schema manifest query not found")?;
    let name = format!("ayni-schema-{}", uuid::Uuid::new_v4().simple());
    command::run(
        "docker",
        &[
            "run",
            "--detach",
            "--rm",
            "--name",
            &name,
            "--env",
            "POSTGRES_HOST_AUTH_METHOD=trust",
            POSTGRES,
        ],
    )?;
    let _guard = Container(name.clone());
    let mut ready = false;
    for _ in 0..60 {
        if Command::new("docker")
            .args([
                "exec",
                &name,
                "pg_isready",
                "--host",
                "127.0.0.1",
                "-U",
                "postgres",
            ])
            .output()?
            .status
            .success()
        {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    ensure!(ready, "disposable PostgreSQL failed readiness");
    let mut sql = String::from(
        "SET search_path TO public, pg_catalog, pg_temp;\nCREATE TABLE public.workflow_schema_migrations (version bigint PRIMARY KEY CHECK(version > 0), description text NOT NULL, checksum text NOT NULL, applied_at timestamptz NOT NULL DEFAULT clock_timestamp());\n",
    );
    let migrations =
        tree_bytes("backend/servers/control-plane/crates/workflow-postgres/migrations")?;
    for (name, bytes) in migrations {
        if std::path::Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sql"))
        {
            sql.push_str(std::str::from_utf8(&bytes)?);
            sql.push('\n');
        }
    }
    let args = [
        "exec",
        "-i",
        &name,
        "psql",
        "-XAt",
        "-v",
        "ON_ERROR_STOP=1",
        "-U",
        "postgres",
    ];
    command::input("docker", &args, sql.as_bytes())?;
    let mut manifest = command::input("docker", &args, query[1].as_bytes())?;
    if manifest.last() == Some(&b'\n') {
        manifest.pop();
    }
    let fingerprint = format!("sha256:{:x}", Sha256::digest(&manifest));
    let pattern = Regex::new(r#"(const SCHEMA_MANIFEST_CHECKSUM: &str =\s*")[^"]+(";)"#)?;
    let current = pattern
        .captures(&source)
        .context("schema manifest constant not found")?;
    if write {
        fs::write(
            path,
            pattern
                .replace(&source, |captures: &regex::Captures<'_>| {
                    format!("{}{fingerprint}{}", &captures[1], &captures[2])
                })
                .as_bytes(),
        )?;
    } else {
        ensure!(
            current[0].contains(&fingerprint),
            "schema manifest differs; run cargo xtask generate schema --write"
        );
    }
    println!("{fingerprint}");
    Ok(())
}

fn bundle(output: &str) -> Result<()> {
    ensure!(
        command::output(
            "git",
            &[
                "status",
                "--porcelain",
                "--untracked-files=all",
                "--",
                "proto"
            ]
        )?
        .trim()
        .is_empty(),
        "commit protocol sources before bundling"
    );
    let revision = command::output("git", &["rev-parse", "HEAD"])?;
    let revision = revision.trim();
    let name = format!("ayni-orchestrator-v1-{}", &revision[..12]);
    let output = Path::new(output);
    fs::create_dir_all(output)?;
    let archive_path = output.join(format!("{name}.tar.gz"));
    let checksum_path = output.join(format!("{name}.tar.gz.sha256"));
    ensure!(
        !archive_path.exists() && !checksum_path.exists(),
        "refusing to overwrite contract bundle"
    );
    let directory = tempfile::tempdir()?;
    let root = directory.path().join(&name);
    fs::create_dir(&root)?;
    let archive = Command::new("git")
        .args([
            "archive",
            "HEAD",
            "proto/ayni",
            "proto/BASELINE",
            "proto/README.md",
            "proto/RELEASES.md",
            "proto/buf.yaml",
        ])
        .output()?;
    ensure!(archive.status.success(), "git archive failed");
    tar::Archive::new(archive.stdout.as_slice()).unpack(&root)?;
    command::run(
        "buf",
        &[
            "build".as_ref(),
            root.join("proto").as_os_str(),
            "--as-file-descriptor-set".as_ref(),
            "--output".as_ref(),
            root.join("ayni.orchestrator.v1.pb").as_os_str(),
        ],
    )?;
    fs::write(
        root.join("VERSION"),
        format!("package=ayni.orchestrator.v1\nrevision={revision}\ncompatibility=additive-only\n"),
    )?;
    let mut sums = String::new();
    for (path, bytes) in tree_bytes(root.to_str().context("bundle path")?)? {
        writeln!(sums, "{:x}  ./{path}", Sha256::digest(bytes))?;
    }
    fs::write(root.join("SHA256SUMS"), sums)?;
    let archive = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&archive_path)?;
    let gzip = flate2::write::GzEncoder::new(archive, flate2::Compression::default());
    let mut archive = tar::Builder::new(gzip);
    archive.append_dir_all(&name, &root)?;
    archive.into_inner()?.finish()?;
    let checksum = format!(
        "{:x}  {name}.tar.gz\n",
        Sha256::digest(fs::read(&archive_path)?)
    );
    fs::write(checksum_path, checksum)?;
    println!("{}", archive_path.display());
    Ok(())
}
