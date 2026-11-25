//! Native software cursor composition, separate from desktop scene effects.
use crate::state::Atelier;
use smithay::{
    backend::renderer::{
        element::{Id,Kind,texture::TextureRenderElement,surface::{render_elements_from_surface_tree,WaylandSurfaceRenderElement}},
        gles::{GlesRenderer,GlesFrame,GlesError,GlesTexture},utils::draw_render_elements,Color32F,Renderer,ImportMem,
    },
    input::pointer::{CursorImageStatus,CursorImageSurfaceData},
    reexports::wayland_server::Resource,
    utils::{Physical,Point,Rectangle,Size,Scale,Transform},
    wayland::compositor::with_states,
};

enum Image {
    Default,
    Theme(TextureRenderElement<GlesTexture>),
    Hidden,
    Client(Vec<WaylandSurfaceRenderElement<GlesRenderer>>),
}

pub struct NativeCursor {
    position:Point<i32,Physical>,
    output:Size<i32,Physical>,
    image:Image,
}

fn hotspot_origin(position:Point<i32,Physical>,hotspot:(i32,i32))->Point<i32,Physical>{
    (position.x.saturating_sub(hotspot.0),position.y.saturating_sub(hotspot.1)).into()
}

/// A single immutable default-theme image, imported once per renderer.
pub struct CursorTheme {
    texture:Option<(GlesTexture,(i32,i32))>,
}

fn requested_size(value:Option<&str>)->u32{
    value.and_then(|s|s.parse::<u32>().ok()).filter(|s|(8..=128).contains(s)).unwrap_or(24)
}

fn valid_image(image:&xcursor::parser::Image)->bool{
    (1..=256).contains(&image.width)&&(1..=256).contains(&image.height)
        &&image.xhot<image.width&&image.yhot<image.height
        &&image.pixels_rgba.len()==(image.width as usize)*(image.height as usize)*4
}

fn file_pixels_to_rgba(bytes:&[u8])->Vec<u8>{
    // pixels_rgba preserves the bytes from disk: little-endian
    // premultiplied ARGB words (B,G,R,A).
    let mut rgba=bytes.to_vec();
    for pixel in rgba.chunks_exact_mut(4){pixel.swap(0,2);}
    rgba
}

// Preflight the table before the library allocates decoded images.
// Bound dimensions, image count and aggregate decoded storage.
fn bounded_file(data:&[u8])->bool{
    let word=|offset:usize|->Option<u32>{Some(u32::from_le_bytes(data.get(offset..offset.checked_add(4)?)?.try_into().ok()?))};
    let check=(||->Option<()>{
        if data.get(..4)?!=b"Xcur"{return None;}
        let header=usize::try_from(word(4)?).ok()?;
        let count=usize::try_from(word(12)?).ok()?;
        if header<16||count>256||header.checked_add(count.checked_mul(12)?)?>data.len(){return None;}
        let mut total=0usize;
        for index in 0..count{
            let at=header+index*12;
            if word(at)?!=0xfffd0002{continue;}
            let chunk=usize::try_from(word(at+8)?).ok()?;
            let length=usize::try_from(word(chunk)?).ok()?;
            if length<36||chunk.checked_add(length)?>data.len(){return None;}
            let w=word(chunk.checked_add(16)?)?;
            let h=word(chunk.checked_add(20)?)?;
            if !(1..=256).contains(&w)||!(1..=256).contains(&h){return None;}
            let bytes=(w as usize)*(h as usize)*4;
            total=total.checked_add(bytes)?;
            if total>8*1024*1024||chunk.checked_add(length)?.checked_add(bytes)?>data.len(){return None;}
        }
        Some(())
    })();
    check.is_some()
}

impl CursorTheme {
    pub fn load(renderer:&mut GlesRenderer)->Self{
        use std::io::Read;
        let name=std::env::var("XCURSOR_THEME").ok().filter(|s|!s.is_empty()).unwrap_or_else(||"default".into());
        let size=requested_size(std::env::var("XCURSOR_SIZE").ok().as_deref());
        let result=(||->anyhow::Result<_>{
            let theme=xcursor::CursorTheme::load(&name);
            let path=theme.load_icon("default").or_else(||theme.load_icon("left_ptr"))
                .ok_or_else(||anyhow::anyhow!("no default cursor in theme"))?;
            const MAX_FILE:u64=8*1024*1024;
            let file=std::fs::File::open(path)?;
            anyhow::ensure!(file.metadata()?.is_file(),"cursor image is not a regular file");
            let mut data=Vec::new();
            file.take(MAX_FILE+1).read_to_end(&mut data)?;
            anyhow::ensure!(data.len() as u64<=MAX_FILE&&bounded_file(&data),"invalid or oversized cursor file");
            let images=xcursor::parser::parse_xcursor(&data).ok_or_else(||anyhow::anyhow!("invalid XCursor image"))?;
            let image=images.iter().filter(|image|valid_image(image)).min_by_key(|image|image.size.abs_diff(size))
                .ok_or_else(||anyhow::anyhow!("no bounded usable cursor image"))?;
            let rgba=file_pixels_to_rgba(&image.pixels_rgba);
            let texture=renderer.import_memory(&rgba,smithay::backend::allocator::Fourcc::Abgr8888,
                (image.width as i32,image.height as i32).into(),false)?;
            Ok((texture,(image.xhot as i32,image.yhot as i32)))
        })();
        match result{
            Ok(texture)=>Self{texture:Some(texture)},
            Err(error)=>{tracing::debug!(?error,"cursor theme unavailable; using default glyph");Self{texture:None}}
        }
    }

