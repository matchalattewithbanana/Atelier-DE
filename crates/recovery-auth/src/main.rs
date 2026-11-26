//! Internal worker. No Wayland connection, unlock authority or CLI mode.
#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    // No logging of authentication or protocol data. The exit status is
    // transport health only; the peer must consume the complete response.
    if std::env::args_os().len() != 1 {
        return std::process::ExitCode::FAILURE;
    }
    // SAFETY: this executable has sole ownership of stdin and starts no
    // threads before transferring the inherited descriptor.
    match unsafe { atelier_recovery_auth::run_inherited_worker() } {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::FAILURE,
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    std::process::ExitCode::FAILURE
}
