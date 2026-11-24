//! Opt-in direct-session bring-up. Acquires the current seat and checks one
//! seat-assigned DRM device, but never performs modesetting or reads input.

use anyhow::{bail, Result};
use smithay::{
    backend::{session::{libseat::LibSeatSession, Session}, udev::UdevBackend},
    reexports::calloop::EventLoop,
};
use std::{io::IsTerminal, time::{Duration, Instant}};

fn tty_allowed(tty: bool, session: Option<&str>, graphical: bool) -> bool {
    tty && session == Some("tty") && !graphical
}

pub fn check() -> Result<()> {
    let graphical = std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("WAYLAND_SOCKET").is_some();
    if !tty_allowed(std::io::stdin().is_terminal(), std::env::var("XDG_SESSION_TYPE").ok().as_deref(), graphical) {
        bail!("seat check requires a local text login (XDG_SESSION_TYPE=tty), with no graphical display environment");
    }

    let (mut session, notifier) = LibSeatSession::new()
        .map_err(|error| anyhow::anyhow!("open libseat session: {error}"))?;
    let mut event_loop = EventLoop::<()>::try_new()
        .map_err(|error| anyhow::anyhow!("create session event loop: {error}"))?;
    event_loop.handle().insert_source(notifier, |event, _, _| {
        tracing::debug!(?event, "seat lifecycle event");
    }).map_err(|error| anyhow::anyhow!("register seat notifier: {error}"))?;

    let deadline = Instant::now() + Duration::from_secs(3);
    while !session.is_active() {
        if Instant::now() >= deadline { bail!("seat did not become active within three seconds"); }
        event_loop.dispatch(Duration::from_millis(100), &mut ())
            .map_err(|error| anyhow::anyhow!("dispatch seat events: {error}"))?;
    }

    let seat = session.seat();
    let udev = UdevBackend::new(&seat)
        .map_err(|error| anyhow::anyhow!("enumerate DRM devices for {seat}: {error}"))?;
    let mut devices: Vec<_> = udev.device_list().map(|(_, path)| path.to_path_buf()).collect();
    devices.sort();
    println!("Active seat: {seat}");
    for device in &devices { println!("  DRM device: {}", device.display()); }
    let Some(device) = devices.first() else { bail!("no DRM primary device assigned to this seat"); };

    // Device access is mediated by the seat provider; no direct opens or permission changes.
    let fd = session.open(device, rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC)
        .map_err(|error| anyhow::anyhow!("seat denied {}: {error}", device.display()))?;
    session.close(fd).map_err(|error| anyhow::anyhow!("release {}: {error}", device.display()))?;
    println!("Seat-mediated DRM open and close succeeded: {}", device.display());
    println!("No display mode was changed. GBM allocation, KMS presentation and input dispatch remain unimplemented.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_non_text_sessions() {
        assert!(tty_allowed(true, Some("tty"), false));
        assert!(!tty_allowed(false, Some("tty"), false));
        assert!(!tty_allowed(true, Some("wayland"), false));
        assert!(!tty_allowed(true, None, false));
        assert!(!tty_allowed(true, Some("tty"), true));
    }
}
