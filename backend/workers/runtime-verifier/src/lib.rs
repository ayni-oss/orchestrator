//! Inspects candidate bytes using Ayni's official contracts, without invoking its executable.
//!
//! The controller must mount this trusted verifier separately, make the candidate
//! root and frozen source read-only, and bound the process's CPU, memory and time.
//! Certification/content verification grants neither runtime readiness nor task admission.
use ayni_core::{
    AyniPolicy, EnvironmentCertificateEnvelope, PrebuiltRuntimeIdentity, sha256_fingerprint,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};

const VERSION: &str = "0.16.2";
const SOURCE_FILE_BYTES: u64 = 512 * 1024;
const SOURCE_TOTAL_BYTES: u64 = 8 * 1024 * 1024;
const EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
const PROTECTED_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const METADATA: &str = "/etc/ayni/runtime.json";
const MANIFEST: &str = "/etc/ayni/protected-content.manifest";
const EXECUTABLE: &str = "/usr/local/bin/ayni";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    None,
    InvalidArguments,
    ExpectedCliDigestRequired,
    UnsupportedPlatform,
    InputUnavailable,
    InputPathUnsupported,
    InputBoundsExceeded,
    InputProtectionInvalid,
    PolicyInvalid,
    LockInvalid,
    TrustRequired,
    TrustMismatch,
    UnsupportedAyniVersion,
    CertificateInvalid,
    CliDigestMismatch,
    ContentInvalid,
}

#[derive(Debug)]
pub struct Options {
    pub source: PathBuf,
    pub expected_platform: String,
    pub expected_cli_sha256: String,
}

impl Options {
    /// # Errors
    /// Reject unknown/duplicate flags and missing trusted executable identity.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, Reason> {
        let platform = match std::env::consts::ARCH {
            "aarch64" => "linux/arm64",
            "x86_64" => "linux/amd64",
            _ => return Err(Reason::UnsupportedPlatform),
        };
        let mut result = Self {
            source: "/source".into(),
            expected_platform: platform.into(),
            expected_cli_sha256: String::new(),
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            if !seen.insert(flag.clone()) {
                return Err(Reason::InvalidArguments);
            }
            let value = args.next().ok_or(Reason::InvalidArguments)?;
            match flag.as_str() {
                "--source" if !value.is_empty() => result.source = value.into(),
                "--expected-platform" => result.expected_platform = value,
                "--expected-cli-sha256" => result.expected_cli_sha256 = value,
                _ => return Err(Reason::InvalidArguments),
            }
        }
        if result.expected_platform != platform {
            return Err(Reason::UnsupportedPlatform);
        }
        if result.expected_cli_sha256.is_empty() {
            return Err(Reason::ExpectedCliDigestRequired);
        }
        if !digest(&result.expected_cli_sha256) {
            return Err(Reason::InvalidArguments);
        }
        Ok(result)
    }
}

#[derive(Debug, Serialize)]
pub struct Evidence {
    pub state: &'static str,
    pub reason: Reason,
    pub policy_digest: String,
    pub lock_digest: String,
    pub lock_fingerprint: String,
    pub certificate_digest: String,
    pub manifest_digest: String,
    pub executable_digest: String,
    pub platform: String,
    pub ayni_version: String,
}

impl Evidence {
    #[must_use]
    pub const fn invalid(reason: Reason) -> Self {
        Self {
            state: "invalid",
            reason,
            policy_digest: String::new(),
            lock_digest: String::new(),
            lock_fingerprint: String::new(),
            certificate_digest: String::new(),
            manifest_digest: String::new(),
            executable_digest: String::new(),
            platform: String::new(),
            ayni_version: String::new(),
        }
    }
}

fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Inspect actual candidate root content; never invoke a candidate program.
#[must_use]
pub fn verify(options: &Options) -> Evidence {
    let mut evidence = Evidence::invalid(Reason::None);
    match inspect(options, &mut evidence) {
        Ok(()) => evidence.state = "content_verified",
        Err(reason) => evidence.reason = reason,
    }
    evidence
}

