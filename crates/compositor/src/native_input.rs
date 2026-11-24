//! Seat-mediated libinput source. It does not log keystrokes or pointer data.
use anyhow::{bail, Result};
use smithay::{
    backend::{
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        session::{libseat::LibSeatSession, Session},
    },
    reexports::{calloop::EventLoop, input::Libinput},
};
use std::{cell::Cell, rc::Rc, time::{Duration, Instant}};

pub fn backend(session: &LibSeatSession) -> Result<LibinputInputBackend> {
    if !session.is_active() { bail!("cannot attach libinput to an inactive seat"); }
    let mut context = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    context.udev_assign_seat(&session.seat())
        .map_err(|_| anyhow::anyhow!("libinput could not assign seat {}", session.seat()))?;
    Ok(LibinputInputBackend::new(context))
}

/// Enumerate devices for a bounded interval. Raw input contents are discarded.
pub fn check(session: &LibSeatSession, event_loop: &mut EventLoop<'_, ()>) -> Result<()> {
    let devices = Rc::new(Cell::new(0usize));
    let count = Rc::clone(&devices);
    let token = event_loop.handle().insert_source(backend(session)?, move |event, _, _| {
        match event {
            InputEvent::DeviceAdded { .. } => count.set(count.get().saturating_add(1)),
            InputEvent::DeviceRemoved { .. } => count.set(count.get().saturating_sub(1)),
            _ => {}
        }
    }).map_err(|error| anyhow::anyhow!("register libinput source: {error}"))?;
    let result = (|| -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(750);
        while Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(50), &mut ())
                .map_err(|error| anyhow::anyhow!("dispatch input source: {error}"))?;
            if !session.is_active() { bail!("seat deactivated during input check"); }
        }
        println!("Libinput devices on {}: {}", session.seat(), devices.get());
        if devices.get() == 0 { bail!("libinput found no accessible devices"); }
        Ok(())
    })();
    event_loop.handle().remove(token);
    result
}
