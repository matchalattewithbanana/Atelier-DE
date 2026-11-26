//! Experimental black-only lock client. No executable or unlock authority.
//! The caller must provide a compositor-created private socket, never an
//! ambient desktop socket. This is not yet an interactive or secure locker.
// The black-only client binds keyboard events but never enables entry.
#[allow(dead_code)]
mod entry;
mod keyboard;
mod seat;
// Rendering is not connected to the black-only client yet.
#[allow(dead_code)]
mod prompt;

use anyhow::{bail, Context, Result};
use std::{fs::File, os::{fd::{AsFd, FromRawFd}, unix::net::UnixStream}};
use wayland_client::{
    delegate_noop, Connection, Dispatch, QueueHandle,
    protocol::{wl_registry, wl_compositor, wl_output, wl_surface, wl_shm, wl_shm_pool, wl_buffer, wl_seat},
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1 as manager,
    ext_session_lock_v1 as lock,
    ext_session_lock_surface_v1 as role,
};

const MAX_BYTES: u32 = 64 * 1024 * 1024;

#[derive(Default)]
struct State {
    compositor: Option<u32>,
    shm_name: Option<u32>,
    output: Option<u32>,
    manager: Option<u32>,
    seat_name: Option<u32>,
    input: seat::Input,
    shm: Option<wl_shm::WlShm>,
    surface: Option<wl_surface::WlSurface>,
    configured: bool,
    confirmed: bool,
    started: bool,
    failure: Option<&'static str>,
}

impl State {
    fn global(&mut self, name: u32, interface: &str) {
        let target = match interface {
            "wl_compositor" => &mut self.compositor,
            "wl_shm" => &mut self.shm_name,
            "wl_output" => &mut self.output,
            "ext_session_lock_manager_v1" => &mut self.manager,
            "wl_seat" => &mut self.seat_name,
            _ => return,
        };
        if self.started || target.is_some() {
            self.fail("unsupported global or output topology change");
        } else {
            *target = Some(name);
        }
    }

    fn remove(&mut self, name: u32) {
        if [self.compositor, self.shm_name, self.output, self.manager,self.seat_name].contains(&Some(name)) {
            self.fail("required global removed");
        }
    }

    fn fail(&mut self, reason:&'static str){
        self.input.revoke();self.failure=Some(reason);
    }

    fn check(&self) -> Result<()> {
        if let Some(reason) = self.failure { bail!(reason); }
        Ok(())
    }
}

fn layout(width: u32, height: u32) -> Option<(i32, i32, i32, i32)> {
    if width == 0 || height == 0 { return None; }
    let stride = width.checked_mul(4)?;
    let bytes = stride.checked_mul(height)?;
    if bytes > MAX_BYTES { return None; }
    Some((width as i32, height as i32, stride as i32, bytes as i32))
}

// Allocate fresh anonymous backing. Wayland SHM servers can require a
// shared writable mapping, so seal the size, not the contents. The client
// never writes or reuses a frame after handing its fd to the server.
fn allocate_file(bytes:i32)->Result<File>{
    if bytes<=0||bytes as u32>MAX_BYTES{bail!("invalid lock buffer size");}
    let fd=unsafe{libc::memfd_create(b"atelier-recovery-frame\0".as_ptr().cast(),libc::MFD_CLOEXEC|libc::MFD_ALLOW_SEALING)};
    if fd<0{return Err(std::io::Error::last_os_error()).context("allocate lock buffer");}
    let file=unsafe{File::from_raw_fd(fd)};
    file.set_len(bytes as u64)?;
    Ok(file)
}
fn seal_size(file:&File)->Result<()>{
    use std::os::fd::AsRawFd;
    let seals=libc::F_SEAL_SHRINK|libc::F_SEAL_GROW|libc::F_SEAL_SEAL;
    if unsafe{libc::fcntl(file.as_raw_fd(),libc::F_ADD_SEALS,seals)}!=0{
        return Err(std::io::Error::last_os_error()).context("seal lock buffer");
    }
    Ok(())
}
// Fresh anonymous memory is zero-filled. XRGB8888 ignores the unused alpha.
fn black_file(bytes:i32)->Result<File>{
    let file=allocate_file(bytes)?;seal_size(&file)?;Ok(file)
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(state: &mut Self, _: &wl_registry::WlRegistry, event: wl_registry::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_registry::Event::Global { name, interface, version } => {
                if interface=="wl_seat" && version<3 {state.fail("unsupported seat protocol version");}
                else {state.global(name,&interface);}
            },
            wl_registry::Event::GlobalRemove { name } => state.remove(name),
            _ => {},
        }
    }
}

impl Dispatch<lock::ExtSessionLockV1, ()> for State {
    fn event(state: &mut Self, _: &lock::ExtSessionLockV1, event: lock::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            lock::Event::Locked if !state.confirmed => state.confirmed = true,
            lock::Event::Locked => state.fail("duplicate lock confirmation"),
            lock::Event::Finished => state.fail("lock rejected or revoked"),
            _ => {},
        }
    }
}