    fn element(&self,renderer:&GlesRenderer,position:Point<i32,Physical>)->Option<TextureRenderElement<GlesTexture>>{
        let (texture,hotspot)=self.texture.as_ref()?;
        let origin=hotspot_origin(position,*hotspot);
        Some(TextureRenderElement::from_static_texture(Id::new(),renderer.context_id(),origin.to_f64(),
            texture.clone(),1,Transform::Normal,Some(1.0),None,None,None,Kind::Cursor))
    }
}

impl NativeCursor {
    pub fn prepare(state:&Atelier,renderer:&mut GlesRenderer,output:Size<i32,Physical>,theme:&CursorTheme)->Self{
        let position:Point<i32,Physical>=((state.pointer_position.x as i32).clamp(0,output.w-1),
            (state.pointer_position.y as i32).clamp(0,output.h-1)).into();
        let image=match state.cursor_status(){
            CursorImageStatus::Hidden=>Image::Hidden,
            CursorImageStatus::Surface(surface) if surface.is_alive()=>{
                let hotspot=with_states(surface,|states|{
                    states.data_map.get::<CursorImageSurfaceData>()
                        .and_then(|data|data.lock().ok().map(|data|data.hotspot))
                }).unwrap_or_default();
                // Limit malicious extreme coordinates before importing the tree.
                // A cursor that is far outside any usable display is invisible.
                let origin=hotspot_origin(position,(hotspot.x,hotspot.y));
                if origin.x.unsigned_abs()>65536||origin.y.unsigned_abs()>65536{
                    Image::Hidden
                }else{
                    Image::Client(render_elements_from_surface_tree(renderer,surface,origin,Scale::from(1.0),1.0,Kind::Cursor))
                }
            }
            _=>theme.element(renderer,position).map_or(Image::Default,Image::Theme),
        };
        Self{position,output,image}
    }
    pub fn is_default(&self)->bool{matches!(&self.image,Image::Default)}
    pub fn default_bounds(&self)->Rectangle<i32,Physical>{
        Rectangle::new(self.position,(11.min(self.output.w-self.position.x),16.min(self.output.h-self.position.y)).into())
    }
    pub fn draw(&self,frame:&mut GlesFrame<'_,'_>,output:Rectangle<i32,Physical>)->Result<(),GlesError>{
        match &self.image{
            Image::Hidden=>Ok(()),
            Image::Theme(element)=>{draw_render_elements::<GlesRenderer,_,_>(frame,1.0,std::slice::from_ref(element),&[output])?;Ok(())}
            Image::Client(elements)=>{draw_render_elements(frame,1.0,elements,&[output])?;Ok(())}
            Image::Default=>{
                for (w,h) in [(3,16),(11,3)]{
                    let cursor=Rectangle::new(self.position,(w.min(self.output.w-self.position.x),h.min(self.output.h-self.position.y)).into());
                    frame.draw_solid(cursor,&[Rectangle::from_size(cursor.size)],Color32F::new(1.0,1.0,1.0,1.0))?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn theme_size_and_channel_order(){
        assert_eq!(requested_size(Some("32")),32);
        for value in ["0","9999","-1","abc"]{assert_eq!(requested_size(Some(value)),24);}
        assert_eq!(requested_size(None),24);
        assert_eq!(file_pixels_to_rgba(&[20,40,60,128,0,0,0,0]),vec![60,40,20,128,0,0,0,0]);
        assert!(!bounded_file(&[]));
        assert!(!bounded_file(b"Xcur"));
    }
    #[test]
    fn cursor_hotspots_allow_offscreen_origins_without_integer_overflow(){
        assert_eq!(hotspot_origin((10,20).into(),(3,5)),(7,15).into());
        assert_eq!(hotspot_origin((0,0).into(),(8,12)),(-8,-12).into());
        assert_eq!(hotspot_origin((10,10).into(),(i32::MIN,i32::MAX)),(i32::MAX,10-i32::MAX).into());
    }
}
