//! Query KMS resources without changing display state.
use anyhow::{bail, Context, Result};
use smithay::reexports::drm::{self, control::{self, connector, Device as ControlDevice, ModeTypeFlags}};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

/// Borrow the descriptor from the seat owner; never take or drop ownership.
struct Card<'a>(&'a OwnedFd);
impl AsFd for Card<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> { self.0.as_fd() }
}
impl drm::Device for Card<'_> {}
impl ControlDevice for Card<'_> {}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ModeRank {
    preferred: bool,
    pixels: u32,
    refresh: u32,
}

fn rank(preferred: bool, size: (u16, u16), refresh: u32) -> ModeRank {
    ModeRank {
        preferred,
        pixels: u32::from(size.0) * u32::from(size.1),
        refresh,
    }
}

pub fn check(fd: &OwnedFd) -> Result<()> {
    let card = Card(fd);
    let resources = card.resource_handles().context("query KMS resource handles")?;
    println!("KMS resources: {} connectors, {} CRTCs, {} encoders", resources.connectors().len(), resources.crtcs().len(), resources.encoders().len());
    let mut candidates = 0;
    for &handle in resources.connectors() {
        // Do not force a connector reprobe, which can interfere with active hardware.
        let output = match card.get_connector(handle, false) {
            Ok(output) => output,
            Err(error) => { eprintln!("Could not query {handle:?}: {error}"); continue; }
        };
        println!("Connector {handle:?}: {:?}", output.state());
        if output.state() != connector::State::Connected { continue; }

        let mode = output.modes().iter()
            .filter(|mode| { let (w, h) = mode.size(); w > 0 && h > 0 })
            .max_by_key(|mode| rank(mode.mode_type().contains(ModeTypeFlags::PREFERRED), mode.size(), mode.vrefresh()));
        let Some(mode) = mode else { println!("  No usable advertised mode"); continue; };
        let mut crtcs = Vec::new();
        for &encoder_handle in output.encoders() {
            match card.get_encoder(encoder_handle) {
                Ok(encoder) => {
                    for crtc in resources.filter_crtcs(encoder.possible_crtcs()) {
                        if !crtcs.contains(&crtc) { crtcs.push(crtc); }
                    }
                }
                Err(error) => eprintln!("  Could not query encoder {encoder_handle:?}: {error}"),
            }
        }
        let (width, height) = mode.size();
        println!("  Candidate mode: {width}x{height} @ {} Hz ({})", mode.vrefresh(), mode.name().to_string_lossy());
        println!("  Compatible CRTCs: {crtcs:?}");
        if !crtcs.is_empty() { candidates += 1; }
    }
    if candidates == 0 { bail!("no connected connector with an advertised mode and compatible CRTC was found"); }
    println!("{candidates} connected output candidate(s). No mode, CRTC, plane or framebuffer was changed.");
    println!("Candidate routes are not reservations; plane formats, multi-output conflicts and atomic TEST_ONLY validation remain pending.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferred_mode_beats_larger_non_preferred_mode() {
        assert!(rank(true, (1920, 1080), 60) > rank(false, (3840, 2160), 144));
    }
    #[test]
    fn resolution_then_refresh_break_ties() {
        assert!(rank(false, (2560, 1440), 60) > rank(false, (1920, 1080), 144));
        assert!(rank(false, (1920, 1080), 144) > rank(false, (1920, 1080), 60));
    }
}

// Append a property by its kernel-provided name, never assuming property IDs.
fn property<H: control::ResourceHandle>(card: &Card<'_>, object: H, name: &str) -> Result<(control::property::Handle, u64)> {
    let values = card.get_properties(object).context("query object properties")?;
    for (&handle, &value) in values.iter() {
        if card.get_property(handle)?.name().to_bytes() == name.as_bytes() {
            return Ok((handle, value));
        }
    }
    bail!("KMS object is missing property {name}")
}

fn set<H: control::ResourceHandle>(card: &Card<'_>, request: &mut control::atomic::AtomicModeReq, object: H, name: &str, value: u64) -> Result<()> {
    let (handle, _) = property(card, object, name)?;
    request.add_raw_property(object.into(), handle, value);
    Ok(())
}

#[derive(Clone, Copy)]
struct Route {
    connector: connector::Handle,
    crtc: control::crtc::Handle,
    plane: control::plane::Handle,
    mode: control::Mode,
}

