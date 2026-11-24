//! Winit/GLES backend. Keep backend-specific presentation/input separate from desktop state.

use crate::{input_router::handle_input, state::{Atelier, ClientState}};
use anyhow::{Context, Result};
use smithay::{
    backend::{
        renderer::{gles::GlesRenderer, utils::draw_render_elements, Color32F, Frame, Renderer},
        winit::{self, WinitEvent},
    },
    reexports::wayland_server::{Display, ListeningSocket},
    utils::{Rectangle, Transform},
};
use std::{collections::HashSet, sync::{atomic::{AtomicBool, Ordering}, Arc}, time::{Duration, Instant}};

pub fn run(socket: &str, color: [f32; 4], layout: atelier_config::Layout) -> Result<()> {
    let mut display = Display::<Atelier>::new()?;
    let mut state = Atelier::new(&display.handle(), layout)?;
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
    let mut intercepted_keys = HashSet::new();
    while running.load(Ordering::SeqCst) {
        let status = events.dispatch_new_events(|event| {
            if let WinitEvent::Input(input) = event { handle_input(&mut state, input, &mut intercepted_keys); }
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
        {
            let (renderer, mut target) = backend.bind().context("bind nested framebuffer")?;
            let elements = state.space.render_elements_for_output(renderer, &state.output, 1.0).map_err(|error| anyhow::anyhow!("render nested output: {error:?}"))?;
            let mut frame = renderer.render(&mut target, size, Transform::Flipped180)?;
            frame.clear(Color32F::new(color[0], color[1], color[2], color[3]), &[damage])?;
            draw_render_elements(&mut frame, 1.0, &elements, &[damage])?;
            let _sync = frame.finish()?;
        }
        state.send_frames(started.elapsed());
        display.flush_clients()?;
        backend.submit(Some(&[damage])).context("present nested framebuffer")?;
        // Bound idle polling; production backend will use calloop and damage-driven scheduling.
        std::thread::sleep(Duration::from_millis(4));
    }
    Ok(())
}

