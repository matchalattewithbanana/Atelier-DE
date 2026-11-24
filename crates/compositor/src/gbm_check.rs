//! Allocation and DMA-BUF export validation, independent of presentation.
use anyhow::{Context, Result};
use smithay::{
    backend::allocator::{
        dmabuf::AsDmabuf,
        gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        Allocator, Fourcc, Modifier,
    },
    utils::DeviceFd,
};
use std::os::fd::OwnedFd;

/// Keep the seat-owned descriptor intact. All allocations and exported handles
/// are scoped here and released before the caller closes its seat descriptor.
pub fn check(fd: &OwnedFd) -> Result<()> {
    let duplicate = fd.try_clone().context("duplicate seat-owned DRM descriptor")?;
    let device = GbmDevice::new(DeviceFd::from(duplicate))
        .map_err(|error| anyhow::anyhow!("initialize GBM device: {error}"))?;
    let mut allocator = GbmAllocator::new(device, GbmBufferFlags::RENDERING);
    let buffer = allocator.create_buffer(256, 256, Fourcc::Xrgb8888, &[Modifier::Invalid])
        .map_err(|error| anyhow::anyhow!("allocate 256x256 XRGB8888 GBM buffer: {error}"))?;
    let exported = buffer.export()
        .map_err(|error| anyhow::anyhow!("export GBM buffer as DMA-BUF: {error}"))?;
    drop(exported);
    drop(buffer);
    drop(allocator);
    println!("GBM allocation and DMA-BUF export succeeded (256x256 XRGB8888)");
    Ok(())
}
