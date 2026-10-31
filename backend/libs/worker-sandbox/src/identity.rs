//! Verify the exec boundary already dropped all identity and capability authority.
use std::io;

/// Verify real/effective UID/GID, empty supplementary groups and capabilities,
/// then set no-new-privileges and the existing bounded process/file limits.
///
/// The launcher must first select UID/GID with `CommandExt`; this does not drop
/// an in-process identity and cannot revoke inherited descriptors.
/// # Errors
/// Reject incomplete identity isolation or unavailable Linux enforcement.
#[cfg(target_os = "linux")]
pub fn isolate_child(uid: u32, gid: u32) -> io::Result<()> {
    use rustix::{
        process,
        thread::{self, CapabilitySet},
    };
    if uid == 0
        || gid == 0
        || process::getuid().as_raw() != uid
        || process::geteuid().as_raw() != uid
        || process::getgid().as_raw() != gid
        || process::getegid().as_raw() != gid
        || !process::getgroups().map_err(io::Error::other)?.is_empty()
    {
        return Err(io::Error::other("worker child identity was not isolated"));
    }
    let capabilities = thread::capabilities(None).map_err(io::Error::other)?;
    if capabilities.effective != CapabilitySet::empty()
        || capabilities.permitted != CapabilitySet::empty()
        || capabilities.inheritable != CapabilitySet::empty()
    {
        return Err(io::Error::other("worker child retained capabilities"));
    }
    thread::set_no_new_privs(true).map_err(io::Error::other)?;
    process_limits()
}

#[cfg(target_os = "linux")]
fn process_limits() -> io::Result<()> {
    use rustix::process::{Resource, Rlimit, setrlimit};
    for (resource, limit) in [(Resource::Nproc, 2048), (Resource::Nofile, 8192)] {
        setrlimit(
            resource,
            Rlimit {
                current: Some(limit),
                maximum: Some(limit),
            },
        )
        .map_err(io::Error::other)?;
    }
    Ok(())
}

/// # Errors
/// Non-Linux hosts cannot supply the required privilege boundary.
#[cfg(not(target_os = "linux"))]
pub fn isolate_child(_: u32, _: u32) -> io::Result<()> {
    Err(io::Error::other(
        "worker privilege isolation requires Linux",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn root_identity_is_never_an_isolated_worker() {
        assert!(isolate_child(0, 0).is_err());
    }
}
