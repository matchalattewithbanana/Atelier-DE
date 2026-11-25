//! Bounded local raster decoding for shell and Settings, never the compositor.
use anyhow::{bail,Result};
use image::{ImageFormat,ImageReader,Limits};
use std::{fs::{self,OpenOptions},io::{Cursor,Read},os::unix::fs::OpenOptionsExt,path::Path};
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
/// Optional static wallpaper blur, not compositor backdrop blur.
pub fn decode_background(path:&Path,strength:u32)->Result<Raster>{
    if strength>24{bail!("wallpaper blur must be 0–24");}
    if strength==0{return decode(path,4096);}
    // Limit working pixels and sigma for the CPU filter. Blur only on image
    // or strength changes, never every compositor frame.
    let r=decode(path,1536)?;
    let image=image::RgbaImage::from_raw(r.width,r.height,r.rgba).ok_or_else(||anyhow::anyhow!("invalid raster dimensions"))?;
    let blurred=image::imageops::blur(&image,strength as f32);
    Ok(Raster{width:r.width,height:r.height,rgba:blurred.into_raw()})
}
/// Non-recursive local catalog. Never follow a final symlink or decode
/// arbitrary directory contents. A candidate still needs decode validation.
pub fn gallery(folder:&Path)->Result<Vec<String>>{
    if !folder.is_absolute()||!fs::symlink_metadata(folder)?.is_dir(){bail!("choose an absolute real directory");}
    let mut images=Vec::new();
    for (index,entry) in fs::read_dir(folder)?.enumerate(){
        if index>=4096{bail!("folder contains more than 4096 entries; choose a smaller folder");}
        let entry=entry?;let path=entry.path();
        let Some(ext)=path.extension().and_then(|e|e.to_str())else{continue;};
        if !matches!(ext.to_ascii_lowercase().as_str(),"png"|"jpg"|"jpeg"|"webp"){continue;}
        let meta=fs::symlink_metadata(&path)?;
        if !meta.is_file()||meta.len()>MAX_FILE{continue;}
        let Some(path)=path.to_str()else{continue;};
        if path.len()>1024||path.chars().any(char::is_control){continue;}
        images.push(path.to_owned());
        if images.len()>1000{bail!("folder contains more than 1000 supported images; choose a smaller folder");}
    }
    images.sort();Ok(images)
}
/// Cheap per-file checks for playlist saves; actual image decoding is
/// bounded independently when previewed or displayed.
pub fn validate_candidate(path:&Path)->Result<()>{
    if !path.is_absolute(){bail!("image path must be absolute");}
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path)?;
    let meta=file.metadata()?;
    if !meta.is_file()||meta.len()>MAX_FILE{bail!("image must be a regular file up to 32 MiB");}
    Ok(())
}
/// Sample a bounded thumbnail into coarse RGB buckets. Ignore transparent
/// pixels; favor chromatic, mid-tone colors without letting a single pixel win.
pub fn palette_seed(path:&Path)->Result<[u8;3]>{
    let raster=decode(path,96)?;
    let mut bins=std::collections::BTreeMap::<[u8;3],(u64,[u64;3])>::new();
    for px in raster.rgba.chunks_exact(4){
        if px[3]<192{continue;}
        let entry=bins.entry([px[0]/32,px[1]/32,px[2]/32]).or_default();
        entry.0+=1;for i in 0..3{entry.1[i]+=u64::from(px[i]);}
    }
    let (_, (count,sum))=bins.into_iter().max_by_key(|(key,(count,_))|{
        let saturation=u64::from(key.iter().max().unwrap()-key.iter().min().unwrap());
        count*(8+saturation*2)
    }).ok_or_else(||anyhow::anyhow!("image contains no sufficiently opaque pixels"))?;
    Ok(std::array::from_fn(|i|(sum[i]/count) as u8))
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn wallpaper_blur_softens_edges(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("edge.png");
        image::RgbaImage::from_fn(64,32,|x,_|if x<32{image::Rgba([0,0,0,255])}else{image::Rgba([255,255,255,255])}).save(&path).unwrap();
        let plain=decode_background(&path,0).unwrap();let soft=decode_background(&path,4).unwrap();
        assert_eq!(plain.rgba[(16*64+31)*4],0);
        assert!(soft.rgba[(16*64+31)*4]>0&&soft.rgba[(16*64+31)*4]<255);
        assert!(decode_background(&path,25).is_err());
    }
    #[test]
    fn gallery_is_sorted_and_excludes_symlinks_and_subdirectories(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path();
        fs::write(path.join("b.JPG"),b"candidate").unwrap();fs::write(path.join("a.png"),b"candidate").unwrap();
        fs::write(path.join("readme.txt"),b"skip").unwrap();
        fs::create_dir(path.join("nested.png")).unwrap();
        std::os::unix::fs::symlink(path.join("a.png"),path.join("link.png")).unwrap();
        let found=gallery(path).unwrap();
        assert_eq!(found,vec![path.join("a.png").to_str().unwrap().to_owned(),path.join("b.JPG").to_str().unwrap().to_owned()]);
        assert!(validate_candidate(&path.join("link.png")).is_err());
    }
    #[test]
    fn bounded_decode_and_path_rejection(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("sample.png");
        image::RgbaImage::from_pixel(800,400,image::Rgba([30,60,90,255])).save(&path).unwrap();
        assert_eq!(palette_seed(&path).unwrap(),[30,60,90]);
        let raster=decode(&path,200).unwrap();
        assert_eq!((raster.width,raster.height,raster.rgba.len()),(200,100,80000));
        let link=dir.path().join("link.png");std::os::unix::fs::symlink(&path,&link).unwrap();
        assert!(decode(&link,200).is_err());
        std::fs::write(&path,b"not an image").unwrap();assert!(decode(&path,200).is_err());
    }
}
