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

// A protected frame is meaningful only if our primary plane covers the
// entire active display topology. The experimental backend supports one
// unscaled output, and cannot protect unrelated planes or active CRTCs.
#[cfg(feature = "native-session")]
fn same_route(a:&Route,b:&Route)->bool{
    a.connector==b.connector&&a.crtc==b.crtc&&a.plane==b.plane&&a.mode==b.mode
}
#[cfg(feature = "native-session")]
fn protected_route(card:&Card<'_>,route:&Route,expected:u64)->Result<()>{
    let (actual,current)=active_route(card)?;
    if !same_route(&actual,route)||current!=expected{
        bail!("protected display route or framebuffer ownership changed");
    }
    let resources=card.resource_handles()?;
    for &crtc in resources.crtcs(){
        if crtc!=route.crtc&&property(card,crtc,"ACTIVE")?.1!=0{
            bail!("protected session cannot cover another active CRTC");
        }
    }
    for plane in card.plane_handles()?{
        if plane!=route.plane&&(property(card,plane,"CRTC_ID")?.1!=0||property(card,plane,"FB_ID")?.1!=0){
            bail!("protected session cannot cover another bound display plane");
        }
    }
    Ok(())
}

// Hardware cursor feasibility is independent of taking ownership of a plane.
#[cfg(feature = "native-session")]
fn cursor_plane_available(kind:u64,compatible:bool,argb:bool,crtc:u64,fb:u64)->bool{
    // DRM_PLANE_TYPE_CURSOR is 2. Never replace an already bound plane.
    kind==2&&compatible&&argb&&crtc==0&&fb==0
}

#[cfg(feature = "native-session")]
fn cursor_extent(output:(u16,u16),position:(u16,u16),edge:u32)->Option<(u32,u32)>{
    if edge==0||position.0>=output.0||position.1>=output.1{return None;}
    Some((edge.min(u32::from(output.0-position.0)),edge.min(u32::from(output.1-position.1))))
}

