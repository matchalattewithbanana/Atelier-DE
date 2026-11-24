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