fn find_route(card: &Card<'_>) -> Result<Route> {
    use smithay::backend::allocator::Fourcc;
    let resources = card.resource_handles()?;
    let planes = card.plane_handles()?;
    for &connector in resources.connectors() {
        let output = card.get_connector(connector, false)?;
        if output.state() != connector::State::Connected { continue; }
        let Some(mode) = output.modes().iter().filter(|mode| {
            let (w, h) = mode.size();
            w > 0 && h > 0
        }).max_by_key(|mode| rank(mode.mode_type().contains(ModeTypeFlags::PREFERRED), mode.size(), mode.vrefresh())).copied() else { continue; };
        for &encoder in output.encoders() {
            let info = card.get_encoder(encoder)?;
            for crtc in resources.filter_crtcs(info.possible_crtcs()) {
                for &plane in &planes {
                    let info = card.get_plane(plane)?;
                    if !resources.filter_crtcs(info.possible_crtcs()).contains(&crtc) { continue; }
                    if !info.formats().contains(&(Fourcc::Xrgb8888 as u32)) { continue; }
                    // DRM_PLANE_TYPE_PRIMARY is 1; the kernel exposes it via the immutable type property.
                    if property(card, plane, "type")?.1 != 1 { continue; }
                    return Ok(Route { connector, crtc, plane, mode });
                }
            }
        }
    }
    bail!("no connected XRGB8888 primary-plane route found")
}

/// Validate one explicit candidate. TEST_ONLY is mandatory and never configurable.
pub fn check_atomic(fd: &OwnedFd) -> Result<()> {
    use drm::{Device as _, ClientCapability};
    use drm::buffer::PlanarBuffer;
    use control::{AtomicCommitFlags, FbCmd2Flags, property::Value};
    use smithay::{backend::allocator::{
        gbm::{GbmAllocator, GbmBufferFlags, GbmDevice}, Allocator, Fourcc, Modifier,
    }, utils::DeviceFd};

    let card = Card(fd);
    card.set_client_capability(ClientCapability::UniversalPlanes, true).context("enable universal plane queries")?;
    card.set_client_capability(ClientCapability::Atomic, true).context("enable atomic KMS on this descriptor")?;
    let route = find_route(&card)?;
    let (width, height) = route.mode.size();
    // Keep a bounded diagnostic allocation even if a driver advertises an unusual mode.
    if u64::from(width) * u64::from(height) > 16_777_216 {
        bail!("candidate mode exceeds the diagnostic allocation limit");
    }
    let device = GbmDevice::new(DeviceFd::from(fd.try_clone()?))
        .map_err(|error| anyhow::anyhow!("create GBM device: {error}"))?;
    let mut allocator = GbmAllocator::new(device, GbmBufferFlags::SCANOUT | GbmBufferFlags::RENDERING);
    let buffer = allocator.create_buffer(u32::from(width), u32::from(height), Fourcc::Xrgb8888, &[Modifier::Linear])
        .map_err(|error| anyhow::anyhow!("allocate linear scanout GBM buffer: {error}"))?;
    let flags = if PlanarBuffer::modifier(&*buffer).is_some() { FbCmd2Flags::MODIFIERS } else { FbCmd2Flags::empty() };
    let fb = card.add_planar_framebuffer(&*buffer, flags).context("register temporary DRM framebuffer")?;

    // Cleanup stays outside the request closure so every error path releases the framebuffer.
    let outcome = (|| -> Result<()> {
        let Value::Blob(blob) = card.create_property_blob(&route.mode).context("create temporary mode blob")? else {
            bail!("DRM did not return a mode blob");
        };
        let result = (|| -> Result<()> {
            let mut request = control::atomic::AtomicModeReq::new();
            let crtc_id = u64::from(u32::from(route.crtc));
            set(&card, &mut request, route.connector, "CRTC_ID", crtc_id)?;
            set(&card, &mut request, route.crtc, "MODE_ID", blob)?;
            set(&card, &mut request, route.crtc, "ACTIVE", 1)?;
            set(&card, &mut request, route.plane, "FB_ID", u64::from(u32::from(fb)))?;
            set(&card, &mut request, route.plane, "CRTC_ID", crtc_id)?;
            for name in ["SRC_X", "SRC_Y", "CRTC_X", "CRTC_Y"] {
                set(&card, &mut request, route.plane, name, 0)?;
            }
            set(&card, &mut request, route.plane, "SRC_W", u64::from(width) << 16)?;
            set(&card, &mut request, route.plane, "SRC_H", u64::from(height) << 16)?;
            set(&card, &mut request, route.plane, "CRTC_W", u64::from(width))?;
            set(&card, &mut request, route.plane, "CRTC_H", u64::from(height))?;

            card.atomic_commit(AtomicCommitFlags::TEST_ONLY | AtomicCommitFlags::ALLOW_MODESET, request)
                .context("atomic TEST_ONLY validation rejected by driver")?;
            println!("KMS TEST_ONLY accepted {:?}, {:?}, {:?}: {width}x{height}", route.connector, route.crtc, route.plane);
            Ok(())
        })();
        let cleanup = card.destroy_property_blob(blob).context("destroy temporary mode blob");
        if let Err(error) = &cleanup { if result.is_err() { eprintln!("{error:#}"); } }
        result.and(cleanup)
    })();
    let cleanup = card.destroy_framebuffer(fb).context("destroy temporary framebuffer");
    if let Err(error) = &cleanup { if outcome.is_err() { eprintln!("{error:#}"); } }
    outcome.and(cleanup)?;
    println!("No display state was applied; actual scanout remains untested.");
    Ok(())
}