/// Read-only plane selection and atomic TEST_ONLY validation. No cursor
/// framebuffer is ever submitted for actual presentation.
#[cfg(feature = "native-session")]
fn probe_cursor_plane(card:&Card<'_>,fd:&OwnedFd,route:&Route)->Result<()>{
    use drm::buffer::PlanarBuffer;
    use smithay::{backend::allocator::{
        gbm::{GbmAllocator,GbmBufferFlags,GbmDevice},Allocator,Fourcc,Modifier,
    },utils::DeviceFd};
    let resources=card.resource_handles()?;
    let mut candidates=Vec::new();
    for plane in card.plane_handles()?{
        let info=card.get_plane(plane)?;
        if cursor_plane_available(property(card,plane,"type")?.1,
            resources.filter_crtcs(info.possible_crtcs()).contains(&route.crtc),
            info.formats().contains(&(Fourcc::Argb8888 as u32)),
            property(card,plane,"CRTC_ID")?.1,property(card,plane,"FB_ID")?.1){
            candidates.push(plane);
        }
    }
    if candidates.is_empty(){bail!("no unused compatible ARGB8888 cursor plane");}
    // A conservative fixed-size probe, not a claim about maximum cursor size.
    const EDGE:u32=64;
    let device=GbmDevice::new(DeviceFd::from(fd.try_clone()?))
        .map_err(|error|anyhow::anyhow!("create cursor probe GBM device: {error}"))?;
    let mut allocator=GbmAllocator::new(device,GbmBufferFlags::SCANOUT|GbmBufferFlags::RENDERING);
    let buffer=allocator.create_buffer(EDGE,EDGE,Fourcc::Argb8888,&[Modifier::Linear])
        .map_err(|error|anyhow::anyhow!("allocate cursor probe buffer: {error}"))?;
    let flags=if PlanarBuffer::modifier(&*buffer).is_some(){control::FbCmd2Flags::MODIFIERS}else{control::FbCmd2Flags::empty()};
    let fb=card.add_planar_framebuffer(&*buffer,flags).context("register cursor probe framebuffer")?;
    let outcome=(||->Result<()>{
        let output=route.mode.size();
        let edge_position=(output.0.saturating_sub(1),output.1.saturating_sub(1));
        for plane in candidates{
            let test=(||->Result<()>{
                // Recheck the candidate immediately before constructing tests.
                if property(card,plane,"CRTC_ID")?.1!=0||property(card,plane,"FB_ID")?.1!=0{
                    bail!("cursor plane no longer unused");
                }
                for position in [(0,0),edge_position]{
                    let (w,h)=cursor_extent(output,position,EDGE).context("invalid cursor probe extent")?;
                    let mut request=control::atomic::AtomicModeReq::new();
                    for (name,value) in [
                        ("FB_ID",u64::from(u32::from(fb))),("CRTC_ID",u64::from(u32::from(route.crtc))),
                        ("SRC_X",0),("SRC_Y",0),("SRC_W",u64::from(w)<<16),("SRC_H",u64::from(h)<<16),
                        ("CRTC_X",u64::from(position.0)),("CRTC_Y",u64::from(position.1)),
                        ("CRTC_W",u64::from(w)),("CRTC_H",u64::from(h)),
                    ]{set(card,&mut request,plane,name,value)?;}
                    // No ALLOW_MODESET, no event, and no non-test commit.
                    card.atomic_commit(control::AtomicCommitFlags::TEST_ONLY,request)
                        .context("cursor plane TEST_ONLY rejected")?;
                }
                Ok(())
            })();
            match test{
                Ok(())=>{println!("Atelier hardware cursor preflight accepted {plane:?} (64x64 ARGB8888, origin and clipped edge); software cursor retained");return Ok(());}
                Err(error)=>tracing::debug!(?plane,?error,"cursor probe candidate rejected"),
            }
        }
        bail!("all unused cursor candidates rejected the bounded TEST_ONLY probe")
    })();
    let cleanup=card.destroy_framebuffer(fb).context("destroy cursor probe framebuffer");
    if let Err(error)=&cleanup{if outcome.is_err(){eprintln!("{error:#}");}}
    outcome.and(cleanup)
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


#[cfg(feature = "native-session")]
fn flip_completed(card: &Card<'_>, crtc: control::crtc::Handle) -> Result<bool> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    let mut fds = [PollFd::new(card, PollFlags::IN)];
    let immediately = Timespec { tv_sec: 0, tv_nsec: 0 };
    poll(&mut fds, Some(&immediately)).context("poll DRM event descriptor")?;
    let flags = fds[0].revents();
    if flags.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL) { bail!("DRM event descriptor failed"); }
    if !flags.contains(PollFlags::IN) { return Ok(false); }
    let events = card.receive_events().context("read DRM events")?;
    Ok(events.into_iter().any(|event| matches!(event, control::Event::PageFlip(flip) if flip.crtc == crtc)))
}

// Recovery must not overwrite a framebuffer installed by another owner.
// When ownership or route identity is uncertain, leave explicit DRM cleanup
// to descriptor/seat release rather than removing a potentially active FB.
#[cfg(feature = "native-session")]
#[derive(Debug,PartialEq,Eq)]
enum Recovery {
    Restore,
    LeaveCurrent,
    Defer,
}

#[cfg(feature = "native-session")]
fn recovery_decision(seat_active:bool,pending:bool,route_matches:bool,current:u64,owned:&[u64])->Recovery {
    if !seat_active||pending||!route_matches{return Recovery::Defer;}
    if owned.contains(&current){Recovery::Restore}else{Recovery::LeaveCurrent}
}

#[cfg(feature = "native-session")]
fn recover_scanout(card:&Card<'_>,route:&Route,original:u64,owned:&[u64],seat_active:bool,pending:bool)->Result<()> {
    if !seat_active||pending{
        bail!("native scanout cleanup deferred: seat inactive or page flip outstanding");
    }
    // Revalidate the same route including primary-plane geometry, connector,
    // mode and mirroring before writing any display property.
    let (current_route,current)=active_route(card).context("revalidate native route before cleanup")?;
    let matches=same_route(&current_route,route);
    match recovery_decision(seat_active,pending,matches,current,owned){
        Recovery::Defer=>bail!("native route changed; deferring framebuffer cleanup"),
        Recovery::Restore=>{
            commit_framebuffer(card,route.plane,original,false).context("restore original native framebuffer")?;
            if property(card,route.plane,"FB_ID")?.1!=original{bail!("restored framebuffer differs from original");}
        }
        Recovery::LeaveCurrent=>{
            if current!=original{eprintln!("Primary-plane framebuffer changed outside Atelier; leaving external state intact.");}
        }
    }
    // Check all planes, not merely the last slot tracked by FrameQueue.
    // An externally changed route may still reference a temporary buffer.
    for plane in card.plane_handles()?{
        if owned.contains(&property(card,plane,"FB_ID")?.1){
            bail!("temporary framebuffer is still referenced by a plane; deferring cleanup");
        }
    }
    Ok(())
}

