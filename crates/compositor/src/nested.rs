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

// Conservative refresh while invalidation coverage is incomplete. This is
// a maximum idle interval, not a frame rate cap for active clients.
const IDLE_REFRESH: Duration = Duration::from_millis(250);
fn needs_frame(dirty: bool, last: Option<Instant>, now: Instant) -> bool {
    dirty || last.is_none_or(|presented| now.saturating_duration_since(presented) >= IDLE_REFRESH)
}

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
    let mut workspace_control = crate::workspace_control::WorkspaceControl::bind(socket)?;
    let mut intercepted_keys = HashSet::new();
    let mut shaders=crate::scene::SceneShaders::default();
    let mut metrics = crate::frame_metrics::FrameMetrics::new("nested");
    let mut last_present = None;
    let mut dirty = true;
    while running.load(Ordering::SeqCst) {
        metrics.report_due();
        let status = events.dispatch_new_events(|event| {
            // Input, host resize and expose/redraw notifications may change
            // the next frame. Treat all backend events conservatively.
            dirty = true;
            if let WinitEvent::Input(input) = event { handle_input(&mut state, input, &mut intercepted_keys); }
        });
        if let ::winit::platform::pump_events::PumpStatus::Exit(_) = status { return Ok(()); }

        while let Some(stream) = listener.accept()? {
            dirty = true;
            if let Err(error) = display.handle().insert_client(stream, Arc::new(ClientState::default())) {
                tracing::warn!(?error, "Wayland client connection failed");
            }
        }
        workspace_control.poll(&mut state, started.elapsed().as_millis() as u32)?;
        if display.dispatch_clients(&mut state)? > 0 { dirty = true; }
        state.popups.cleanup();
        state.space.refresh();

        let size = backend.window_size();
        if size.w <= 0 || size.h <= 0 {
            // Force a full refresh when the host window becomes drawable again.
            dirty = true;
            display.flush_clients()?;
            std::thread::sleep(Duration::from_millis(16));
            continue;
        }
        state.resize(size.w, size.h);
        // Consume the scene and animation invalidation after applying resize.
        dirty |= state.take_scene_dirty();
        if !needs_frame(dirty, last_present, Instant::now()) {
            display.flush_clients()?;
            std::thread::sleep(Duration::from_millis(4));
            continue;
        }
        let mut timing = metrics.begin_frame();
        let damage = Rectangle::from_size(size);
        timing.repaint(damage,damage);
        {
            let (renderer, mut target) = backend.bind().context("bind nested framebuffer")?;
            let elements = crate::scene::elements(&state,renderer,&mut shaders,color)?;
            timing.scene_ready();
            let mut frame = renderer.render(&mut target, size, Transform::Flipped180)?;
            let clear=if state.desktop_visible(){Color32F::new(color[0],color[1],color[2],color[3])}else{Color32F::new(0.0,0.0,0.0,1.0)};
            frame.clear(clear, &[damage])?;
            draw_render_elements(&mut frame, 1.0, &elements, &[damage])?;
            let _sync = frame.finish()?;
        }
        timing.render_ready();
        state.send_frames(started.elapsed());
        display.flush_clients()?;
        backend.submit(Some(&[damage])).context("present nested framebuffer")?;
        metrics.submitted(timing);
        last_present = Some(Instant::now());
        dirty = false;
        // Keep protocol polling responsive, independently of presentation.
        std::thread::sleep(Duration::from_millis(4));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_redraws_only_on_invalidation_or_idle_refresh() {
        let now = Instant::now();
        assert!(needs_frame(false, None, now));
        assert!(needs_frame(true, Some(now), now));
        assert!(!needs_frame(false, Some(now), now));
        assert!(!needs_frame(false, Some(now), now + IDLE_REFRESH - Duration::from_millis(1)));
        assert!(needs_frame(false, Some(now), now + IDLE_REFRESH));
        assert!(needs_frame(true, Some(now), now + Duration::from_millis(1)));
    }
}