// The preview deliberately operates on an already active, unscaled primary
// plane. Only FB_ID changes, so no mode blob or connector reassignment is needed.
fn active_route(card: &Card<'_>) -> Result<(Route, u64)> {
    use smithay::backend::allocator::Fourcc;
    let resources = card.resource_handles()?;
    let planes = card.plane_handles()?;
    for &connector in resources.connectors() {
        let info = card.get_connector(connector, false)?;
        if info.state() != connector::State::Connected { continue; }
        let crtc_id = property(card, connector, "CRTC_ID")?.1;
        let Some(crtc) = u32::try_from(crtc_id).ok().and_then(control::from_u32::<control::crtc::Handle>) else { continue; };
        let Some(mode) = card.get_crtc(crtc)?.mode() else { continue; };
        if property(card, crtc, "ACTIVE")?.1 != 1 { continue; }
        // Refuse mirrored routes to keep this preview limited to one monitor.
        let mut attached = 0;
        for &other in resources.connectors() {
            if property(card, other, "CRTC_ID")?.1 == crtc_id { attached += 1; }
        }
        if attached != 1 { continue; }
        let (width, height) = mode.size();
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16_777_216 { continue; }
        for &plane in &planes {
            let info = card.get_plane(plane)?;
            if !info.formats().contains(&(Fourcc::Xrgb8888 as u32)) ||
                property(card, plane, "type")?.1 != 1 ||
                property(card, plane, "CRTC_ID")?.1 != crtc_id { continue; }
            let old_fb = property(card, plane, "FB_ID")?.1;
            if old_fb == 0 { continue; }
            let expected = [
                ("SRC_X", 0), ("SRC_Y", 0), ("CRTC_X", 0), ("CRTC_Y", 0),
                ("SRC_W", u64::from(width) << 16), ("SRC_H", u64::from(height) << 16),
                ("CRTC_W", u64::from(width)), ("CRTC_H", u64::from(height)),
            ];
            let mut matches = true;
            for (name, value) in expected {
                if property(card, plane, name)?.1 != value { matches = false; break; }
            }
            if matches { return Ok((Route { connector, crtc, plane, mode }, old_fb)); }
        }
    }
    bail!("no active, unscaled, single-monitor primary plane with a restorable framebuffer")
}

fn framebuffer_request(card: &Card<'_>, plane: control::plane::Handle, fb: u64) -> Result<control::atomic::AtomicModeReq> {
    let mut request = control::atomic::AtomicModeReq::new();
    set(card, &mut request, plane, "FB_ID", fb)?;
    Ok(request)
}

fn commit_framebuffer(card: &Card<'_>, plane: control::plane::Handle, fb: u64, test: bool) -> Result<()> {
    let request = framebuffer_request(card, plane, fb)?;
    // Without ALLOW_MODESET, the kernel must reject any change that needs a modeset.
    let flags = if test { control::AtomicCommitFlags::TEST_ONLY } else { control::AtomicCommitFlags::empty() };
    card.atomic_commit(flags, request).context("atomic framebuffer swap")
}

