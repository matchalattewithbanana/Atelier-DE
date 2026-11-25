use smithay::{
    backend::renderer::utils::{on_commit_buffer_handler, RendererSurfaceStateUserData},
    delegate_compositor, delegate_data_device, delegate_output, delegate_seat, delegate_shm, delegate_xdg_shell, delegate_layer_shell,
    desktop::{layer_map_for_output, LayerSurface as DesktopLayer, PopupKind, PopupManager, Space, Window, WindowSurfaceType},
    input::{pointer::CursorImageStatus, Seat, SeatHandler, SeatState},
    reexports::wayland_server::{
        backend::{ClientData, ClientId, DisconnectReason},
        protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface},
        Client, DisplayHandle,
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    utils::{Logical, Point, Serial, Transform, SERIAL_COUNTER},
    wayland::{
        buffer::BufferHandler,
        compositor::{get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler, CompositorState},
        output::{OutputHandler, OutputManagerState},
        selection::{data_device::{ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler}, SelectionHandler},
        shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState, XdgToplevelSurfaceData},
        shm::{ShmHandler, ShmState},
        shell::wlr_layer::{Layer, LayerSurface, WlrLayerShellHandler, WlrLayerShellState},
    },
};
use std::os::fd::OwnedFd;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;

/// Fullscreen takes precedence over maximized; restore geometry belongs to
/// the normal placement and survives workspace changes and mode transitions.
#[derive(Clone)]
struct Placement{
    surface:ToplevelSurface,
    normal:Option<Point<i32,Logical>>,
    fullscreen:bool,
    maximized:bool,
}
pub struct Atelier {
    display: DisplayHandle,
    compositor: CompositorState,
    shell: XdgShellState,
    layer_shell: WlrLayerShellState,
    focused_layer: Option<WlSurface>,
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
    task_ids: Vec<(Window,u64)>,
    placements:Vec<Placement>,
    spacing:atelier_config::WindowSpacing,
    border:atelier_config::WindowBorder,
    appearance_monitor:Option<atelier_config::AppearanceMonitor>,
    next_task_id: u64,
    active_workspace: usize,
    parked: [Vec<(Window, Point<i32, Logical>)>; 4],
    focused: Option<Window>,
    scene_dirty: bool,
}

impl Atelier {
    pub fn new(display: &DisplayHandle, layout: atelier_config::Layout) -> anyhow::Result<Self> {
        Self::new_for_output(display, layout, "ATELIER-NESTED-1", "Nested display")
    }

    pub fn new_for_output(display: &DisplayHandle, layout: atelier_config::Layout, name: &str, model: &str) -> anyhow::Result<Self> {
        let mut seats = SeatState::new();
        let mut seat = seats.new_wl_seat(display, "atelier-seat");
        seat.add_keyboard(Default::default(), 200, 25)?;
        seat.add_pointer();
        let output = Output::new(name.into(), PhysicalProperties {
            size: (0, 0).into(), subpixel: Subpixel::Unknown,
            make: "Atelier".into(), model: model.into(),
        });
        output.create_global::<Self>(display);
        let mut space = Space::default();
        space.map_output(&output, (0, 0));
        let appearance=atelier_config::appearance_path().and_then(|p|atelier_config::Appearance::load(&p)).unwrap_or_else(|error|{tracing::warn!(?error,"using default native appearance");Default::default()});
        Ok(Self {
            display: display.clone(),
            compositor: CompositorState::new::<Self>(display), shell: XdgShellState::new::<Self>(display),
            layer_shell: WlrLayerShellState::new::<Self>(display), focused_layer: None,
            shm: ShmState::new::<Self>(display, vec![]), seats,
            data_device: DataDeviceState::new::<Self>(display), _outputs: OutputManagerState::new_with_xdg_output::<Self>(display),
            seat, space, output, popups: PopupManager::default(), pointer_position: (0.0, 0.0).into(), cascade: 0, pending: Vec::new(), layout, windows: Vec::new(), task_ids:Vec::new(), next_task_id:1, placements:Vec::new(), spacing:appearance.window_spacing,border:appearance.window_border,
            appearance_monitor:atelier_config::appearance_path().ok().map(atelier_config::AppearanceMonitor::new),
            active_workspace: 0, parked: std::array::from_fn(|_| Vec::new()), focused: None, scene_dirty: true,
        })
    }