fn inspect(options: &Options, evidence: &mut Evidence) -> Result<(), Reason> {
    if !digest(&options.expected_cli_sha256) {
        return Err(Reason::ExpectedCliDigestRequired);
    }
    let source = options
        .source
        .canonicalize()
        .map_err(|_| Reason::InputUnavailable)?;
    bound_source(&source)?;
    let policy_bytes = bounded(&source.join(".ayni.toml"), SOURCE_FILE_BYTES, false)?;
    let lock_bytes = bounded(&source.join(".ayni.lock"), SOURCE_FILE_BYTES, false)?;
    evidence.policy_digest = sha256_fingerprint(&policy_bytes);
    evidence.lock_digest = sha256_fingerprint(&lock_bytes);
    let policy =
        AyniPolicy::parse(std::str::from_utf8(&policy_bytes).map_err(|_| Reason::PolicyInvalid)?)
            .map_err(|_| Reason::PolicyInvalid)?;
    let trust = policy
        .environment_certificate_trust_policy()
        .map_err(|_| Reason::PolicyInvalid)?;
    if trust.is_empty() {
        return Err(Reason::TrustRequired);
    }
    let lock = ayni_environment::read_lock(&source).map_err(|_| Reason::LockInvalid)?;
    // Upstream read_lock validates target evidence. Include the generic frozen
    // source evidence retained by repository-wide tools and Debian packages too.
    for input in lock
        .tools()
        .iter()
        .map(|v| &v.source)
        .chain(lock.debian_packages().iter().map(|v| &v.source))
    {
        if let Some(expected) = &input.digest {
            let path = source.join(&input.path);
            let canonical = path.canonicalize().map_err(|_| Reason::LockInvalid)?;
            if !canonical.starts_with(&source) {
                return Err(Reason::InputPathUnsupported);
            }
            if sha256_fingerprint(bounded(&path, SOURCE_FILE_BYTES, false)?) != *expected {
                return Err(Reason::LockInvalid);
            }
        }
    }
    if lock.repository().contract_path != ".ayni.toml" {
        return Err(Reason::LockInvalid);
    }
    if lock.ayni_version() != VERSION {
        return Err(Reason::UnsupportedAyniVersion);
    }
    if lock.certificate_trust_policy().fingerprint != trust.fingerprint() {
        return Err(Reason::TrustMismatch);
    }
    evidence.lock_fingerprint = lock.fingerprint().into();
    let bytes = bounded(
        Path::new(METADATA),
        ayni_environment::MAX_CERTIFICATE_BYTES,
        true,
    )?;
    let certificate = decode_certificate(&bytes)?;
    certificate
        .verify(&trust)
        .map_err(|_| Reason::CertificateInvalid)?;
    if certificate.certificate.platform != options.expected_platform {
        return Err(Reason::UnsupportedPlatform);
    }
    if certificate.certificate.ayni_version != VERSION {
        return Err(Reason::UnsupportedAyniVersion);
    }
    evidence.certificate_digest = sha256_fingerprint(&bytes);
    let manifest = bounded(
        Path::new(MANIFEST),
        ayni_environment::MAX_PROTECTED_MANIFEST_BYTES,
        true,
    )?;
    evidence.manifest_digest = sha256_fingerprint(&manifest);
    evidence.executable_digest =
        executable_digest(Path::new(EXECUTABLE), &options.expected_cli_sha256, true)?;
    bound_protected()?;
    let runtime = PrebuiltRuntimeIdentity {
        metadata_path: METADATA.into(),
        metadata_digest: evidence.certificate_digest.clone(),
        certificate,
    };
    ayni_environment::prebuilt::verify(&runtime, &lock, &trust)
        .map_err(|_| Reason::ContentInvalid)?;
    evidence.platform.clone_from(&options.expected_platform);
    evidence.ayni_version = VERSION.into();
    Ok(())
}

fn decode_certificate(bytes: &[u8]) -> Result<EnvironmentCertificateEnvelope, Reason> {
    let value: EnvironmentCertificateEnvelope =
        serde_json::from_slice(bytes).map_err(|_| Reason::CertificateInvalid)?;
    value
        .certificate
        .canonical_payload()
        .map_err(|_| Reason::CertificateInvalid)?;
    let mut canonical = serde_json::to_vec(&value).map_err(|_| Reason::CertificateInvalid)?;
    canonical.push(b'\n');
    if canonical != bytes {
        return Err(Reason::CertificateInvalid);
    }
    Ok(value)
}