impl Dispatch<role::ExtSessionLockSurfaceV1, ()> for State {
    fn event(state: &mut Self, proxy: &role::ExtSessionLockSurfaceV1, event: role::Event,
        _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let role::Event::Configure { serial, width, height } = event {
            // The compositor currently supports one fixed unscaled output.
            // Do not acknowledge or commit on unsupported reconfiguration.
            if state.failure.is_some() || state.configured {
                state.fail("unsupported lock surface reconfiguration");
                return;
            }
            let Some((w,h,stride,bytes)) = layout(width,height) else {
                state.fail("invalid lock buffer dimensions");
                return;
            };
            let Ok(file) = black_file(bytes) else {
                state.fail("could not allocate protected buffer");
                return;
            };
            let (Some(shm), Some(surface)) = (&state.shm, &state.surface) else {
                state.fail("missing lock surface prerequisites");
                return;
            };
            let pool = shm.create_pool(file.as_fd(), bytes, qh, ());
            let buffer = pool.create_buffer(0,w,h,stride,wl_shm::Format::Xrgb8888,qh,());
            pool.destroy();
            proxy.ack_configure(serial);
            surface.attach(Some(&buffer),0,0);
            surface.damage(0,0,w,h);
            surface.commit();
            // Server retains the size-sealed backing storage for its own use.
            // Buffer release handler destroys the protocol object.
            state.configured = true;
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(_: &mut Self, buffer: &wl_buffer::WlBuffer, event: wl_buffer::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_buffer::Event::Release = event { buffer.destroy(); }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_output::WlOutput);
delegate_noop!(State: ignore manager::ExtSessionLockManagerV1);

/// Exercise the private protocol with an opaque black surface. Never sends
/// unlock_and_destroy, accepts password input, or performs authentication.
/// An optional single seat is bound to validate keyboard protocol routing;
/// entry readiness remains false because this surface has no prompt.
/// Blocks until disconnection or failure; the supervising compositor must
/// keep the desktop protected even if this client exits. No runtime caller
/// exists. Do not invoke this on an ordinary desktop session.
pub fn hold_black(socket: UnixStream) -> Result<()> {
    let connection = Connection::from_socket(socket).context("private Wayland connection")?;
    let mut queue = connection.new_event_queue::<State>();
    let qh = queue.handle();
    let registry = connection.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state)?;
    state.check()?;
    let compositor: wl_compositor::WlCompositor = registry.bind(state.compositor.context("missing compositor")?, 1, &qh, ());
    let shm: wl_shm::WlShm = registry.bind(state.shm_name.context("missing shared memory")?, 1, &qh, ());
    let output: wl_output::WlOutput = registry.bind(state.output.context("missing output")?, 1, &qh, ());
    let manager: manager::ExtSessionLockManagerV1 = registry.bind(state.manager.context("missing session lock")?, 1, &qh, ());
    state.started = true;
    let surface = compositor.create_surface(&qh, ());
    state.shm = Some(shm);
    state.surface = Some(surface.clone());
    if let Some(name)=state.seat_name{
        let seat:wl_seat::WlSeat=registry.bind(name,3,&qh,());
        state.input.bind(seat);
    }
    let lock = manager.lock(&qh, ());
    let _role = lock.get_lock_surface(&surface, &output, &qh, ());
    loop {
        queue.blocking_dispatch(&mut state)?;
        state.check()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dimensions_are_bounded_before_allocation() {
        assert_eq!(layout(1920,1080), Some((1920,1080,7680,8294400)));
        for (w,h) in [(0,1),(1,0),(u32::MAX,1),(8192,8192)] {
            assert_eq!(layout(w,h),None);
        }
    }
    #[test]
    fn black_backing_is_zero_filled_and_size_sealed() {
        use std::io::Read;
        let mut file = black_file(16).unwrap();
        let mut data = [255;16];
        file.read_exact(&mut data).unwrap();
        assert_eq!(data,[0;16]);
        assert!(file.set_len(32).is_err());
        assert!(file.set_len(0).is_err());
    }
    #[test]
    fn frame_supports_server_shared_writable_mapping() {
        use std::os::fd::AsRawFd;
        let file=black_file(4096).unwrap();
        // Smithay's SHM pool requires READ|WRITE and MAP_SHARED.
        let mapping=unsafe{libc::mmap(std::ptr::null_mut(),4096,
            libc::PROT_READ|libc::PROT_WRITE,libc::MAP_SHARED,file.as_raw_fd(),0)};
        assert_ne!(mapping,libc::MAP_FAILED);
        assert_eq!(unsafe{libc::munmap(mapping,4096)},0);
        assert!(file.set_len(0).is_err());
        assert!(file.set_len(8192).is_err());
    }
    #[test]
    fn refuses_duplicate_or_removed_outputs() {
        let mut state = State::default();
        state.global(1,"wl_output");
        assert!(state.check().is_ok());
        state.global(2,"wl_output");
        assert!(state.check().is_err());
        let mut state = State::default();
        state.global(1,"wl_output");
        state.started = true;
        state.remove(1);
        assert!(state.check().is_err());
    }
}
