//! Opt-in direct-session bring-up. Acquires the current seat and checks one
//! seat-assigned DRM device, but never performs modesetting or reads input.

use anyhow::{bail, Result};
use smithay::{
    backend::{session::{libseat::LibSeatSession, Event as SessionEvent, Session}, udev::UdevBackend},
    reexports::calloop::EventLoop,
};
use std::{cell::Cell,io::IsTerminal,rc::Rc,time::{Duration, Instant}};

/// Once a running native session loses the seat, its original DRM/GBM
/// ownership assumptions cannot be re-established by an Enable event alone.
/// This latch deliberately survives a Pause + Activate in one dispatch.
#[derive(Default)]
pub(crate) struct SeatContinuity{pauses:Cell<u64>}
impl SeatContinuity{
    fn observe(&self,event:&SessionEvent){
        if matches!(event,SessionEvent::PauseSession){self.pauses.set(self.pauses.get().saturating_add(1));}
    }
    pub(crate) fn usable(&self,active:bool)->bool{active&&self.pauses.get()==0}
    /// Recovery may inspect a new active epoch but must never clear the latch.
    #[cfg(feature = "native-session")]
    pub(crate) fn recovery_epoch(&self,active:bool)->Option<u64>{active.then(||self.pauses.get())}
    #[cfg(feature = "native-session")]
    pub(crate) fn ensure(&self,active:bool)->Result<()>{
        if !self.usable(active){bail!("native seat ownership interrupted; recovery requires a verified new route");}
        Ok(())
    }
}

fn tty_allowed(tty: bool, session: Option<&str>, graphical: bool) -> bool {
    tty && session == Some("tty") && !graphical
}

pub enum Check {
    Seat,
    #[cfg(feature = "native-input")]
    Input,
    #[cfg(feature = "native-session")]
    Native { socket: String, config: atelier_config::Config },
    #[cfg(feature = "native-gbm")]
    Gbm,
    #[cfg(feature = "native-gbm")]
    Render,
    #[cfg(feature = "native-kms")]
    Kms,
    #[cfg(feature = "native-kms")]
    Atomic,
    #[cfg(feature = "native-kms")]
    Present,
}

pub fn check(kind: Check) -> Result<()> {
    let graphical = std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("WAYLAND_SOCKET").is_some();
    if !tty_allowed(std::io::stdin().is_terminal(), std::env::var("XDG_SESSION_TYPE").ok().as_deref(), graphical) {
        bail!("seat check requires a local text login (XDG_SESSION_TYPE=tty), with no graphical display environment");
    }

    let (mut session, notifier) = LibSeatSession::new()
        .map_err(|error| anyhow::anyhow!("open libseat session: {error}"))?;
    let mut event_loop = EventLoop::<()>::try_new()
        .map_err(|error| anyhow::anyhow!("create session event loop: {error}"))?;
    let continuity=Rc::new(SeatContinuity::default());
    let observed=Rc::clone(&continuity);
    event_loop.handle().insert_source(notifier, move |event, _, _| {
        observed.observe(&event);
        tracing::debug!(?event, "seat lifecycle event");
    }).map_err(|error| anyhow::anyhow!("register seat notifier: {error}"))?;

    let deadline = Instant::now() + Duration::from_secs(3);
    while !session.is_active() {
        if Instant::now() >= deadline { bail!("seat did not become active within three seconds"); }
        event_loop.dispatch(Duration::from_millis(100), &mut ())
            .map_err(|error| anyhow::anyhow!("dispatch seat events: {error}"))?;
    }

    #[cfg(feature = "native-input")]
    if matches!(&kind, Check::Input) { return crate::native_input::check(&session, &mut event_loop); }

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
    let diagnostic_result = match kind {
        Check::Seat => Ok(()),
        #[cfg(feature = "native-input")]
        Check::Input => unreachable!("input handled before DRM acquisition"),
        #[cfg(feature = "native-session")]
        Check::Native { socket, config } => crate::kms_check::run_native(&fd, &session, &mut event_loop, &socket, &config, Rc::clone(&continuity)),
        #[cfg(feature = "native-gbm")]
        Check::Gbm => crate::gbm_check::check(&fd, false),
        #[cfg(feature = "native-gbm")]
        Check::Render => crate::gbm_check::check(&fd, true),
        #[cfg(feature = "native-kms")]
        Check::Kms => crate::kms_check::check(&fd),
        #[cfg(feature = "native-kms")]
        Check::Atomic => crate::kms_check::check_atomic(&fd),
        #[cfg(feature = "native-kms")]
        Check::Present => {
            use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
            let running = Arc::new(AtomicBool::new(true));
            let signal = Arc::clone(&running);
            let setup = ctrlc::set_handler(move || signal.store(false, Ordering::SeqCst));
            match setup {
                Err(error) => Err(anyhow::anyhow!("install preview signal handler: {error}")),
                Ok(()) => crate::kms_check::present_test(&fd, || {
                    event_loop.dispatch(Duration::from_millis(20), &mut ())
                        .map_err(|error| anyhow::anyhow!("dispatch seat events: {error}"))?;
                    Ok(session.is_active() && running.load(Ordering::SeqCst))
                }),
            }
        },
    };
    // Always release the seat descriptor, including after allocation/export failure.
    let release_result = session.close(fd).map_err(|error| anyhow::anyhow!("release {}: {error}", device.display()));
    match (diagnostic_result, release_result) {
        (Err(diagnostic), Err(release)) => bail!("{diagnostic}; additionally failed to {release}"),
        (Err(error), _) | (_, Err(error)) => return Err(error),
        (Ok(()), Ok(())) => {}
    }
    println!("Seat-mediated DRM open and close succeeded: {}", device.display());
    println!("DRM device released.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_pause_is_not_erased_by_reactivation(){
        let continuity=SeatContinuity::default();
        assert!(continuity.usable(true));assert!(!continuity.usable(false));
        continuity.observe(&SessionEvent::ActivateSession);
        assert!(continuity.usable(true));
        continuity.observe(&SessionEvent::PauseSession);
        assert!(!continuity.usable(false));
        continuity.observe(&SessionEvent::ActivateSession);
        assert!(!continuity.usable(true));
    }
    #[test]
    fn rejects_non_text_sessions() {
        assert!(tty_allowed(true, Some("tty"), false));
        assert!(!tty_allowed(false, Some("tty"), false));
        assert!(!tty_allowed(true, Some("wayland"), false));
        assert!(!tty_allowed(true, None, false));
        assert!(!tty_allowed(true, Some("tty"), true));
    }
}
