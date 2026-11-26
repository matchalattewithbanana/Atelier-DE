//! Trusted-parent half of the one-shot recovery authentication channel.
use crate::{Secret, MAX_SECRET, worker::{VERSION, harden}};
use std::{
    fs, io::{self, Read, Write}, net::Shutdown,
    os::{fd::OwnedFd, unix::{fs::MetadataExt, net::UnixStream, process::CommandExt}},
    path::Path, process::{Child, Command, Stdio}, thread,
    time::{Duration, Instant},
};

const WORKER: &str = "/usr/lib/atelier/atelier-recovery-auth-worker";
const DEADLINE: Duration = Duration::from_secs(30);

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid recovery authentication exchange")
}
fn denied() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "untrusted recovery authentication environment")
}
fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "recovery authentication deadline expired")
}

/// Call at locker startup, before collecting credentials or opening the
/// private Wayland connection. This is process-wide defense in depth.
pub fn prepare_authentication_process() -> io::Result<()> {
    let uid = unsafe { libc::getuid() };
    if uid == 0 || uid != unsafe { libc::geteuid() } {
        return Err(denied());
    }
    harden()
}

fn parent_protected() -> bool {
    unsafe {
        let uid = libc::getuid();
        let mut limit: libc::rlimit = std::mem::zeroed();
        uid != 0 && uid == libc::geteuid() &&
            libc::prctl(libc::PR_GET_DUMPABLE, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong) == 0 &&
            libc::getrlimit(libc::RLIMIT_CORE, &mut limit) == 0 &&
            limit.rlim_cur == 0 && limit.rlim_max == 0
    }
}

fn trusted_worker(path: &Path) -> bool {
    let Ok(file) = fs::symlink_metadata(path) else { return false; };
    file.is_file() && file.uid() == 0 &&
        file.mode() & 0o6022 == 0 && file.mode() & 0o111 != 0 &&
        path.parent().is_some_and(|p| p.ancestors().all(|p|
            fs::symlink_metadata(p).is_ok_and(|m|
                m.is_dir() && m.uid() == 0 && m.mode() & 0o022 == 0)))
}

/// Run one attempt in the fixed administrator-installed worker. The supplied
/// buffer is erased on every return. Call on a dedicated background thread,
/// not a compositor or UI event loop. Errors never authorize unlocking.
/// True requires a complete positive response AND successful child exit.
pub fn authenticate_in_worker(password: &mut [u8]) -> io::Result<bool> {
    let secret = Secret(password);
    if !parent_protected() || !trusted_worker(Path::new(WORKER)) {
        return Err(denied());
    }
    let mut command = Command::new(WORKER);
    command.env_clear().env("PATH", "/usr/bin:/bin").current_dir("/");
    exchange(command, secret.0, DEADLINE)
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        // try_wait caches the exit status. Never signal a successfully reaped
        // child; otherwise terminate the direct child and reap it.
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    let now = Instant::now();
    if now >= deadline { Err(timed_out()) } else { Ok(deadline - now) }
}

