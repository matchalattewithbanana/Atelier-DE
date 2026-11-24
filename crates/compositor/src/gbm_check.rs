//! GPU allocation, export and optional EGL/GLES off-screen rendering validation.
use anyhow::{bail, Context, Result};
use smithay::{
    backend::{
        allocator::{
            dmabuf::AsDmabuf,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Allocator, Fourcc, Modifier,
        },
        egl::{EGLContext, EGLDisplay},
        renderer::{gles::GlesRenderer, Bind, Color32F, ExportMem, Frame, Renderer},
    },
    utils::{DeviceFd, Rectangle, Transform},
};
use std::os::fd::OwnedFd;

const SIDE: u32 = 256;
const SHADE: u8 = 64;

fn verify_pixels(bytes: &[u8], pixels: usize) -> Result<()> {
    if pixels.checked_mul(4) != Some(bytes.len()) { bail!("unexpected readback length: {}", bytes.len()); }
    for (index, rgba) in bytes.chunks_exact(4).enumerate() {
        if rgba[..3].iter().any(|&channel| channel.abs_diff(SHADE) > 2) {
            bail!("GPU readback mismatch at pixel {index}: {:?}", rgba);
        }
    }
    Ok(())
}

/// Keep the seat-owned descriptor intact. All allocations, EGL objects and
/// exported handles are scoped here and released before the seat closes it.
pub fn check(fd: &OwnedFd, render: bool) -> Result<()> {
    let duplicate = fd.try_clone().context("duplicate seat-owned DRM descriptor")?;
    let device = GbmDevice::new(DeviceFd::from(duplicate))
        .map_err(|error| anyhow::anyhow!("initialize GBM device: {error}"))?;
    if render { return render_check(device); }

    let mut allocator = GbmAllocator::new(device, GbmBufferFlags::RENDERING);
    let buffer = allocator.create_buffer(SIDE, SIDE, Fourcc::Xrgb8888, &[Modifier::Invalid])
        .map_err(|error| anyhow::anyhow!("allocate XRGB8888 GBM buffer: {error}"))?;
    let exported = buffer.export()
        .map_err(|error| anyhow::anyhow!("export GBM buffer as DMA-BUF: {error}"))?;
    drop(exported);
    drop(buffer);
    drop(allocator);
    println!("GBM allocation and DMA-BUF export succeeded (256x256 XRGB8888)");
    Ok(())
}

fn render_check(device: GbmDevice<DeviceFd>) -> Result<()> {
    // SAFETY: the native GBM device is owned by this function; the clone
    // retained by EGL keeps the device alive for all EGL resources.
    let display = unsafe { EGLDisplay::new(device.clone()) }
        .map_err(|error| anyhow::anyhow!("initialize EGL on GBM: {error}"))?;
    let modifiers: Vec<_> = display.dmabuf_render_formats().iter()
        .filter(|format| format.code == Fourcc::Xrgb8888)
        .map(|format| format.modifier)
        .collect();
    if modifiers.is_empty() { bail!("EGL does not advertise renderable XRGB8888 DMA-BUF formats"); }
    let context = EGLContext::new(&display)
        .map_err(|error| anyhow::anyhow!("create EGL context: {error}"))?;
    // SAFETY: context is new, not shared, and used only on this thread.
    let mut renderer = unsafe { GlesRenderer::new(context) }
        .map_err(|error| anyhow::anyhow!("create GLES renderer: {error}"))?;
    let mut allocator = GbmAllocator::new(device, GbmBufferFlags::RENDERING);
    let buffer = allocator.create_buffer(SIDE, SIDE, Fourcc::Xrgb8888, &modifiers)
        .map_err(|error| anyhow::anyhow!("allocate EGL-compatible GBM buffer: {error}"))?;
    let mut dmabuf = buffer.export()
        .map_err(|error| anyhow::anyhow!("export EGL-compatible buffer: {error}"))?;
    let mut target = renderer.bind(&mut dmabuf)
        .map_err(|error| anyhow::anyhow!("bind DMA-BUF as GLES render target: {error}"))?;
    let size = (SIDE as i32, SIDE as i32).into();
    let damage = Rectangle::from_size(size);
    let mut frame = renderer.render(&mut target, size, Transform::Normal)
        .map_err(|error| anyhow::anyhow!("begin GPU frame: {error}"))?;
    let shade = f32::from(SHADE) / 255.0;
    frame.clear(Color32F::new(shade, shade, shade, 1.0), &[damage])
        .map_err(|error| anyhow::anyhow!("clear GPU frame: {error}"))?;
    let sync = frame.finish().map_err(|error| anyhow::anyhow!("finish GPU frame: {error}"))?;
    sync.wait().map_err(|error| anyhow::anyhow!("GPU synchronization failed: {error:?}"))?;

    let mapping = renderer.copy_framebuffer(&target, Rectangle::from_size((SIDE as i32, SIDE as i32).into()), Fourcc::Abgr8888)
        .map_err(|error| anyhow::anyhow!("read GPU framebuffer: {error}"))?;
    let bytes = renderer.map_texture(&mapping)
        .map_err(|error| anyhow::anyhow!("map GPU framebuffer readback: {error}"))?;
    verify_pixels(bytes, SIDE as usize * SIDE as usize)?;
    println!("EGL/GLES off-screen render and full pixel verification succeeded (256x256)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_expected_pixels_and_rounding() {
        assert!(verify_pixels(&[64, 64, 64, 255, 62, 65, 66, 255], 2).is_ok());
    }

    #[test]
    fn rejects_corrupt_or_incomplete_readback() {
        assert!(verify_pixels(&[0, 64, 64, 255], 1).is_err());
        assert!(verify_pixels(&[64, 64, 64], 1).is_err());
    }
}