fn open_bounded(path: &Path, maximum: u64, protected: bool) -> Result<fs::File, Reason> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| Reason::InputUnavailable)?;
        if metadata.file_type().is_symlink() {
            return Err(Reason::InputPathUnsupported);
        }
        #[cfg(unix)]
        if protected {
            use std::os::unix::fs::MetadataExt as _;
            if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
                return Err(Reason::InputProtectionInvalid);
            }
        }
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| Reason::InputUnavailable)?;
    let metadata = file.metadata().map_err(|_| Reason::InputUnavailable)?;
    if !metadata.is_file() {
        return Err(Reason::InputPathUnsupported);
    }
    if metadata.len() > maximum {
        return Err(Reason::InputBoundsExceeded);
    }
    Ok(file)
}

fn bounded(path: &Path, maximum: u64, protected: bool) -> Result<Vec<u8>, Reason> {
    let file = open_bounded(path, maximum, protected)?;
    let mut bytes = Vec::new();
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Reason::InputUnavailable)?;
    if bytes.len() as u64 > maximum {
        return Err(Reason::InputBoundsExceeded);
    }
    Ok(bytes)
}

fn executable_digest(path: &Path, expected: &str, protected: bool) -> Result<String, Reason> {
    let mut file = open_bounded(path, EXECUTABLE_BYTES, protected)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 16384];
    let mut total = 0_u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| Reason::InputUnavailable)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > EXECUTABLE_BYTES {
            return Err(Reason::InputBoundsExceeded);
        }
        hash.update(&buffer[..count]);
    }
    let actual = format!("sha256:{:x}", hash.finalize());
    if actual != expected {
        return Err(Reason::CliDigestMismatch);
    }
    Ok(actual)
}

fn bound_source(root: &Path) -> Result<(), Reason> {
    let mut directories = vec![(root.to_path_buf(), 0_u32)];
    let mut entries = 0_usize;
    let mut files = 0_usize;
    let mut bytes = 0_u64;
    while let Some((directory, depth)) = directories.pop() {
        if depth > 64 {
            return Err(Reason::InputBoundsExceeded);
        }
        for item in fs::read_dir(directory).map_err(|_| Reason::InputUnavailable)? {
            entries += 1;
            if entries > 512 {
                return Err(Reason::InputBoundsExceeded);
            }
            let item = item.map_err(|_| Reason::InputUnavailable)?;
            let metadata =
                fs::symlink_metadata(item.path()).map_err(|_| Reason::InputUnavailable)?;
            if metadata.file_type().is_symlink() {
                return Err(Reason::InputPathUnsupported);
            }
            if metadata.is_dir() {
                directories.push((item.path(), depth + 1));
            } else if metadata.is_file() {
                files += 1;
                bytes = bytes.saturating_add(metadata.len());
                if files > 256 || metadata.len() > SOURCE_FILE_BYTES || bytes > SOURCE_TOTAL_BYTES {
                    return Err(Reason::InputBoundsExceeded);
                }
            } else {
                return Err(Reason::InputPathUnsupported);
            }
        }
    }
    Ok(())
}

fn bound_protected() -> Result<(), Reason> {
    let mut pending: Vec<_> = ayni_environment::PROTECTED_FILE_ROOTS
        .iter()
        .chain(ayni_environment::PROTECTED_TREE_ROOTS)
        .map(|v| (PathBuf::from(v), 0_u32))
        .collect();
    let mut entries = 0_usize;
    let mut bytes = 0_u64;
    while let Some((path, depth)) = pending.pop() {
        entries += 1;
        if entries > ayni_environment::MAX_PROTECTED_MANIFEST_ENTRIES || depth > 128 {
            return Err(Reason::InputBoundsExceeded);
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(Reason::InputUnavailable),
        };
        if metadata.is_file() {
            bytes = bytes.saturating_add(metadata.len());
            if metadata.len() > EXECUTABLE_BYTES || bytes > PROTECTED_TOTAL_BYTES {
                return Err(Reason::InputBoundsExceeded);
            }
        } else if metadata.is_dir() {
            for item in fs::read_dir(path).map_err(|_| Reason::InputUnavailable)? {
                if pending.len() >= ayni_environment::MAX_PROTECTED_MANIFEST_ENTRIES {
                    return Err(Reason::InputBoundsExceeded);
                }
                pending.push((
                    item.map_err(|_| Reason::InputUnavailable)?.path(),
                    depth + 1,
                ));
            }
        } else if !metadata.file_type().is_symlink() {
            return Err(Reason::InputPathUnsupported);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
