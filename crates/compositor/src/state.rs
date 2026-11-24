use smithay::{
    backend::renderer::utils::{on_commit_buffer_handler, RendererSurfaceStateUserData},
    delegate_compositor, delegate_data_device, delegate_output, delegate_seat, delegate_shm, delegate_xdg_shell,
    desktop::{PopupKind, PopupManager, Space, Window, WindowSurfaceType},
    input::{pointer::CursorImageStatus, Seat, SeatHandler, SeatState},
    reexports::wayland_server::{
        backend::{ClientData, ClientId, DisconnectReason},
        protocol::{wl_buffer::WlBuffer, wl_seat::WlSeat, wl_surface::WlSurface},
        Client, DisplayHandle,
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    utils::{Logical, Point, Serial, Transform, SERIAL_COUNTER},
    wayland::{
        buffer::BufferHandler,
        compositor::{get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler, CompositorState},
        output::{OutputHandler, OutputManagerState},
        selection::{data_device::{ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler}, SelectionHandler},
        shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState},
        shm::{ShmHandler, ShmState},
    },
};
use std::os::fd::OwnedFd;

pub struct Atelier {
    display: DisplayHandle,
    compositor: CompositorState,
    shell: XdgShellState,
    shm: ShmState,
    seats: SeatState<Self>,
    data_device: DataDeviceState,
    _outputs: OutputManagerState,
    pub seat: Seat<Self>,
    pub space: Space<Window>,
    pub output: Output,
    pub popups: PopupManager,
    pub pointer_position: Point<f64, Logical>,
    cascade: usize,
    pending: Vec<ToplevelSurface>,
    layout: atelier_config::Layout,
    windows: Vec<Window>,
    active_workspace: usize,
    parked: [Vec<(Window, Point<i32, Logical>)>; 4],
    focused: Option<Window>,
    scene_dirty: bool,
}

impl Atelier {
    pub fn new(display: &DisplayHandle, layout: atelier_config::Layout) -> anyhow::Result<Self> {
        let mut seats = SeatState::new();
        let mut seat = seats.new_wl_seat(display, "atelier-seat");
        seat.add_keyboard(Default::default(), 200, 25)?;
        seat.add_pointer();
        let output = Output::new("ATELIER-NESTED-1".into(), PhysicalProperties {
            size: (0, 0).into(), subpixel: Subpixel::Unknown,
            make: "Atelier".into(), model: "Nested display".into(),
        });
        output.create_global::<Self>(display);
        let mut space = Space::default();
        space.map_output(&output, (0, 0));
        Ok(Self {
            display: display.clone(),
            compositor: CompositorState::new::<Self>(display), shell: XdgShellState::new::<Self>(display),
            shm: ShmState::new::<Self>(display, vec![]), seats,
            data_device: DataDeviceState::new::<Self>(display), _outputs: OutputManagerState::new_with_xdg_output::<Self>(display),
            seat, space, output, popups: PopupManager::default(), pointer_position: (0.0, 0.0).into(), cascade: 0, pending: Vec::new(), layout, windows: Vec::new(),
            active_workspace: 0, parked: std::array::from_fn(|_| Vec::new()), focused: None, scene_dirty: true,
        })
    }

    #[cfg(feature = "native-session")]
    pub fn take_scene_dirty(&mut self) -> bool { std::mem::take(&mut self.scene_dirty) }

    pub fn resize(&mut self, width: i32, height: i32) {
        let mode = Mode { size: (width, height).into(), refresh: 60_000 };
        if self.output.current_mode() != Some(mode) {
            self.scene_dirty = true;
            let previous = self.output.current_mode();
            self.output.change_current_state(Some(mode), Some(Transform::Normal), Some(Scale::Integer(1)), Some((0, 0).into()));
            self.output.set_preferred(mode);
            if let Some(previous) = previous { self.output.delete_mode(previous); }
            self.reflow();
        }
    }