// A protected recovery never resumes normal dispatch. The old renderer,
// queue, input device state and frame acknowledgments are untrusted.
#[cfg(feature = "native-session")]
#[derive(Default)]
struct BlackRecovery{
    attempted:Option<u64>,
    attempts:u8,
}
#[cfg(feature = "native-session")]
impl BlackRecovery{
    /// Bound the number of new kernel allocations retained until fd release.
    /// A new attempt requires a fresh active epoch, not repeated polling.
    fn begin(&mut self,epoch:Option<u64>)->bool{
        let Some(epoch)=epoch else{return false;};
        if self.attempted==Some(epoch)||self.attempts>=8{return false;}
        self.attempted=Some(epoch);self.attempts+=1;true
    }
}

#[cfg(feature = "native-session")]
fn present_recovery_black(card:&Card<'_>,route:&Route,owned:&[u64])->Result<u64>{
    use smithay::backend::allocator::Fourcc;
    let (actual,current)=active_route(card)?;
    if !same_route(&actual,route)||!owned.contains(&current){
        bail!("recovery refuses a changed route or externally owned framebuffer");
    }
    protected_route(card,route,current)?;
    let (w,h)=route.mode.size();
    // Fresh CPU-initialized scanout; do not reuse the old GBM renderer,
    // its fences, damage history or potentially outstanding slots.
    let mut buffer=card.create_dumb_buffer((u32::from(w),u32::from(h)),Fourcc::Xrgb8888,32)
        .context("allocate emergency black buffer")?;
    // No explicit removal of an ambiguously submitted FB. Kernel handles
    // remain owned by the seat descriptor until its release.
    {
        let mut mapping=card.map_dumb_buffer(&mut buffer).context("map emergency black buffer")?;
        mapping.as_mut().fill(0);
    }
    let fb=card.add_framebuffer(&buffer,24,32).context("register emergency black framebuffer")?;
    let id=u64::from(u32::from(fb));
    commit_framebuffer(card,route.plane,id,true).context("test emergency black framebuffer")?;
    protected_route(card,route,current)?;
    // Blocking commit, no page-flip event from this recovery request.
    // An old pending flip may cause failure; never force a modeset.
    commit_framebuffer(card,route.plane,id,false).context("present emergency black framebuffer")?;
    protected_route(card,route,id)?;
    Ok(id)
}

#[cfg(feature = "native-session")]
fn quarantine_protected(
    card:&Card<'_>,route:&Route,mut owned:Vec<u64>,
    session:&smithay::backend::session::libseat::LibSeatSession,
    continuity:&crate::seat::SeatContinuity,
    event_loop:&mut smithay::reexports::calloop::EventLoop<'_,()>,
    running:&std::sync::atomic::AtomicBool,
)->Result<()>{
    use smithay::backend::session::Session;
    use std::{sync::atomic::Ordering,time::Duration};
    let mut recovery=BlackRecovery::default();
    tracing::error!("protected session quarantined; no Wayland dispatch or input; explicit termination required");
    while running.load(Ordering::SeqCst){
        event_loop.dispatch(Duration::from_millis(100),&mut ())
            .map_err(|e|anyhow::anyhow!("dispatch quarantined seat events: {e}"))?;
        if !running.load(Ordering::SeqCst){break;}
        let epoch=continuity.recovery_epoch(session.is_active());
        if !recovery.begin(epoch){continue;}
        match present_recovery_black(card,route,&owned){
            Ok(id)=>{
                owned.push(id);
                tracing::warn!("emergency black framebuffer verified; session remains quarantined");
            }
            Err(error)=>tracing::error!(?error,"emergency black presentation failed; session remains quarantined"),
        }
    }
    bail!("protected session terminated from quarantine; seat release remains unverified")
}

