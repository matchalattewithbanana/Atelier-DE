//! Bounded XCursor image decoding shared by native rendering and local previews.
//! Only image chunks are decoded; no execution or theme configuration changes.
use anyhow::{bail,Context,Result};
use std::{fs::OpenOptions,io::Read,os::unix::fs::OpenOptionsExt,path::Path};

const MAX_FILE:usize=8*1024*1024;
const MAX_FRAMES:usize=32;
const MAX_SHAPE_BYTES:usize=512*1024;
const IMAGE_TYPE:u32=0xfffd0002;

#[derive(Debug)]
pub struct CursorFrame{
    pub width:u32,pub height:u32,pub xhot:u32,pub yhot:u32,pub delay_ms:u32,
    /// Premultiplied RGBA, not straight alpha.
    pub rgba:Vec<u8>,
}
#[derive(Clone,Copy)]
struct Chunk{
    size:u32,width:u32,height:u32,xhot:u32,yhot:u32,delay_ms:u32,start:usize,bytes:usize,
}

fn word(data:&[u8],at:usize)->Result<u32>{
    Ok(u32::from_le_bytes(data.get(at..at.checked_add(4).context("cursor offset overflow")?)
        .context("truncated cursor word")?.try_into()?))
}

pub fn decode(data:&[u8],size:u32)->Result<Vec<CursorFrame>>{
    if data.len()>MAX_FILE||data.get(..4)!=Some(b"Xcur"){bail!("invalid or oversized XCursor file");}
    let header=word(data,4)? as usize;
    let count=word(data,12)? as usize;
    if header<16||count>256||header.checked_add(count*12).is_none_or(|end|end>data.len()){
        bail!("invalid XCursor table");
    }
    let table_end=header+count*12;
    let mut chunks=Vec::new();let mut total=0usize;
    for index in 0..count{
        let at=header+index*12;
        if word(data,at)?!=IMAGE_TYPE{continue;}
        let nominal=word(data,at+4)?;
        let pos=word(data,at+8)? as usize;
        if pos<table_end||pos.checked_add(36).is_none_or(|end|end>data.len()){bail!("invalid image offset");}
        let length=word(data,pos)? as usize;
        if length<36||word(data,pos+4)?!=IMAGE_TYPE||word(data,pos+8)?!=nominal||word(data,pos+12)?!=1{
            bail!("invalid XCursor image header");
        }
        let width=word(data,pos+16)?;let height=word(data,pos+20)?;
        let xhot=word(data,pos+24)?;let yhot=word(data,pos+28)?;
        if !(1..=256).contains(&width)||!(1..=256).contains(&height)||xhot>=width||yhot>=height{
            bail!("invalid XCursor dimensions or hotspot");
        }
        let bytes=width as usize*height as usize*4;
        let start=pos.checked_add(length).context("cursor image offset overflow")?;
        if start.checked_add(bytes).is_none_or(|end|end>data.len()){bail!("truncated XCursor pixels");}
        total=total.checked_add(bytes).context("cursor storage overflow")?;
        if total>MAX_FILE{bail!("XCursor decoded storage exceeds limit");}
        chunks.push(Chunk{size:nominal,width,height,xhot,yhot,delay_ms:word(data,pos+32)?.clamp(16,60_000),start,bytes});
    }
    let first=*chunks.iter().min_by_key(|c|c.size.abs_diff(size)).context("no usable XCursor image")?;
    let mut selected:Vec<_>=chunks.into_iter().filter(|c|c.size==first.size).collect();
    if selected.len()>MAX_FRAMES||selected.iter().map(|c|c.bytes).sum::<usize>()>MAX_SHAPE_BYTES{
        selected=vec![first];
    }
    Ok(selected.into_iter().map(|c|{
        // XCursor words are little-endian ARGB, hence BGRA bytes on disk.
        let mut rgba=data[c.start..c.start+c.bytes].to_vec();
        for p in rgba.chunks_exact_mut(4){p.swap(0,2);}
        CursorFrame{width:c.width,height:c.height,xhot:c.xhot,yhot:c.yhot,delay_ms:c.delay_ms,rgba}
    }).collect())
}

/// Follow ordinary cursor aliases, but refuse nonregular sources and bound reads.
/// O_NONBLOCK prevents an alias to a FIFO from stalling a preview.
pub fn load(path:&Path,size:u32)->Result<Vec<CursorFrame>>{
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
    let metadata=file.metadata()?;
    if !metadata.is_file()||metadata.len()>MAX_FILE as u64{bail!("cursor image is not a bounded regular file");}
    let mut data=Vec::new();file.take(MAX_FILE as u64+1).read_to_end(&mut data)?;
    decode(&data,size)
}

#[cfg(test)]
mod tests{
    use super::*;
    fn fixture()->Vec<u8>{
        // Two 1x1 image chunks, differing nominal sizes and BGRA values.
        let mut words=vec![0x72756358u32,16,0x00010000,2,IMAGE_TYPE,24,40,IMAGE_TYPE,48,80];
        for (size,pixel) in [(24,0x803c2814),(48,0xff112233)]{
            words.extend([36,IMAGE_TYPE,size,1,1,1,0,0,0,pixel]);
        }
        words.into_iter().flat_map(u32::to_le_bytes).collect()
    }
    #[test]
    fn decode_nearest_size_and_premultiplied_channels(){
        let data=fixture();
        let frames=decode(&data,24).unwrap();
        assert_eq!(frames.len(),1);assert_eq!(frames[0].rgba,[60,40,20,128]);
        assert_eq!(frames[0].delay_ms,16);
        assert_eq!(decode(&data,48).unwrap()[0].rgba,[17,34,51,255]);
    }
    #[test]
    fn rejects_truncated_and_inconsistent_chunks(){
        let mut data=fixture();data.pop();assert!(decode(&data,24).is_err());
        let mut data=fixture();data[56..60].copy_from_slice(&257u32.to_le_bytes());assert!(decode(&data,24).is_err());
        let mut data=fixture();data[44..48].copy_from_slice(&0u32.to_le_bytes());assert!(decode(&data,24).is_err());
        assert!(decode(b"Xcur",24).is_err());
    }
}