    fn reflow(&mut self) {
        if self.layout != atelier_config::Layout::Grid { return; }
        let Some(mode) = self.output.current_mode() else { return; };
        let Some(tiles) = crate::layout::grid(mode.size.w, mode.size.h, self.windows.len()) else { return; };
        let top = self.space.elements().next_back().cloned();
        for (window, tile) in self.windows.iter().zip(tiles) {
            self.space.map_element(window.clone(), (tile.x, tile.y), false);
            if let Some(surface) = window.toplevel() {
                surface.with_pending_state(|state| state.size = Some((tile.width, tile.height).into()));
                surface.send_pending_configure();
            }
        }
        if let Some(top) = top { self.space.raise_element(&top, false); }
    }

    pub fn focus(&mut self, window: Option<Window>, serial: Serial) {
        self.scene_dirty = true;
        self.focused = window.clone();
        if let Some(window) = &window { self.space.raise_element(window, true); }
        for candidate in self.space.elements() {
            if let Some(toplevel) = candidate.toplevel() {
                if window.is_none() { candidate.set_activated(false); }
                toplevel.send_pending_configure();
            }
        }
        let target = window.as_ref().and_then(Window::toplevel).map(|s| s.wl_surface().clone());
        if let Some(keyboard) = self.seat.get_keyboard() { keyboard.set_focus(self, target, serial); }
    }

    pub fn select_workspace(&mut self, target: usize, move_focused: bool, serial: Serial) {
        if target >= self.parked.len() || target == self.active_workspace { return; }
        if self.seat.get_pointer().is_some_and(|p| p.is_grabbed()) || self.seat.get_keyboard().is_some_and(|k| k.is_grabbed()) { return; }
        if move_focused {
            let Some(window) = self.focused.clone() else { return; };
            let position = self.space.element_location(&window).unwrap_or((24, 24).into());
            self.unmap(&window);
            window.set_activated(false);
            if let Some(surface) = window.toplevel() { surface.send_pending_configure(); }
            self.parked[target].push((window, position));
            return;
        }
        self.focus(None, serial);
        // Preserve stacking and floating coordinates while removing every hidden window from rendering and hit testing.
        let outgoing: Vec<_> = self.space.elements().filter_map(|w| self.space.element_location(w).map(|p| (w.clone(), p))).collect();
        for (window, _) in &outgoing { self.space.unmap_elem(window); }
        self.parked[self.active_workspace] = outgoing;
        self.active_workspace = target;
        let incoming = std::mem::take(&mut self.parked[target]);
        self.windows.clear();
        for (window, position) in incoming {
            self.space.map_element(window.clone(), position, false);
            self.windows.push(window);
        }
        self.reflow();
        let next = self.space.elements().next_back().cloned();
        self.focus(next, serial);
        tracing::info!(workspace = target + 1, "workspace activated");
    }

    fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.windows.iter().chain(self.parked.iter().flatten().map(|(window, _)| window))
            .find(|w| w.toplevel().is_some_and(|s| s.wl_surface() == surface)).cloned()
    }

    pub fn surface_under_pointer(&self) -> Option<(WlSurface, Point<f64, Logical>)> {
        let p = self.pointer_position;
        let (window, origin) = self.space.element_under(p)?;
        let local = (p.x - f64::from(origin.x), p.y - f64::from(origin.y));
        let (surface, offset) = window.surface_under(local, WindowSurfaceType::ALL)?;
        Some((surface, (f64::from(origin.x + offset.x), f64::from(origin.y + offset.y)).into()))
    }

    fn map_toplevel(&mut self, surface: ToplevelSurface) {
        let offset = 24 + (self.cascade % 8) as i32 * 24;
        self.cascade = self.cascade.wrapping_add(1);
        let window = Window::new_wayland_window(surface);
        window.on_commit();
        self.space.map_element(window.clone(), (offset, offset), true);
        self.windows.push(window.clone());
        self.reflow();
        self.focus(Some(window), SERIAL_COUNTER.next_serial());
    }

    fn unmap(&mut self, window: &Window) {
        self.scene_dirty = true;
        for hidden in &mut self.parked { hidden.retain(|(candidate, _)| candidate != window); }
        let had_focus = self.focused.as_ref() == Some(window);
        self.space.unmap_elem(window);
        self.windows.retain(|candidate| candidate != window);
        self.reflow();
        if had_focus {
            let next = self.space.elements().next_back().cloned();
            self.focus(next, SERIAL_COUNTER.next_serial());
        }
    }
}

