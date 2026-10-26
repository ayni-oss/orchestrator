//! Process-level network isolation survives exec and does not depend on the CNI.
use std::io;

/// Deny every socket domain except local Unix IPC and disable async syscall bypass.
///
/// The caller must close inherited network descriptors before entering this jail.
/// Seccomp cannot revoke an already-open socket; the quality launcher passes only
/// standard streams to the child and no authority client or provider descriptor.
#[cfg(target_os = "linux")]
pub fn isolate() -> io::Result<()> {
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule,
    };
    let domain = SeccompRule::new(vec![
        SeccompCondition::new(
            0,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::Ne,
            u64::try_from(libc::AF_UNIX).map_err(io::Error::other)?,
        )
        .map_err(io::Error::other)?,
    ])
    .map_err(io::Error::other)?;
    let denied = [
        (libc::SYS_socket, vec![domain.clone()]),
        (libc::SYS_socketpair, vec![domain]),
        // io_uring can issue socket/connect operations outside normal syscall BPF.
        (libc::SYS_io_uring_setup, vec![]),
        (libc::SYS_io_uring_enter, vec![]),
        (libc::SYS_io_uring_register, vec![]),
        // Prevent obtaining an authority descriptor from another process.
        (libc::SYS_pidfd_getfd, vec![]),
        (libc::SYS_ptrace, vec![]),
        (libc::SYS_process_vm_readv, vec![]),
        (libc::SYS_process_vm_writev, vec![]),
    ];
    let filter: BpfProgram = SeccompFilter::new(
        denied.into_iter().collect(),
        SeccompAction::Allow,
        SeccompAction::Errno(u32::try_from(libc::EPERM).map_err(io::Error::other)?),
        std::env::consts::ARCH
            .try_into()
            .map_err(io::Error::other)?,
    )
    .map_err(io::Error::other)?
    .try_into()
    .map_err(io::Error::other)?;
    seccompiler::apply_filter_all_threads(&filter).map_err(io::Error::other)
}

#[cfg(not(target_os = "linux"))]
pub fn isolate() -> io::Result<()> {
    Err(io::Error::other("quality isolation requires Linux seccomp"))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{
        net::{TcpListener, UdpSocket},
        os::unix::net::UnixStream,
        process::Command,
    };

    #[test]
    fn network_jail_denies_inet_and_survives_exec_with_unix_ipc() -> io::Result<()> {
        const CHILD: &str = "AYNI_NETWORK_ISOLATION_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let sockets = std::fs::read_dir("/proc/self/fd")?
                .filter_map(Result::ok)
                .filter_map(|entry| std::fs::read_link(entry.path()).ok())
                .filter(|target| target.to_string_lossy().starts_with("socket:"))
                .count();
            assert_eq!(sockets, 0, "the jail must inherit no network descriptor");
            isolate()?;
            for address in ["127.0.0.1:0", "[::1]:0"] {
                let error = TcpListener::bind(address)
                    .err()
                    .ok_or_else(|| io::Error::other("Internet socket allowed"))?;
                assert_eq!(error.raw_os_error(), Some(libc::EPERM));
                let error = UdpSocket::bind(address)
                    .err()
                    .ok_or_else(|| io::Error::other("datagram socket allowed"))?;
                assert_eq!(error.raw_os_error(), Some(libc::EPERM));
            }
            let _local = UnixStream::pair()?;
            // Exec inherits the filter. Creating an Internet descriptor is
            // rechecked after exec by a second isolated test process.
            if std::env::var(CHILD).as_deref() == Ok("first") {
                let status = Command::new(std::env::current_exe()?)
                    .args([
                        "--exact",
                        "network::tests::network_jail_denies_inet_and_survives_exec_with_unix_ipc",
                    ])
                    .env(CHILD, "after-exec")
                    .status()?;
                assert!(status.success());
            }
            return Ok(());
        }
        let status = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "network::tests::network_jail_denies_inet_and_survives_exec_with_unix_ipc",
            ])
            .env(CHILD, "first")
            .status()?;
        assert!(status.success());
        Ok(())
    }
}
