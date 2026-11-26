//! One-shot local authentication transport. This is not an unlock protocol.
use std::{io::{self, Read, Write}, os::{fd::{AsRawFd, FromRawFd}, unix::net::UnixStream}, time::Duration};
use crate::{Secret, MAX_SECRET};

pub(super) const VERSION: u8 = 1;
const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// Call at the beginning of the dedicated executable, before accepting secrets.
/// This is defense in depth, not isolation from a compromised desktop UID.
pub(super) fn harden() -> io::Result<()> {
    unsafe {
        if libc::prctl(libc::PR_SET_DUMPABLE, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong) != 0 {
            return Err(io::Error::last_os_error());
        }
        let limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn invalid() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "invalid authentication channel") }

fn check_channel(socket: &UnixStream) -> io::Result<()> {
    let fd = socket.as_raw_fd();
    let uid = unsafe { libc::getuid() };
    if uid == 0 || uid != unsafe { libc::geteuid() } {
        return Err(invalid());
    }
    // Require a connected AF_UNIX stream, not a terminal, file, listener,
    // datagram socket or network endpoint. Peer UID is a sanity check only.
    socket.peer_addr()?;
    unsafe {
        let mut family: libc::c_int = 0;
        let mut len = std::mem::size_of_val(&family) as libc::socklen_t;
        if libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_DOMAIN, (&mut family as *mut libc::c_int).cast(), &mut len) != 0 ||
            len as usize != std::mem::size_of_val(&family) || family != libc::AF_UNIX {
            return Err(invalid());
        }
        let mut kind: libc::c_int = 0;
        let mut len = std::mem::size_of_val(&kind) as libc::socklen_t;
        if libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_TYPE, (&mut kind as *mut libc::c_int).cast(), &mut len) != 0 ||
            len as usize != std::mem::size_of_val(&kind) || kind != libc::SOCK_STREAM {
            return Err(invalid());
        }
        let mut peer: libc::ucred = std::mem::zeroed();
        let mut len = std::mem::size_of_val(&peer) as libc::socklen_t;
        if libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut peer as *mut libc::ucred).cast(), &mut len) != 0 ||
            len as usize != std::mem::size_of_val(&peer) || peer.uid != uid {
            return Err(invalid());
        }
        // Do not leak the authentication channel to PAM's exec'd helpers.
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    socket.set_read_timeout(Some(IO_TIMEOUT))?;
    socket.set_write_timeout(Some(IO_TIMEOUT))?;
    Ok(())
}

/// Request: version byte, big-endian u16 length, then exactly that many
/// password bytes and a write-half shutdown. Response: version, 0 or 1.
/// One attempt per connection; malformed frames produce no positive reply.
/// A caller must impose an overall deadline, including the PAM transaction.
fn serve(mut channel: impl Read + Write, authenticate: impl FnOnce(&mut [u8]) -> bool) -> io::Result<()> {
    let mut header = [0u8; 3];
    channel.read_exact(&mut header)?;
    let len = u16::from_be_bytes([header[1], header[2]]) as usize;
    if header[0] != VERSION || len == 0 || len > MAX_SECRET {
        return Err(invalid());
    }
    let mut bytes = [0u8; MAX_SECRET];
    let secret = Secret(&mut bytes);
    channel.read_exact(&mut secret.0[..len])?;
    if secret.0[..len].contains(&0) {
        return Err(invalid());
    }
    // Reject trailing bytes, a second attempt, or a missing half-close.
    // Never copy an attacker-controlled extra byte into an unwiped buffer.
    let mut tail = [0u8; 1];
    let extra = Secret(&mut tail);
    if channel.read(extra.0)? != 0 {
        return Err(invalid());
    }
    let ok = authenticate(&mut secret.0[..len]);
    drop(secret);
    channel.write_all(&[VERSION, u8::from(ok)])?;
    channel.flush()
}

/// Entry point for the dedicated worker binary only. FD 0 must be an owned,
/// inherited private socket. No passwords through arguments, environment,
/// normal stdin pipes, stdout or a public listening endpoint.
///
/// # Safety
/// Call once, before starting any worker threads, with exclusive ownership
/// of descriptor 0 and no other code using stdin.
pub unsafe fn run_inherited_worker() -> io::Result<()> {
    harden()?;
    if libc::fcntl(0, libc::F_GETFD) < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = UnixStream::from_raw_fd(0);
    check_channel(&socket)?;
    serve(socket, crate::authenticate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Cursor, net::Shutdown};

    #[derive(Default)]
    struct Wire { input: Cursor<Vec<u8>>, output: Vec<u8> }
    impl Read for Wire {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.input.read(buf) }
    }
    impl Write for Wire {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> { self.output.write(buf) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    fn wire(input: &[u8]) -> Wire { Wire { input: Cursor::new(input.to_vec()), output: vec![] } }

    #[test]
    fn one_request_one_decision() {
        for decision in [false, true] {
            let mut w = wire(b"\x01\x00\x03abc");
            serve(&mut w, |secret| { assert_eq!(secret, b"abc"); decision }).unwrap();
            assert_eq!(w.output, [1, u8::from(decision)]);
        }
    }

    #[test]
    fn malformed_or_repeated_requests_never_authenticate() {
        for input in [
            &b""[..], b"\x02\x00\x01a", b"\x01\x00\x00", b"\x01\x10\x01",
            b"\x01\x00\x04ab", b"\x01\x00\x03a\x00b", b"\x01\x00\x01ab",
            b"\x01\x00\x01a\x01\x00\x01b",
        ] {
            let mut w = wire(input);
            assert!(serve(&mut w, |_| panic!("invalid request reached authentication")).is_err());
            assert!(w.output.is_empty());
        }
    }

    #[test]
    fn real_socket_half_close_delivers_response() {
        let (mut parent, child) = UnixStream::pair().unwrap();
        parent.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let thread = std::thread::spawn(move || serve(child, |secret| secret == b"test").unwrap());
        parent.write_all(b"\x01\x00\x04test").unwrap();
        parent.shutdown(Shutdown::Write).unwrap();
        let mut response = Vec::new();
        parent.read_to_end(&mut response).unwrap();
        thread.join().unwrap();
        assert_eq!(response, [1, 1]);
    }

    #[test]
    fn process_hardening_is_applied_in_disposable_child() {
        const MARKER: &str = "ATELIER_TEST_HARDEN_CHILD";
        if std::env::var_os(MARKER).is_some() {
            harden().unwrap();
            unsafe {
                assert_eq!(libc::prctl(libc::PR_GET_DUMPABLE, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong), 0);
                let mut limit: libc::rlimit = std::mem::zeroed();
                assert_eq!(libc::getrlimit(libc::RLIMIT_CORE, &mut limit), 0);
                assert_eq!(limit.rlim_cur, 0);
                assert_eq!(limit.rlim_max, 0);
            }
            return;
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "worker::tests::process_hardening_is_applied_in_disposable_child"])
            .env(MARKER, "1").status().unwrap();
        assert!(status.success());
    }
}