/// Display a static rendered frame for at most one second of the event loop,
/// then restore the original primary-plane framebuffer. Never use this for a
/// production session: an involuntary seat loss can prevent restoration.
pub fn present_test(fd: &OwnedFd, mut poll_session: impl FnMut() -> Result<bool>) -> Result<()> {
    use drm::{Device as _, ClientCapability};
    use drm::buffer::PlanarBuffer;
    use control::FbCmd2Flags;
    use smithay::{backend::{
        allocator::{dmabuf::AsDmabuf, gbm::{GbmAllocator, GbmBufferFlags, GbmDevice}, Allocator, Fourcc, Modifier},
        egl::{EGLContext, EGLDisplay},
        renderer::{gles::GlesRenderer, Bind, Color32F, Frame, Renderer},
    }, utils::{DeviceFd, Rectangle, Transform}};
    use std::time::{Duration, Instant};

    let card = Card(fd);
    card.set_client_capability(ClientCapability::UniversalPlanes, true)?;
    card.set_client_capability(ClientCapability::Atomic, true)?;
    let (route, old_fb) = active_route(&card)?;
    let (width, height) = route.mode.size();

    let device = GbmDevice::new(DeviceFd::from(fd.try_clone()?))
        .map_err(|error| anyhow::anyhow!("create GBM device: {error}"))?;
    // SAFETY: the native display is an owned GBM device clone, retained by EGL.
    let display = unsafe { EGLDisplay::new(device.clone()) }
        .map_err(|error| anyhow::anyhow!("initialize EGL on GBM: {error}"))?;
    let context = EGLContext::new(&display)
        .map_err(|error| anyhow::anyhow!("create EGL context: {error}"))?;
    // SAFETY: the context is new and used only on this thread.
    let mut renderer = unsafe { GlesRenderer::new(context) }
        .map_err(|error| anyhow::anyhow!("create GLES renderer: {error}"))?;
    let mut allocator = GbmAllocator::new(device, GbmBufferFlags::SCANOUT | GbmBufferFlags::RENDERING);
    let buffer = allocator.create_buffer(u32::from(width), u32::from(height), Fourcc::Xrgb8888, &[Modifier::Linear])
        .map_err(|error| anyhow::anyhow!("allocate preview buffer: {error}"))?;
    {
        let mut dmabuf = buffer.export().map_err(|error| anyhow::anyhow!("export preview buffer: {error}"))?;
        let mut target = renderer.bind(&mut dmabuf).map_err(|error| anyhow::anyhow!("bind preview target: {error}"))?;
        let size = (i32::from(width), i32::from(height)).into();
        let mut frame = renderer.render(&mut target, size, Transform::Normal)
            .map_err(|error| anyhow::anyhow!("begin preview frame: {error}"))?;
        frame.clear(Color32F::new(0.10, 0.20, 0.35, 1.0), &[Rectangle::from_size(size)])
            .map_err(|error| anyhow::anyhow!("clear preview frame: {error}"))?;
        frame.finish().map_err(|error| anyhow::anyhow!("finish preview frame: {error}"))?
            .wait().map_err(|error| anyhow::anyhow!("preview fence: {error:?}"))?;
    }
    let flags = if PlanarBuffer::modifier(&*buffer).is_some() { FbCmd2Flags::MODIFIERS } else { FbCmd2Flags::empty() };
    let fb = card.add_planar_framebuffer(&*buffer, flags).context("register preview framebuffer")?;
    let fb_id = u64::from(u32::from(fb));
    let mut presented = false;
    let outcome = (|| -> Result<()> {
        commit_framebuffer(&card, route.plane, old_fb, true).context("preflight restoration")?;
        commit_framebuffer(&card, route.plane, fb_id, true).context("preflight preview")?;
        if !poll_session()? { bail!("preview cancelled before presentation"); }
        if property(&card, route.plane, "FB_ID")?.1 != old_fb { bail!("original framebuffer changed before presentation"); }
        commit_framebuffer(&card, route.plane, fb_id, false)?;
        presented = true;
        println!("Presented preview on {:?}: {width}x{height}", route.connector);
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if !poll_session()? { break; }
        }
        Ok(())
    })();

    // Always attempt restoration after a successful blocking commit, including
    // when event dispatch fails or the signal handler requests early exit.
    if presented {
        let restore = commit_framebuffer(&card, route.plane, old_fb, false)
            .and_then(|_| {
                if property(&card, route.plane, "FB_ID")?.1 != old_fb { bail!("restoration did not restore the original framebuffer"); }
                Ok(())
            });
        if let Err(error) = restore {
            // Do not explicitly remove a framebuffer that might still be scanned
            // out. The owning seat will release the DRM descriptor on return.
            bail!("preview restoration failed; seat release follows: {error:#}");
        }
        println!("Original primary-plane framebuffer restored.");
    }
    let cleanup = card.destroy_framebuffer(fb).context("destroy preview framebuffer");
    if let Err(error) = &cleanup { if outcome.is_err() { eprintln!("{error:#}"); } }
    outcome.and(cleanup)
}