// Experimental continuous native session. One outstanding nonblocking page flip keeps
// the displayed and pending buffers out of the renderer.
#[cfg(feature = "native-session")]
pub fn run_native(
    fd: &OwnedFd,
    session: &smithay::backend::session::libseat::LibSeatSession,
    event_loop: &mut smithay::reexports::calloop::EventLoop<'_, ()>,
    socket: &str,
    config: &atelier_config::Config,
    continuity:std::rc::Rc<crate::seat::SeatContinuity>,
) -> Result<()> {
    use crate::{state::{Atelier, ClientState}, input_router::handle_input, frame_queue::{FrameQueue,InputDamage}};
    use drm::{Device as _, ClientCapability};
    use drm::buffer::PlanarBuffer;
    use control::FbCmd2Flags;
    use smithay::{
        backend::{
            allocator::{dmabuf::AsDmabuf, gbm::{GbmAllocator, GbmBufferFlags, GbmDevice}, Allocator, Fourcc, Modifier},
            egl::{EGLContext, EGLDisplay},
            renderer::{gles::GlesRenderer, utils::draw_render_elements, Bind, Color32F, Frame, Renderer},
            session::Session,
        },
        reexports::wayland_server::{Display, ListeningSocket},
        utils::{DeviceFd, Rectangle, Transform},
    };
    use std::{cell::{Cell, RefCell}, collections::HashSet, rc::Rc, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

    let card = Card(fd);
    card.set_client_capability(ClientCapability::UniversalPlanes, true)?;
    card.set_client_capability(ClientCapability::Atomic, true)?;
    continuity.ensure(session.is_active())?;
    let (route, original) = active_route(&card)?;
    if std::env::var("ATELIER_CURSOR_PROBE").as_deref()==Ok("1"){
        if let Err(error)=probe_cursor_plane(&card,fd,&route){
            tracing::warn!(?error,"hardware cursor preflight unavailable; retaining software cursor");
        }
    }
    let (width, height) = route.mode.size();
    let size = (i32::from(width), i32::from(height)).into();
    let output_damage = Rectangle::from_size(size);

    let mut server = Display::<Atelier>::new()?;
    let mut compositor = Atelier::new_for_output(&server.handle(), config.layout, "ATELIER-DRM-1", "Native DRM display")?;
    compositor.enable_cursor_shapes();
    let refresh = route.mode.vrefresh().saturating_mul(1000).max(1000).min(i32::MAX as u32) as i32;
    compositor.resize_with_refresh(i32::from(width), i32::from(height), refresh);
    let state = Rc::new(RefCell::new(compositor));
    let listener = ListeningSocket::bind(socket).context("bind native Wayland socket")?;
    let mut workspace_control = crate::workspace_control::WorkspaceControl::bind(socket)?;

    let device = GbmDevice::new(DeviceFd::from(fd.try_clone()?))
        .map_err(|error| anyhow::anyhow!("create native GBM device: {error}"))?;
    // SAFETY: EGL retains an owned native device and all context use is on this thread.
    let egl = unsafe { EGLDisplay::new(device.clone()) }
        .map_err(|error| anyhow::anyhow!("initialize native EGL: {error}"))?;
    let context = EGLContext::new(&egl).map_err(|error| anyhow::anyhow!("create native context: {error}"))?;
    // SAFETY: newly created context is never shared with another thread.
    let mut renderer = unsafe { GlesRenderer::new(context) }
        .map_err(|error| anyhow::anyhow!("initialize native GLES: {error}"))?;
    let mut allocator = GbmAllocator::new(device, GbmBufferFlags::SCANOUT | GbmBufferFlags::RENDERING);
    let mut slots = Vec::new();
    let mut queue = FrameQueue::default();
    let mut client_damage=crate::client_damage::ClientDamage::new();
    let mut shaders=crate::scene::SceneShaders::default();
    let mut cursor_theme=crate::native_cursor::CursorTheme::load(&mut renderer);

    let running = Arc::new(AtomicBool::new(true));
    let signal = Arc::clone(&running);
    ctrlc::set_handler(move || signal.store(false, Ordering::SeqCst))?;
    let outcome = (|| -> Result<()> {
        for _ in 0..2 {
            let buffer = allocator.create_buffer(u32::from(width), u32::from(height), Fourcc::Xrgb8888, &[Modifier::Linear])
                .map_err(|error| anyhow::anyhow!("allocate native scanout buffer: {error}"))?;
            // Keep the same exported object for the lifetime of this slot.
            // Validate EGL import before registering the DRM framebuffer.
            let mut dmabuf = buffer.export().map_err(|error| anyhow::anyhow!("export native scanout buffer: {error}"))?;
            {
                let _target = renderer.bind(&mut dmabuf).map_err(|error| anyhow::anyhow!("preflight native render target: {error}"))?;
            }
            let flags = if PlanarBuffer::modifier(&*buffer).is_some() { FbCmd2Flags::MODIFIERS } else { FbCmd2Flags::empty() };
            let fb = card.add_planar_framebuffer(&*buffer, flags).context("register native framebuffer")?;
            // Retain both the GBM allocation and the exported DMA-BUF; the
            // framebuffer handle remains registered until guarded cleanup.
            slots.push((buffer, fb, dmabuf));
            commit_framebuffer(&card, route.plane, u64::from(u32::from(fb)), true)
                .context("preflight native framebuffer")?;
        }
        commit_framebuffer(&card, route.plane, original, true).context("preflight original framebuffer")?;

        let input_state = Rc::clone(&state);
        let input_dirty = Rc::new(Cell::new(InputDamage::None));
        let event_dirty = Rc::clone(&input_dirty);
        let input_session = session.clone();
        let input_continuity=Rc::clone(&continuity);
        let mut intercepted = HashSet::new();
        let input_token = event_loop.handle().insert_source(crate::native_input::backend(session)?, move |event, _, _| {
            if input_continuity.usable(input_session.is_active()) {
                let mut state=input_state.borrow_mut();
                use smithay::backend::input::InputEvent;
                let motion=matches!(&event,InputEvent::PointerMotion{..}|InputEvent::PointerMotionAbsolute{..});
                let ungrabbed=state.seat.get_pointer().is_some_and(|p|!p.is_grabbed());
                handle_input(&mut state,event,&mut intercepted);
                let still_ungrabbed=state.seat.get_pointer().is_some_and(|p|!p.is_grabbed());
                let change=if motion&&ungrabbed&&still_ungrabbed{InputDamage::Cursor}else{InputDamage::Scene};
                event_dirty.set(event_dirty.get().max(change));
            }
        }).map_err(|error| anyhow::anyhow!("register native input: {error}"))?;

        let result = (|| -> Result<()> {
            let started = Instant::now();
            let color = config.theme.background();
            let mut metrics = crate::frame_metrics::FrameMetrics::new("native");
            let mut pending_since: Option<Instant> = None;
            let mut pending_protected_frame=None;
            let mut last_route_check = Instant::now();
            println!("Atelier native ready: WAYLAND_DISPLAY={socket}");
            while running.load(Ordering::SeqCst) || queue.pending().is_some() {
                metrics.report_due();
                event_loop.dispatch(Duration::from_millis(8), &mut ())
                    .map_err(|error| anyhow::anyhow!("dispatch native events: {error}"))?;
                continuity.ensure(session.is_active())?;

                // A completed flip, not submission, releases the old buffer and timestamps clients.
                if let Some(slot)=queue.pending(){
                    if flip_completed(&card, route.crtc)? {
                    continuity.ensure(session.is_active())?;
                    // Never acknowledge a protected flip merely because an
                    // event for this CRTC arrived. Verify the presented FB
                    // and every active plane before notifying the lock owner.
                    if !state.borrow().desktop_visible(){
                        protected_route(&card,&route,u64::from(u32::from(slots[slot].1)))?;
                    }
                    queue.completed();
                    metrics.flip_completed(pending_since.take());
                    let mut state = state.borrow_mut();
                    if let Some(frame)=pending_protected_frame.take(){state.protected_frame_presented(frame);}
                    state.send_frames(started.elapsed());
                    state.send_cursor_frames(started.elapsed());
                    }
                }
                if pending_since.is_some_and(|since| since.elapsed() > Duration::from_secs(1)) {
                    bail!("timed out waiting for native page flip");
                }
                if last_route_check.elapsed() >= Duration::from_millis(500) {
                    let connector = card.get_connector(route.connector, false)?;
                    if connector.state() != connector::State::Connected ||
                        property(&card, route.connector, "CRTC_ID")?.1 != u64::from(u32::from(route.crtc)) ||
                        card.get_crtc(route.crtc)?.mode() != Some(route.mode) {
                        bail!("native monitor route changed; restart after reconnecting");
                    }
                    last_route_check = Instant::now();
                    // Periodic conservative baseline, without forcing idle flips.
                    queue.forget_damage_history();
                    client_damage.reset();
                }
                if !running.load(Ordering::SeqCst) {
                    server.flush_clients()?;
                    continue;
                }
                while let Some(stream) = listener.accept()? {
                    if let Err(error) = server.handle().insert_client(stream, Arc::new(ClientState::default())) {
                        tracing::warn!(?error, "native client connection failed");
                    }
                    queue.invalidate();
                }
                match input_dirty.replace(InputDamage::None){
                    InputDamage::None=>{},
                    InputDamage::Cursor=>queue.invalidate_cursor(),
                    InputDamage::Scene=>queue.invalidate(),
                }
                {
                    let mut state = state.borrow_mut();
                    workspace_control.poll(&mut state, started.elapsed().as_millis() as u32)?;
                    // Conservative scene invalidation: a client request may alter content,
                    // stacking, regions or frame callbacks. Idle sessions do not submit.
                    if server.dispatch_clients(&mut *state)? > 0 { queue.invalidate(); }
                    state.sync_input_boundary(started.elapsed().as_millis() as u32);
                    state.popups.cleanup();
                    state.space.refresh();
                    if state.take_scene_dirty() { queue.invalidate(); }
                }
                server.flush_clients()?;
                if cursor_theme.animation_due(started.elapsed()){queue.invalidate_cursor();}
                let Some(next) = queue.next() else { continue; };
                let mut timing = metrics.begin_frame();
                let cursor_bounds;
                let snapshot;
                let custom_cursor;
                let protected_frame;
                {
                    let state = state.borrow();
                    let desktop_visible=state.desktop_visible();
                    protected_frame=state.protected_frame();
                    let mut target = renderer.bind(&mut slots[next].2).map_err(|error| anyhow::anyhow!("bind native framebuffer: {error}"))?;
                    let elements = crate::scene::elements(&state,&mut renderer,&mut shaders,color)?;
                    let cursor=crate::native_cursor::NativeCursor::prepare(&state,&mut renderer,size,&mut cursor_theme,started.elapsed());
                    let bounds=cursor.bounds();
                    custom_cursor=!desktop_visible||bounds.is_none();
                    cursor_bounds=bounds.unwrap_or(output_damage);
                    let damage=if !custom_cursor{
                        let fallback=queue.damage(next,output_damage,cursor_bounds);
                        snapshot=client_damage.capture(&elements,cursor_bounds);
                        client_damage.damage(next,output_damage,fallback,snapshot.as_ref(),&elements)
                    }else{
                        // Uncertain cursor geometry cannot establish a retained baseline.
                        queue.forget_damage_history();
                        client_damage.reset();
                        snapshot=None;
                        output_damage
                    };
                    timing.repaint(damage,output_damage);
                    timing.scene_ready();
                    let mut frame = renderer.render(&mut target, size, Transform::Normal)
                        .map_err(|error| anyhow::anyhow!("begin native frame: {error}"))?;
                    let clear=if desktop_visible{Color32F::new(color[0],color[1],color[2],color[3])}else{Color32F::new(0.0,0.0,0.0,1.0)};
                    frame.clear(clear, &[damage])?;
                    draw_render_elements(&mut frame, 1.0, &elements, &[damage])?;
                    cursor.draw(&mut frame,damage)?;
                    frame.finish()?.wait().map_err(|error| anyhow::anyhow!("native frame fence: {error:?}"))?;
                }
                timing.render_ready();
                continuity.ensure(session.is_active())?;
                let expected = queue.displayed().map(|slot| u64::from(u32::from(slots[slot].1))).unwrap_or(original);
                if !state.borrow().desktop_visible(){
                    protected_route(&card,&route,expected)?;
                }else if property(&card, route.plane, "FB_ID")?.1 != expected {
                    bail!("native framebuffer changed outside Atelier");
                }
                let request = framebuffer_request(&card, route.plane, u64::from(u32::from(slots[next].1)))?;
                card.atomic_commit(control::AtomicCommitFlags::NONBLOCK | control::AtomicCommitFlags::PAGE_FLIP_EVENT, request)
                    .context("queue native page flip")?;
                queue.submitted(next,cursor_bounds);
                if custom_cursor{queue.forget_damage_history();}
                client_damage.submitted(next,snapshot);
                pending_protected_frame=protected_frame;
                pending_since = Some(Instant::now());
                metrics.submitted(timing);
            }
            Ok(())
        })();
        event_loop.handle().remove(input_token);
        result
    })();

    // Never intentionally restore pre-session content after an accepted
    // lock. Descriptor/seat release is NOT a verified secure recovery path.
    if !state.borrow().desktop_visible(){
        if let Err(cause)=&outcome{eprintln!("Native protected session error: {cause:#}");}
        let owned=slots.iter().map(|(_,fb,_)|u64::from(u32::from(*fb))).collect();
        // Input source was removed above. No client dispatch or old flip
        // completion handling in quarantine; no automatic unlock/resume.
        return quarantine_protected(&card,&route,owned,session,&continuity,event_loop,&running);
    }
    // Even if the seat is currently active again, an observed pause
    // invalidates all old route/FB assumptions. Never restore or explicitly
    // destroy buffers using the previous ownership epoch.
    if !continuity.usable(session.is_active()){
        if let Err(cause)=&outcome{eprintln!("Native session error: {cause:#}");}
        bail!("seat continuity lost; refusing stale scanout cleanup; seat release follows");
    }
    let owned:Vec<u64>=slots.iter().map(|(_,fb,_)|u64::from(u32::from(*fb))).collect();
    if !owned.is_empty(){
        if let Err(error)=recover_scanout(&card,&route,original,&owned,continuity.usable(session.is_active()),queue.pending().is_some()){
            if let Err(cause)=&outcome{eprintln!("Native session error: {cause:#}");}
            return Err(error.context("native recovery incomplete; seat release follows"));
        }
        if !continuity.usable(session.is_active()){
            if let Err(cause)=&outcome{eprintln!("Native session error: {cause:#}");}
            bail!("native seat continuity lost before framebuffer cleanup; seat release follows");
        }
    }
    let mut cleanup = Ok(());
    for (_, framebuffer, _) in &slots {
        if let Err(error) = card.destroy_framebuffer(*framebuffer) {
            eprintln!("Unable to destroy temporary framebuffer: {error}");
            cleanup = Err(anyhow::anyhow!("native framebuffer cleanup failed: {error}"));
        }
    }
    outcome.and(cleanup)
}

#[cfg(all(test,feature = "native-session"))]
mod recovery_tests {
    use super::*;
    #[test]
    fn cursor_probe_never_selects_bound_or_incompatible_planes(){
        assert!(cursor_plane_available(2,true,true,0,0));
        assert!(!cursor_plane_available(1,true,true,0,0));
        assert!(!cursor_plane_available(2,false,true,0,0));
        assert!(!cursor_plane_available(2,true,false,0,0));
        assert!(!cursor_plane_available(2,true,true,3,0));
        assert!(!cursor_plane_available(2,true,true,0,4));
        assert_eq!(cursor_extent((1920,1080),(0,0),64),Some((64,64)));
        assert_eq!(cursor_extent((1920,1080),(1919,1079),64),Some((1,1)));
        assert_eq!(cursor_extent((1920,1080),(1900,1060),64),Some((20,20)));
        assert_eq!(cursor_extent((1920,1080),(1920,0),64),None);
        assert_eq!(cursor_extent((0,0),(0,0),64),None);
    }
    #[test]
    fn black_recovery_attempts_once_per_active_epoch_and_is_bounded(){
        let mut r=BlackRecovery::default();
        assert!(!r.begin(None));
        assert!(r.begin(Some(0)));assert!(!r.begin(Some(0)));
        assert!(!r.begin(None));assert!(r.begin(Some(1)));
        for epoch in 2..8{assert!(r.begin(Some(epoch)));}
        assert!(!r.begin(Some(8)));assert!(!r.begin(None));
    }
    #[test]
    fn recovery_restores_only_owned_framebuffers_on_verified_route(){
        let owned=[11,12];
        assert_eq!(recovery_decision(true,false,true,11,&owned),Recovery::Restore);
        assert_eq!(recovery_decision(true,false,true,12,&owned),Recovery::Restore);
        assert_eq!(recovery_decision(true,false,true,9,&owned),Recovery::LeaveCurrent);
        assert_eq!(recovery_decision(true,false,true,11,&[]),Recovery::LeaveCurrent);
        for current in [9,11,12]{
            assert_eq!(recovery_decision(false,false,true,current,&owned),Recovery::Defer);
            assert_eq!(recovery_decision(true,true,true,current,&owned),Recovery::Defer);
            assert_eq!(recovery_decision(true,false,false,current,&owned),Recovery::Defer);
        }
    }
}
