//! Static, opaque recovery prompt pixels. No desktop capture, font lookup,
//! user-provided strings, password characters or exact password length.
use anyhow::{bail,Result};
use font8x8::{UnicodeFonts,BASIC_FONTS};
use std::io::Write;

#[derive(Clone,Copy,PartialEq,Eq)]
pub(super) enum Status{Ready,Entered,Checking,Retry,Unavailable}

const BACKGROUND:u32=0x101217;
const PANEL:u32=0x20242d;
const TEXT:u32=0xf1f3f8;
const MUTED:u32=0xb9c0ce;
const ACCENT:u32=0x9aafff;
const WARNING:u32=0xf5bd86;
struct Canvas{width:usize,height:usize,pixels:Vec<u8>}
impl Canvas{
    fn new(width:usize,height:usize)->Result<Self>{
        let bytes=width.checked_mul(height).and_then(|n|n.checked_mul(4)).ok_or_else(||anyhow::anyhow!("invalid prompt dimensions"))?;
        if width<192||height<144||bytes>crate::MAX_BYTES as usize{bail!("unsupported prompt dimensions");}
        let mut canvas=Self{width,height,pixels:vec![0;bytes]};
        canvas.rect(0,0,width,height,BACKGROUND);
        Ok(canvas)
    }
    fn rect(&mut self,x:usize,y:usize,w:usize,h:usize,color:u32){
        let pixel=color.to_ne_bytes();
        for row in y.min(self.height)..y.saturating_add(h).min(self.height){
            for col in x.min(self.width)..x.saturating_add(w).min(self.width){
                let offset=(row*self.width+col)*4;
                self.pixels[offset..offset+4].copy_from_slice(&pixel);
            }
        }
    }
    fn centered_text(&mut self,y:usize,scale:usize,text:&str,color:u32){
        let width=text.len()*8*scale;
        let x=self.width.saturating_sub(width)/2;
        for (i,ch) in text.chars().enumerate(){
            if let Some(glyph)=BASIC_FONTS.get(ch){
                for (row,bits) in glyph.into_iter().enumerate(){
                    for col in 0..8{
                        if bits&(1<<col)!=0{self.rect(x+(i*8+col)*scale,y+row*scale,scale,scale,color);}
                    }
                }
            }
        }
    }
}
/// Render a fixed-size status card into a fresh size-sealed SHM file. The caller
/// must not accept password input until this prompt is presented by the
/// compositor. Only the enum crosses the rendering boundary.
pub(super) fn render(width:u32,height:u32,status:Status)->Result<std::fs::File>{
    let mut canvas=Canvas::new(width as usize,height as usize)?;
    let scale=if width>=640&&height>=360{2}else{1};
    let w=(336*scale).min(canvas.width-16);
    let h=128*scale;
    let y=(canvas.height-h)/2;
    canvas.rect((canvas.width-w)/2,y,w,h,PANEL);
    canvas.centered_text(y+12*scale,scale,"ATELIER",ACCENT);
    canvas.centered_text(y+30*scale,scale,"SESSION LOCKED",TEXT);
    let (message,color)=match status{
        Status::Ready=>("Enter password",MUTED),
        Status::Entered=>("Password entered",TEXT),
        Status::Checking=>("Verifying...",ACCENT),
        Status::Retry=>("Try again shortly",WARNING),
        Status::Unavailable=>("Input unavailable",WARNING),
    };
    canvas.centered_text(y+60*scale,scale,message,color);
    if matches!(status,Status::Ready|Status::Entered){
        canvas.centered_text(y+86*scale,scale,"Enter: submit",MUTED);
        canvas.centered_text(y+100*scale,scale,"Esc: clear",MUTED);
    }
    let mut file=crate::allocate_file(canvas.pixels.len() as i32)?;
    file.write_all(&canvas.pixels)?;
    crate::seal_size(&file)?;
    Ok(file)
}
#[cfg(test)]
mod tests{
    use super::*;
    use std::os::unix::fs::FileExt;
    #[test]
    fn prompt_has_opaque_background_and_sealed_backing(){
        let file=render(320,240,Status::Ready).unwrap();
        let mut pixel=[0;4];file.read_exact_at(&mut pixel,0).unwrap();
        assert_eq!(u32::from_ne_bytes(pixel),BACKGROUND);
        assert_eq!(file.metadata().unwrap().len(),320*240*4);
        assert!(file.set_len(0).is_err());assert!(file.set_len(320*240*4+4).is_err());
    }
    #[test]
    fn invalid_geometry_is_rejected(){
        assert!(render(100,100,Status::Ready).is_err());
        assert!(render(u32::MAX,u32::MAX,Status::Ready).is_err());
    }
    #[test]
    fn status_changes_only_generic_prompt_pixels(){
        fn pixels(status:Status)->Vec<u8>{
            let file=render(320,240,status).unwrap();let mut data=vec![0;320*240*4];
            file.read_exact_at(&mut data,0).unwrap();data
        }
        let ready=pixels(Status::Ready);
        for status in [Status::Entered,Status::Checking,Status::Retry,Status::Unavailable]{
            assert_ne!(ready,pixels(status));
            assert_eq!(pixels(status),pixels(status));
        }
    }
}
