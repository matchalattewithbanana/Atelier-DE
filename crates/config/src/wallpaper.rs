//! Bounded local raster decoding for shell and Settings, never the compositor.
use anyhow::{bail,Result};
use image::{ImageFormat,ImageReader,Limits};
use std::{fs::OpenOptions,io::{Cursor,Read},os::unix::fs::OpenOptionsExt,path::Path};
const MAX_FILE:u64=32*1024*1024;
const MAX_EDGE:u32=8192;
const MAX_PIXELS:u64=24_000_000;

pub struct Raster{pub width:u32,pub height:u32,pub rgba:Vec<u8>}
fn reader(bytes:&[u8])->Result<ImageReader<Cursor<&[u8]>>>{
    let mut reader=ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    if !matches!(reader.format(),Some(ImageFormat::Png|ImageFormat::Jpeg|ImageFormat::WebP)){bail!("use a PNG, JPEG or WebP image");}
    let mut limits=Limits::default();
    limits.max_image_width=Some(MAX_EDGE);limits.max_image_height=Some(MAX_EDGE);
    limits.max_alloc=Some(192*1024*1024);
    reader.limits(limits);Ok(reader)
}
pub fn decode(path:&Path,max_edge:u32)->Result<Raster>{
    if !path.is_absolute() || !(1..=4096).contains(&max_edge){bail!("invalid local image request");}
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path)?;
    let meta=file.metadata()?;
    if !meta.is_file()||meta.len()>MAX_FILE{bail!("wallpaper must be a regular file up to 32 MiB");}
    let mut bytes=Vec::new();file.take(MAX_FILE+1).read_to_end(&mut bytes)?;
    if bytes.len() as u64>MAX_FILE{bail!("wallpaper exceeds 32 MiB");}
    let (w,h)=reader(&bytes)?.into_dimensions()?;
    if w==0||h==0||w>MAX_EDGE||h>MAX_EDGE||u64::from(w)*u64::from(h)>MAX_PIXELS{bail!("wallpaper exceeds image dimensions limit");}
    let image=reader(&bytes)?.decode()?;
    let image=if w>max_edge||h>max_edge{image.resize(max_edge,max_edge,image::imageops::FilterType::Triangle)}else{image};
    let rgba=image.into_rgba8();let (width,height)=rgba.dimensions();
    Ok(Raster{width,height,rgba:rgba.into_raw()})
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn bounded_decode_and_path_rejection(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("sample.png");
        image::RgbaImage::from_pixel(800,400,image::Rgba([30,60,90,255])).save(&path).unwrap();
        let raster=decode(&path,200).unwrap();
        assert_eq!((raster.width,raster.height,raster.rgba.len()),(200,100,80000));
        let link=dir.path().join("link.png");std::os::unix::fs::symlink(&path,&link).unwrap();
        assert!(decode(&link,200).is_err());
        std::fs::write(&path,b"not an image").unwrap();assert!(decode(&path,200).is_err());
    }
}