    #[cfg(feature = "native-session")]
    pub fn take_scene_dirty(&mut self) -> bool { std::mem::take(&mut self.scene_dirty) }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.resize_with_refresh(width, height, 60_000);
    }

    pub fn resize_with_refresh(&mut self, width: i32, height: i32, refresh: i32) {
        let mode = Mode { size: (width, height).into(), refresh };
        if self.output.current_mode() != Some(mode) {
            self.scene_dirty = true;
            let previous = self.output.current_mode();
            self.output.change_current_state(Some(mode), Some(Transform::Normal), Some(Scale::Integer(1)), Some((0, 0).into()));
            self.output.set_preferred(mode);
            if let Some(previous) = previous { self.output.delete_mode(previous); }
            layer_map_for_output(&self.output).arrange();
            self.reflow();
        }
    }

    pub(crate) fn window_border(&self,window:&Window)->Option<(&atelier_config::WindowBorder,bool)>{
        (self.border.width>0&&self.placement(window).is_none()).then_some((&self.border,self.focused.as_ref()==Some(window)))
    }
    fn placement(&self,window:&Window)->Option<&Placement>{
        window.toplevel().and_then(|surface|self.placements.iter().find(|p|&p.surface==surface))
    }
    fn is_fullscreen(&self,window:&Window)->bool{
        self.placement(window).is_some_and(|p|p.fullscreen)
    }
    fn reflow(&mut self) {
        let Some(mode)=self.output.current_mode() else{return;};
        let top=self.space.elements().next_back().cloned();
        let zone=layer_map_for_output(&self.output).non_exclusive_zone();
        if self.layout==atelier_config::Layout::Grid{
            let normal:Vec<_>=self.windows.iter().filter(|w|self.placement(w).is_none()).cloned().collect();
            if let Some(tiles)=crate::layout::spaced_grid(zone.size.w,zone.size.h,normal.len(),self.spacing){
                for (window,tile) in normal.iter().zip(tiles){
                    let tile=crate::scene::inset_tile(tile,self.border.width);
                    self.space.map_element(window.clone(),(zone.loc.x+tile.x,zone.loc.y+tile.y),false);
                    if let Some(surface)=window.toplevel(){
                        surface.with_pending_state(|state|state.size=Some((tile.width,tile.height).into()));
                        surface.send_pending_configure();
                    }
                }
            }
        }
        let special:Vec<_>=self.windows.iter().filter_map(|w|self.placement(w).map(|p|(w.clone(),p.fullscreen))).collect();
        for (window,full) in special{
            let (position,size)=if full{((0,0).into(),(mode.size.w,mode.size.h).into())}else{
                (zone.loc,(zone.size.w.max(1),zone.size.h.max(1)).into())
            };
            self.space.map_element(window.clone(),position,false);
            if let Some(surface)=window.toplevel(){
                surface.with_pending_state(|state|state.size=Some(size));
                surface.send_pending_configure();
            }
        }
        if let Some(top)=top{self.space.raise_element(&top,false);}
    }

    fn set_placement(&mut self,surface:ToplevelSurface,fullscreen:bool,enabled:bool){
        let window=self.window_for_surface(surface.wl_surface());
        let Some(mode)=self.output.current_mode()else{return;};
        let zone=layer_map_for_output(&self.output).non_exclusive_zone();
        let index=self.placements.iter().position(|p|p.surface==surface);
        let mut placement=match index{
            Some(index)=>self.placements.remove(index),
            None=>Placement{surface:surface.clone(),normal:window.as_ref().and_then(|w|
                self.space.element_location(w).or_else(||self.parked.iter().flatten().find(|(candidate,_)|candidate==w).map(|(_,p)|*p))),
                fullscreen:false,maximized:false},
        };
        if fullscreen{placement.fullscreen=enabled;}else{placement.maximized=enabled;}
        let active=placement.fullscreen||placement.maximized;
        surface.with_pending_state(|state|{
            if placement.fullscreen{state.states.set(XdgState::Fullscreen);}else{state.states.unset(XdgState::Fullscreen);}
            if placement.maximized&&!placement.fullscreen{state.states.set(XdgState::Maximized);}else{state.states.unset(XdgState::Maximized);}
            state.size=if placement.fullscreen{Some((mode.size.w,mode.size.h).into())}
                else if placement.maximized{Some((zone.size.w.max(1),zone.size.h.max(1)).into())}else{None};
        });
        if active{self.placements.push(placement);}
        else if let (Some(window),Some(location))=(&window,placement.normal){
            if self.windows.contains(window){self.space.map_element(window.clone(),location,false);}
            else{for (candidate,position) in self.parked.iter_mut().flatten(){if candidate==window{*position=location;}}}
        }
        // Reflow may supply the final grid size before sending configure.
        self.scene_dirty=true;self.reflow();
        if surface.is_initial_configure_sent(){surface.send_pending_configure();}
    }

    pub fn focus(&mut self, window: Option<Window>, serial: Serial) {
        self.scene_dirty = true;
        self.focused_layer = None;
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

    /// Apply only geometry settings owned by the compositor. Defer reflow
    /// during grabs so a save cannot invalidate active input interactions.
    pub fn refresh_appearance(&mut self)->bool{
        if self.seat.get_pointer().is_some_and(|p|p.is_grabbed())||self.seat.get_keyboard().is_some_and(|k|k.is_grabbed()){return false;}
        let Some(monitor)=self.appearance_monitor.as_mut()else{return false;};
        match monitor.poll(){
            Ok(Some(appearance)) if appearance.window_spacing!=self.spacing||appearance.window_border!=self.border=>{
                let geometry=appearance.window_spacing!=self.spacing||appearance.window_border.width!=self.border.width;
                self.spacing=appearance.window_spacing;self.border=appearance.window_border;
                if geometry{self.reflow();}self.scene_dirty=true;geometry
            }
            Err(error)=>{tracing::warn!(?error,"retaining last valid native appearance");false}
            _=>false
        }
    }

    pub fn active_workspace(&self) -> usize { self.active_workspace }

    pub fn select_workspace(&mut self, target: usize, move_focused: bool, serial: Serial) {
        if target >= self.parked.len() || target == self.active_workspace { return; }
        if self.seat.get_pointer().is_some_and(|p| p.is_grabbed()) || self.seat.get_keyboard().is_some_and(|k| k.is_grabbed()) { return; }
        if move_focused {
            let Some(window) = self.focused.clone() else { return; };
            let position = self.space.element_location(&window).unwrap_or((24, 24).into());
            let id=self.task_ids.iter().find(|(w,_)|w==&window).map(|(_,id)|*id);
            let placement=self.placement(&window).cloned();
            self.unmap(&window);
            if let Some(placement)=placement{self.placements.push(placement);}
            if let Some(id)=id{self.task_ids.push((window.clone(),id));}
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

    pub fn task_inventory(&self,session:[u8;16])->atelier_config::tasks::TaskSnapshot{
        use atelier_config::tasks::{TaskSnapshot,TaskWindow,MAX_WINDOWS};
        let mut windows=Vec::new();
        let mut truncated=false;
        let all=self.windows.iter().map(|w|(self.active_workspace,w)).chain(
            self.parked.iter().enumerate().flat_map(|(index,items)|items.iter().map(move |(w,_)|(index,w))));
        for (workspace,window) in all{
            let Some((_,id))=self.task_ids.iter().find(|(w,_)|w==window)else{continue;};
            if windows.len()>=MAX_WINDOWS{truncated=true;break;}
            let Some(surface)=window.toplevel()else{continue;};
            let (app_id,title)=with_states(surface.wl_surface(),|states|{
                states.data_map.get::<XdgToplevelSurfaceData>().and_then(|data|{
                    let data=data.lock().ok()?;
                    Some((data.app_id.clone().unwrap_or_default(),data.title.clone().unwrap_or_default()))
                }).unwrap_or_default()
            });
            let clean=|text:String,limit:usize|text.chars().filter(|c|!c.is_control()).take(limit).collect::<String>();
            windows.push(TaskWindow{id:*id,app_id:clean(app_id,128),title:clean(title,256),
                workspace:(workspace+1) as u8,focused:self.focused.as_ref()==Some(window)});
        }
        windows.sort_by_key(|w|w.id);
        let fullscreen_active=self.focused.as_ref().is_some_and(|w|self.is_fullscreen(w));
        TaskSnapshot{version:2,session,truncated,windows,fullscreen_active:Some(fullscreen_active)}
    }

    pub fn activate_task(&mut self,id:u64,serial:Serial)->bool{
        if self.seat.get_pointer().is_some_and(|p|p.is_grabbed())||self.seat.get_keyboard().is_some_and(|k|k.is_grabbed()){return false;}
        // Do not steal keyboard focus from an interactive layer surface.
        if self.focused_layer.is_some(){return false;}
        let Some(window)=self.task_ids.iter().find(|(_,key)|*key==id).map(|(w,_)|w.clone())else{return false;};
        if !self.windows.contains(&window){
            let Some(target)=self.parked.iter().position(|items|items.iter().any(|(w,_)|w==&window))else{return false;};
            self.select_workspace(target,false,serial);
        }
        if !self.windows.contains(&window){return false;}
        self.focus(Some(window),serial);
        true
    }

    fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.windows.iter().chain(self.parked.iter().flatten().map(|(window, _)| window))
            .find(|w| w.toplevel().is_some_and(|s| s.wl_surface() == surface)).cloned()
    }

    fn layer_hit(&self, layers: &[Layer]) -> Option<(DesktopLayer, WlSurface, Point<f64, Logical>)> {
        let p = self.pointer_position;
        let map = layer_map_for_output(&self.output);
        for &level in layers {
            for layer in map.layers_on(level).rev() {
                let Some(geometry) = map.layer_geometry(layer) else { continue; };
                let origin = geometry.loc - layer.bbox().loc;
                let local = (p.x - f64::from(origin.x), p.y - f64::from(origin.y));
                if let Some((surface, offset)) = layer.surface_under(local, WindowSurfaceType::ALL) {
                    return Some((layer.clone(), surface, (f64::from(origin.x + offset.x), f64::from(origin.y + offset.y)).into()));
                }
            }
        }
        None
    }

    pub fn surface_under_pointer(&self) -> Option<(WlSurface, Point<f64, Logical>)> {
        if let Some((_, surface, origin)) = self.layer_hit(&[Layer::Overlay, Layer::Top]) {
            return Some((surface, origin));
        }
        let p = self.pointer_position;
        if let Some((window, origin)) = self.space.element_under(p) {
            let local = (p.x - f64::from(origin.x), p.y - f64::from(origin.y));
            if let Some((surface, offset)) = window.surface_under(local, WindowSurfaceType::ALL) {
                return Some((surface, (f64::from(origin.x + offset.x), f64::from(origin.y + offset.y)).into()));
            }
        }
        self.layer_hit(&[Layer::Bottom, Layer::Background]).map(|(_, surface, origin)| (surface, origin))
    }

    pub fn focus_pointer(&mut self, serial: Serial) {
        let layer = self.layer_hit(&[Layer::Overlay, Layer::Top]).map(|(layer, _, _)| layer);
        let window = self.space.element_under(self.pointer_position).map(|(window, _)| window.clone());
        let layer = layer.or_else(|| if window.is_none() { self.layer_hit(&[Layer::Bottom, Layer::Background]).map(|(layer, _, _)| layer) } else { None });
        if let Some(layer) = layer {
            // Non-interactive layer clicks never focus the application behind them.
            if layer.can_receive_keyboard_focus() {
                self.focus(None, serial);
                let surface = layer.wl_surface().clone();
                self.focused_layer = Some(surface.clone());
                if let Some(keyboard) = self.seat.get_keyboard() { keyboard.set_focus(self, Some(surface), serial); }
            }
        } else {
            self.focus(window, serial);
        }
    }

    pub fn send_frames(&self, time: std::time::Duration) {
        for window in self.space.elements_for_output(&self.output) {
            window.send_frame(&self.output, time, None, |_, _| Some(self.output.clone()));
        }
        let map = layer_map_for_output(&self.output);
        for layer in map.layers() {
            layer.send_frame(&self.output, time, None, |_, _| Some(self.output.clone()));
        }
    }

    fn map_toplevel(&mut self, surface: ToplevelSurface) {
        let offset = 24 + (self.cascade % 8) as i32 * 24;
        self.cascade = self.cascade.wrapping_add(1);
        if let Some(p)=self.placements.iter_mut().find(|p|p.surface==surface){if p.normal.is_none(){p.normal=Some((offset,offset).into());}}
        let window = Window::new_wayland_window(surface);
        window.on_commit();
        self.space.map_element(window.clone(), (offset, offset), true);
        self.windows.push(window.clone());
        if let Some(next)=self.next_task_id.checked_add(1){self.task_ids.push((window.clone(),self.next_task_id));self.next_task_id=next;}
        self.reflow();
        self.focus(Some(window), SERIAL_COUNTER.next_serial());
    }

    fn unmap(&mut self, window: &Window) {
        self.scene_dirty = true;
        self.task_ids.retain(|(candidate,_)|candidate!=window);
        if let Some(surface)=window.toplevel(){self.placements.retain(|p|&p.surface!=surface);}
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
            let layer = { layer_map_for_output(&self.output).layer_for_surface(&root, WindowSurfaceType::TOPLEVEL).cloned() };
            if let Some(layer) = layer {
                let changed = layer_map_for_output(&self.output).arrange();
                layer.layer_surface().send_pending_configure();
                if changed { self.reflow(); }
                return;
            }
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
    fn fullscreen_request(&mut self,surface:ToplevelSurface,_:Option<WlOutput>){self.set_placement(surface,true,true);}
    fn unfullscreen_request(&mut self,surface:ToplevelSurface){self.set_placement(surface,true,false);}
    fn maximize_request(&mut self,surface:ToplevelSurface){self.set_placement(surface,false,true);}
    fn unmaximize_request(&mut self,surface:ToplevelSurface){self.set_placement(surface,false,false);}
    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.placements.retain(|p|p.surface!=surface);
        self.pending.retain(|candidate| candidate != &surface);
        let removed = self.window_for_surface(surface.wl_surface());
        if let Some(window) = removed { self.unmap(&window); }
    }
}

impl WlrLayerShellHandler for Atelier {
    fn shell_state(&mut self) -> &mut WlrLayerShellState { &mut self.layer_shell }

    fn new_layer_surface(&mut self, surface: LayerSurface, _: Option<WlOutput>, _: Layer, namespace: String) {
        // There is exactly one output in this milestone. LayerMap handles anchors,
        // margins and exclusive zones; initial configure is sent on first commit.
        let layer = DesktopLayer::new(surface.clone(), namespace);
        let result = layer_map_for_output(&self.output).map_layer(&layer);
        if let Err(error) = result {
            tracing::warn!(?error, "could not map layer surface");
            surface.send_close();
        } else {
            self.scene_dirty = true;
            self.reflow();
        }
    }

    fn new_popup(&mut self, _: LayerSurface, popup: PopupSurface) {
        // The xdg-shell handler tracks popups; layer-shell assigns their parent.
        // Tracking again is unnecessary and may reject a duplicate registration.
        let _ = popup;
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        let mapped = { layer_map_for_output(&self.output).layer_for_surface(surface.wl_surface(), WindowSurfaceType::TOPLEVEL).cloned() };
        if let Some(layer) = mapped {
            layer_map_for_output(&self.output).unmap_layer(&layer);
        }
        self.scene_dirty = true;
        self.reflow();
        if self.focused_layer.as_ref() == Some(surface.wl_surface()) {
            let next = self.space.elements().next_back().cloned();
            self.focus(next, SERIAL_COUNTER.next_serial());
        }
    }
}

delegate_layer_shell!(Atelier);
delegate_compositor!(Atelier);
delegate_xdg_shell!(Atelier);
delegate_shm!(Atelier);
delegate_seat!(Atelier);
delegate_data_device!(Atelier);
delegate_output!(Atelier);
