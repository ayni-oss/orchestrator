use crate::command;
use anyhow::{Result, ensure};
use regex::Regex;
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::Duration,
};

pub fn run() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let mut client = TcpStream::connect_timeout(&listener.local_addr()?, Duration::from_secs(5))?;
    let (mut server, _) = listener.accept()?;
    server.set_read_timeout(Some(Duration::from_secs(5)))?;
    client.write_all(b"loopback")?;
    let mut received = [0; 8];
    server.read_exact(&mut received)?;
    ensure!(&received == b"loopback", "loopback capability failed");
    let output = command::output(
        "cargo",
        &[
            "test",
            "--locked",
            "--workspace",
            "--all-targets",
            "requires_loopback",
            "--",
            "--list",
            "--ignored",
            "--format",
            "terse",
        ],
    )?;
    let declaration = Regex::new(
        r#"(?s)#\[ignore\s*=\s*"requires loopback networking; run (?:scripts/ci/check-workflow-loopback.sh|cargo xtask test loopback)"\s*\](?:[^f]|f[^n]|fn[^\s])*?\bfn\s+(\w+)"#,
    )?;
    let mut declared = BTreeMap::new();
    for entry in walkdir::WalkDir::new("backend") {
        let entry = entry?;
        if entry.file_type().is_file() && entry.path().extension().is_some_and(|ext| ext == "rs") {
            for capture in declaration.captures_iter(&fs::read_to_string(entry.path())?) {
                ensure!(
                    capture[1].ends_with("_requires_loopback"),
                    "loopback test lacks required suffix"
                );
                *declared.entry(capture[1].to_owned()).or_insert(0) += 1;
            }
        }
    }
    let listed = Regex::new(r"(?m)^([\w:]+): test$")?;
    let mut actual = BTreeMap::new();
    for capture in listed.captures_iter(&output) {
        *actual
            .entry(
                capture[1]
                    .rsplit("::")
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
            )
            .or_insert(0) += 1;
    }
    ensure!(
        !declared.is_empty() && declared == actual,
        "loopback inventory mismatch: declared={declared:?} actual={actual:?}"
    );
    command::run(
        "cargo",
        &[
            "test",
            "--locked",
            "--workspace",
            "--all-targets",
            "requires_loopback",
            "--",
            "--ignored",
            "--nocapture",
        ],
    )
}
