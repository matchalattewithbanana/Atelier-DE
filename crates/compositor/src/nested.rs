//! Winit/GLES backend. Keep backend-specific presentation/input separate from desktop state.

use crate::state::{Atelier, ClientState};
use anyhow::{Context, Result};
use smithay::{
    backend::{
        input::{AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputEvent, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent},
        renderer::{gles::GlesRenderer, utils::draw_render_elements, Color32F, Frame, Renderer},
        winit::{self, WinitEvent, WinitInput},
    },
    input::{keyboard::FilterResult, pointer::{AxisFrame, ButtonEvent, MotionEvent}},
    reexports::wayland_server::{Display, ListeningSocket},
    utils::{Rectangle, Transform, SERIAL_COUNTER},
};
use std::{sync::{atomic::{AtomicBool, Ordering}, Arc}, time::{Duration, Instant}};

pub fn run(socket: &str, color: [f32; 4]) -> Result<()> {
    let mut display = Display::<Atelier>::new()?;
    let mut state = Atelier::new(&display.handle())?;
    let (mut backend, mut events) = winit::init::<GlesRenderer>()
        .map_err(|error| anyhow::anyhow!("initialize nested GLES backend: {error}"))?;
    let size = backend.window_size();
    state.resize(size.w.max(1), size.h.max(1));
    let running = Arc::new(AtomicBool::new(true));
    let signal_flag = Arc::clone(&running);
    ctrlc::set_handler(move || signal_flag.store(false, Ordering::SeqCst))?;
    let listener = ListeningSocket::bind(socket).context("bind Atelier Wayland socket")?;
    println!("Atelier ready: WAYLAND_DISPLAY={socket}");
    let started = Instant::now();
    while running.load(Ordering::SeqCst) {
        let status = events.dispatch_new_events(|event| {
            if let WinitEvent::Input(input) = event { handle_input(&mut state, input); }
        });
        if let ::winit::platform::pump_events::PumpStatus::Exit(_) = status { return Ok(()); }

        while let Some(stream) = listener.accept()? {
            if let Err(error) = display.handle().insert_client(stream, Arc::new(ClientState::default())) {
                tracing::warn!(?error, "Wayland client connection failed");
            }
        }
        display.dispatch_clients(&mut state)?;
        state.popups.cleanup();
        state.space.refresh();

        let size = backend.window_size();
        if size.w <= 0 || size.h <= 0 {
            display.flush_clients()?;
            std::thread::sleep(Duration::from_millis(16));
            continue;
        }
        state.resize(size.w, size.h);
        let damage = Rectangle::from_size(size);
        let region = state.space.output_geometry(&state.output).context("nested output geometry")?;
        {
            let (renderer, mut target) = backend.bind().context("bind nested framebuffer")?;
            let elements = state.space.render_elements_for_region(renderer, &region, 1.0, 1.0);
            let mut frame = renderer.render(&mut target, size, Transform::Flipped180)?;
            frame.clear(Color32F::new(color[0], color[1], color[2], color[3]), &[damage])?;
            draw_render_elements(&mut frame, 1.0, &elements, &[damage])?;
            let _sync = frame.finish()?;
        }
        let output = &state.output;
        for window in state.space.elements_for_output(output) {
            window.send_frame(output, started.elapsed(), None, |_, _| Some(output.clone()));
        }
        display.flush_clients()?;
        backend.submit(Some(&[damage])).context("present nested framebuffer")?;
        // Bound idle polling; production backend will use calloop and damage-driven scheduling.
        std::thread::sleep(Duration::from_millis(4));
    }
    Ok(())
}

fn handle_input(state: &mut Atelier, input: InputEvent<WinitInput>) {
    let serial = SERIAL_COUNTER.next_serial();
    match input {
        InputEvent::Keyboard { event } => {
            if let Some(keyboard) = state.seat.get_keyboard() {
                keyboard.input::<(), _>(state, event.key_code(), event.state(), serial, event.time_msec(), |_, _, _| FilterResult::Forward);
            }
        }
        InputEvent::PointerMotionAbsolute { event } => {
            if let Some(region) = state.space.output_geometry(&state.output) {
                state.pointer_position = event.position_transformed(region.size);
                let focus = state.surface_under_pointer();
                let location = state.pointer_position;
                if let Some(pointer) = state.seat.get_pointer() {
                    pointer.motion(state, focus, &MotionEvent { location, serial, time: event.time_msec() });
                    pointer.frame(state);
                }
            }
        }
        InputEvent::PointerButton { event } => {
            if let Some(pointer) = state.seat.get_pointer() {
                if event.state() == ButtonState::Pressed && !pointer.is_grabbed() {
                    let window = state.space.element_under(state.pointer_position).map(|(w, _)| w.clone());
                    state.focus(window, serial);
                }
                pointer.button(state, &ButtonEvent { button: event.button_code(), state: event.state(), serial, time: event.time_msec() });
                pointer.frame(state);
            }
        }
        InputEvent::PointerAxis { event } => {
            if let Some(pointer) = state.seat.get_pointer() {
                let source = event.source();
                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let steps = event.amount_v120(axis);
                    if let Some(steps) = steps { frame = frame.v120(axis, steps.round() as i32); }
                    let value = event.amount(axis).or_else(|| steps.map(|steps| steps * 15.0 / 120.0)).unwrap_or(0.0);
                    frame = frame.relative_direction(axis, event.relative_direction(axis));
                    if value != 0.0 { frame = frame.value(axis, value); }
                    else if source == AxisSource::Finger { frame = frame.stop(axis); }
                }
                pointer.axis(state, frame);
                pointer.frame(state);
            }
        }
        _ => {}
    }
}