#[derive(Default)]
pub struct ClientState { compositor: CompositorClientState }
impl ClientData for ClientState {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

impl CompositorHandler for Atelier {
    fn compositor_state(&mut self) -> &mut CompositorState { &mut self.compositor }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().expect("Atelier client data").compositor
    }
    fn commit(&mut self, surface: &WlSurface) {
        self.scene_dirty = true;
        on_commit_buffer_handler::<Self>(surface);
        self.popups.commit(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) { root = parent; }
            let mapped = with_states(&root, |states| {
                states.data_map.get::<RendererSurfaceStateUserData>().is_some_and(|state| state.lock().unwrap().buffer().is_some())
            });
            let existing = self.window_for_surface(&root);
            if let Some(window) = existing {
                if mapped { window.on_commit(); }
                else if let Some(toplevel) = window.toplevel().cloned() {
                    self.unmap(&window);
                    self.pending.push(toplevel);
                }
            } else if let Some(index) = self.pending.iter().position(|s| s.wl_surface() == &root) {
                if !self.pending[index].is_initial_configure_sent() {
                    self.pending[index].send_configure();
                } else if mapped {
                    let surface = self.pending.swap_remove(index);
                    self.map_toplevel(surface);
                }
            }
        }
    }
}
impl BufferHandler for Atelier { fn buffer_destroyed(&mut self, _: &WlBuffer) {} }
impl ShmHandler for Atelier { fn shm_state(&self) -> &ShmState { &self.shm } }
impl OutputHandler for Atelier {}
impl SeatHandler for Atelier {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;
    fn seat_state(&mut self) -> &mut SeatState<Self> { &mut self.seats }
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        use smithay::{reexports::wayland_server::Resource, wayland::selection::data_device::set_data_device_focus};
        let client = focused.and_then(|surface| surface.client());
        set_data_device_focus(&self.display, seat, client);
    }
    // Host cursor remains visible; client cursor surfaces are not rendered in this milestone.
    fn cursor_image(&mut self, _: &Seat<Self>, _: CursorImageStatus) {}
}
impl SelectionHandler for Atelier { type SelectionUserData = (); }
impl DataDeviceHandler for Atelier { fn data_device_state(&self) -> &DataDeviceState { &self.data_device } }
impl ClientDndGrabHandler for Atelier {}
impl ServerDndGrabHandler for Atelier { fn send(&mut self, _: String, _: OwnedFd, _: Seat<Self>) {} }

impl XdgShellHandler for Atelier {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState { &mut self.shell }
    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.pending.push(surface);
    }
    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        surface.with_pending_state(|state| { state.geometry = positioner.get_geometry(); });
        if self.popups.track_popup(PopupKind::Xdg(surface.clone())).is_ok() {
            if let Err(error) = surface.send_configure() { tracing::debug!(?error, "popup configure rejected"); }
        }
    }
    fn grab(&mut self, surface: PopupSurface, _: WlSeat, _: Serial) {
        // Explicit popup grabs need serial/seat/focus validation. Refuse rather than grant an unchecked grab.
        surface.send_popup_done();
    }
    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| { state.geometry = positioner.get_geometry(); });
        surface.send_repositioned(token);
    }
    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.pending.retain(|candidate| candidate != &surface);
        let removed = self.window_for_surface(surface.wl_surface());
        if let Some(window) = removed { self.unmap(&window); }
    }
}

delegate_compositor!(Atelier);
delegate_xdg_shell!(Atelier);
delegate_shm!(Atelier);
delegate_seat!(Atelier);
delegate_data_device!(Atelier);
delegate_output!(Atelier);