fn write_all(socket: &mut UnixStream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        socket.set_write_timeout(Some(remaining(deadline)?))?;
        match socket.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => return Err(timed_out()),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn read_response(socket: &mut UnixStream, deadline: Instant) -> io::Result<bool> {
    let mut bytes = [0u8; 3];
    let mut used = 0;
    loop {
        if used == bytes.len() { return Err(invalid()); }
        socket.set_read_timeout(Some(remaining(deadline)?))?;
        match socket.read(&mut bytes[used..]) {
            Ok(0) => break,
            Ok(n) => used += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => return Err(timed_out()),
            Err(e) => return Err(e),
        }
    }
    if used != 2 || bytes[0] != VERSION || bytes[1] > 1 { return Err(invalid()); }
    Ok(bytes[1] == 1)
}

fn exchange(mut command: Command, password: &mut [u8], timeout: Duration) -> io::Result<bool> {
    let secret = Secret(password);
    if secret.0.is_empty() || secret.0.len() > MAX_SECRET || secret.0.contains(&0) {
        return Err(invalid());
    }
    let deadline = Instant::now() + timeout;
    let (mut socket, endpoint) = UnixStream::pair()?;
    let fd: OwnedFd = endpoint.into();
    command.stdin(Stdio::from(fd)).stdout(Stdio::null()).stderr(Stdio::null());
    // Mark all non-stdio descriptors close-on-exec, including accidental
    // inherited capabilities. CLOEXEC preserves Rust's exec-error pipe until
    // exec completes. Fail closed on kernels without close_range support.
    unsafe {
        command.pre_exec(|| {
            const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
            if libc::syscall(libc::SYS_close_range, 3 as libc::c_uint, libc::c_uint::MAX, CLOSE_RANGE_CLOEXEC) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = ChildGuard(command.spawn()?);
    // Command retains its configured Stdio handle for reuse after spawn.
    // Close the parent's duplicate of the child endpoint before waiting for EOF.
    drop(command);
    let len = (secret.0.len() as u16).to_be_bytes();
    write_all(&mut socket, &[VERSION, len[0], len[1]], deadline)?;
    write_all(&mut socket, secret.0, deadline)?;
    drop(secret);
    socket.shutdown(Shutdown::Write)?;
    let decision = read_response(&mut socket, deadline)?;
    loop {
        let left = remaining(deadline)?;
        if let Some(status) = child.0.try_wait()? {
            // Never grant a positive response from a failed or overdue child.
            remaining(deadline)?;
            return if status.success() { Ok(decision) } else { Err(invalid()) };
        }
        thread::sleep(left.min(Duration::from_millis(5)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::FromRawFd;

    const MODE: &str = "ATELIER_AUTH_LAUNCHER_FIXTURE";

    #[test]
    fn fixture_child() {
        let Ok(mode) = std::env::var(MODE) else { return; };
        // The test executable is a synthetic child, never a PAM worker.
        let mut socket = unsafe { UnixStream::from_raw_fd(0) };
        let mut bytes = Vec::new();
        socket.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"\x01\x00\x04test");
        match mode.as_str() {
            "allow" => socket.write_all(&[1, 1]).unwrap(),
            "deny" => socket.write_all(&[1, 0]).unwrap(),
            "trailing" => socket.write_all(&[1, 1, 0]).unwrap(),
            "version" => socket.write_all(&[2, 1]).unwrap(),
            "value" => socket.write_all(&[1, 2]).unwrap(),
            "short" => socket.write_all(&[1]).unwrap(),
            "failure" => { socket.write_all(&[1, 1]).unwrap(); std::process::exit(7); }
            "stall" => thread::sleep(Duration::from_secs(10)),
            "stall_after_reply" => {
                socket.write_all(&[1, 1]).unwrap();
                drop(socket);
                thread::sleep(Duration::from_secs(10));
            }
            _ => panic!("unknown test mode"),
        }
    }

    fn run(mode: &str, timeout: Duration) -> io::Result<bool> {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "launcher::tests::fixture_child"]).env(MODE, mode);
        let mut password = *b"test";
        let result = exchange(command, &mut password, timeout);
        assert_eq!(password, [0; 4]);
        result
    }

    #[test]
    fn exact_child_response_and_success_required() {
        let timeout = Duration::from_secs(5);
        assert!(run("allow", timeout).unwrap());
        assert!(!run("deny", timeout).unwrap());
        for mode in ["trailing", "version", "value", "short", "failure"] {
            assert_eq!(run(mode, timeout).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn overall_deadline_covers_response_and_exit() {
        for mode in ["stall", "stall_after_reply"] {
            assert_eq!(run(mode, Duration::from_millis(300)).unwrap_err().kind(), io::ErrorKind::TimedOut);
        }
    }

    #[test]
    fn invalid_input_and_spawn_failure_erase_password() {
        let mut invalid_password = [1, 0, 2];
        let command = Command::new("/nonexistent/atelier-test-worker");
        assert!(exchange(command, &mut invalid_password, DEADLINE).is_err());
        assert_eq!(invalid_password, [0; 3]);
        let mut password = *b"test";
        assert!(exchange(Command::new("/nonexistent/atelier-test-worker"), &mut password, DEADLINE).is_err());
        assert_eq!(password, [0; 4]);
    }
}
