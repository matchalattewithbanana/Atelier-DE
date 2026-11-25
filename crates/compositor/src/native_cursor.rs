//! Native software cursor composition, separate from desktop scene effects.
use crate::state::Atelier;
use std::time::Duration;
use smithay::{
    backend::renderer::{
        element::{Element,Id,Kind,texture::TextureRenderElement,surface::{render_elements_from_surface_tree,WaylandSurfaceRenderElement}},
        gles::{GlesRenderer,GlesFrame,GlesError,GlesTexture},utils::draw_render_elements,Color32F,Renderer,ImportMem,
    },
    input::pointer::{CursorIcon,CursorImageStatus,CursorImageSurfaceData},
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

// Use i64 while clipping client-provided geometry so extreme coordinates
// cannot overflow the envelope calculation. Never guess invalid extents.
fn cursor_envelope(geometries:impl IntoIterator<Item=Rectangle<i32,Physical>>,output:Rectangle<i32,Physical>,
    empty:Rectangle<i32,Physical>)->Option<Rectangle<i32,Physical>>{
    let mut bounds:Option<(i64,i64,i64,i64)>=None;
    let ox=i64::from(output.loc.x);let oy=i64::from(output.loc.y);
    let or=ox+i64::from(output.size.w);let ob=oy+i64::from(output.size.h);
    for rect in geometries{
        if rect.size.w<0||rect.size.h<0{return None;}
        if rect.size.w==0||rect.size.h==0{continue;}
        let x=i64::from(rect.loc.x);let y=i64::from(rect.loc.y);
        let l=x.max(ox);let t=y.max(oy);
        let r=(x+i64::from(rect.size.w)).min(or);
        let b=(y+i64::from(rect.size.h)).min(ob);
        if l>=r||t>=b{continue;}
        bounds=Some(match bounds{Some((a,c,d,e))=>(a.min(l),c.min(t),d.max(r),e.max(b)),None=>(l,t,r,b)});
    }
    match bounds{
        Some((l,t,r,b))=>Some(Rectangle::new((i32::try_from(l).ok()?,i32::try_from(t).ok()?).into(),
            (i32::try_from(r-l).ok()?,i32::try_from(b-t).ok()?).into())),
        None=>Some(empty),
    }
}

type ThemeTexture=(GlesTexture,(i32,i32));
const MAX_CACHED_SHAPES:usize=40;
const MAX_FRAMES:usize=32;
const MAX_SHAPE_BYTES:usize=512*1024;

struct ThemeFrame{
    image:ThemeTexture,
    delay_ms:u32,
}

// Select one frame and the time until its next boundary. Static assets
// never schedule a wakeup. Clamp zero/very short delays at import.
fn animation_frame(delays:impl Iterator<Item=u32>+Clone,elapsed:Duration)->(usize,Option<Duration>){
    if delays.clone().count()<=1{return (0,None);}
    let total:u64=delays.clone().map(u64::from).sum();
    if total==0{return (0,None);}
    let mut offset=(elapsed.as_millis()%u128::from(total)) as u64;
    for (index,delay) in delays.enumerate(){
        let delay=u64::from(delay);
        if offset<delay{return (index,Some(Duration::from_millis(delay-offset)));}
        offset-=delay;
    }
    (0,None)
}

/// Session-local cache of bounded images, including unsuccessful lookups.
/// No repeated decoding or GPU upload on pointer motion.
pub struct CursorTheme {
    theme:xcursor::CursorTheme,
    size:u32,
    cache:Vec<(CursorIcon,Option<Vec<ThemeFrame>>)>,
    next_frame:Option<Duration>,
}

fn shape_names(icon:CursorIcon)->&'static [&'static str]{
    use CursorIcon::*;
    match icon{
        Default=>&["default","left_ptr"],
        ContextMenu=>&["context-menu","left_ptr"],
        Help=>&["help","question_arrow"],
        Pointer=>&["pointer","hand2","hand1"],
        Progress=>&["progress","left_ptr_watch"],
        Wait=>&["wait","watch"],
        Cell=>&["cell","plus"],
        Crosshair=>&["crosshair","cross"],
        Text=>&["text","xterm"],
        VerticalText=>&["vertical-text","xterm"],
        Alias=>&["alias","link"],
        Copy=>&["copy","dnd-copy"],
        Move=>&["move","fleur"],
        NoDrop=>&["no-drop","dnd-none"],
        NotAllowed=>&["not-allowed","crossed_circle"],
        Grab=>&["grab","openhand"],
        Grabbing=>&["grabbing","closedhand"],
        EResize=>&["e-resize","right_side"],
        NResize=>&["n-resize","top_side"],
        NeResize=>&["ne-resize","top_right_corner"],
        NwResize=>&["nw-resize","top_left_corner"],
        SResize=>&["s-resize","bottom_side"],
        SeResize=>&["se-resize","bottom_right_corner"],
        SwResize=>&["sw-resize","bottom_left_corner"],
        WResize=>&["w-resize","left_side"],
        EwResize=>&["ew-resize","sb_h_double_arrow"],
        NsResize=>&["ns-resize","sb_v_double_arrow"],
        NeswResize=>&["nesw-resize","fd_double_arrow"],
        NwseResize=>&["nwse-resize","bd_double_arrow"],
        ColResize=>&["col-resize","sb_h_double_arrow"],
        RowResize=>&["row-resize","sb_v_double_arrow"],
        AllScroll=>&["all-scroll","fleur"],
        ZoomIn=>&["zoom-in"],
        ZoomOut=>&["zoom-out"],
        DndAsk=>&["dnd-ask","question_arrow"],
        AllResize=>&["all-resize","fleur"],
        _=>&["default","left_ptr"],
    }
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
        let name=std::env::var("XCURSOR_THEME").ok().filter(|s|!s.is_empty()).unwrap_or_else(||"default".into());
        let size=requested_size(std::env::var("XCURSOR_SIZE").ok().as_deref());
        let mut theme=Self{theme:xcursor::CursorTheme::load(&name),size,cache:Vec::new(),next_frame:None};
        theme.ensure(renderer,CursorIcon::Default);
        theme
    }

    fn import(&self,renderer:&mut GlesRenderer,icon:CursorIcon)->anyhow::Result<Vec<ThemeFrame>>{
        use std::io::Read;
        let path=shape_names(icon).iter().find_map(|name|self.theme.load_icon(name))
            .ok_or_else(||anyhow::anyhow!("cursor shape missing from theme"))?;
        const MAX_FILE:u64=8*1024*1024;
        anyhow::ensure!(std::fs::metadata(&path)?.is_file(),"cursor image is not a regular file");
        let file=std::fs::File::open(path)?;
        anyhow::ensure!(file.metadata()?.is_file(),"cursor image is not a regular file");
        let mut data=Vec::new();
        file.take(MAX_FILE+1).read_to_end(&mut data)?;
        anyhow::ensure!(data.len() as u64<=MAX_FILE&&bounded_file(&data),"invalid or oversized cursor file");
        let images=xcursor::parser::parse_xcursor(&data).ok_or_else(||anyhow::anyhow!("invalid XCursor image"))?;
        let image=images.iter().filter(|image|valid_image(image)).min_by_key(|image|image.size.abs_diff(self.size))
            .ok_or_else(||anyhow::anyhow!("no bounded usable cursor image"))?;
        // Animate only complete, bounded nominal-size groups. An oversized
        // sequence uses its first selected image rather than truncating a loop.
        let mut frames:Vec<_>=images.iter().filter(|candidate|valid_image(candidate)&&candidate.size==image.size).collect();
        let bytes=frames.iter().map(|frame|frame.pixels_rgba.len()).sum::<usize>();
        if frames.len()>MAX_FRAMES||bytes>MAX_SHAPE_BYTES{frames=vec![image];}
        let mut imported=Vec::with_capacity(frames.len());
        for frame in frames{
            let rgba=file_pixels_to_rgba(&frame.pixels_rgba);
            let texture=renderer.import_memory(&rgba,smithay::backend::allocator::Fourcc::Abgr8888,
                (frame.width as i32,frame.height as i32).into(),false)?;
            imported.push(ThemeFrame{image:(texture,(frame.xhot as i32,frame.yhot as i32)),delay_ms:frame.delay.clamp(16,60_000)});
        }
        Ok(imported)
    }

    fn ensure(&mut self,renderer:&mut GlesRenderer,icon:CursorIcon){
        if self.cache.len()>=MAX_CACHED_SHAPES||self.cache.iter().any(|(key,_)|*key==icon){return;}
        let texture=match self.import(renderer,icon){
            Ok(texture)=>Some(texture),
            Err(error)=>{tracing::debug!(?icon,?error,"cursor shape unavailable; using fallback");None}
        };
        self.cache.push((icon,texture));
    }

    pub fn animation_due(&self,now:Duration)->bool{
        self.next_frame.is_some_and(|deadline|now>=deadline)
    }

    fn element(&mut self,renderer:&mut GlesRenderer,position:Point<i32,Physical>,icon:CursorIcon,now:Duration)->Option<TextureRenderElement<GlesTexture>>{
        self.ensure(renderer,icon);
        let lookup=|key|self.cache.iter().find(|(cached,_)|*cached==key).and_then(|(_,image)|image.as_ref());
        let frames=lookup(icon).or_else(||lookup(CursorIcon::Default))?;
        let (index,remaining)=animation_frame(frames.iter().map(|frame|frame.delay_ms),now);
        let (texture,hotspot)=&frames.get(index)?.image;
        let origin=hotspot_origin(position,*hotspot);
        let element=TextureRenderElement::from_static_texture(Id::new(),renderer.context_id(),origin.to_f64(),
            texture.clone(),1,Transform::Normal,Some(1.0),None,None,None,Kind::Cursor);
        self.next_frame=remaining.and_then(|delay|now.checked_add(delay));
        Some(element)
    }
}

impl NativeCursor {
    pub fn prepare(state:&Atelier,renderer:&mut GlesRenderer,output:Size<i32,Physical>,theme:&mut CursorTheme,now:Duration)->Self{
        // Hidden and client-owned images must not inherit a theme timer.
        theme.next_frame=None;
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
            CursorImageStatus::Named(icon)=>theme.element(renderer,position,*icon,now).map_or(Image::Default,Image::Theme),
            _=>theme.element(renderer,position,CursorIcon::Default,now).map_or(Image::Default,Image::Theme),
        };
        Self{position,output,image}
    }
    /// A conservative envelope of everything drawn by this cursor.
    /// None means uncertain geometry and forces a full-frame baseline.
    /// Hidden or fully offscreen cursors retain a single harmless pixel
    /// at the pointer position to keep the existing damage API nonempty.
    pub fn bounds(&self)->Option<Rectangle<i32,Physical>>{
        let output=Rectangle::from_size(self.output);
        let empty=Rectangle::new(self.position,(1,1).into());
        match &self.image{
            Image::Default=>Some(Rectangle::new(self.position,(11.min(self.output.w-self.position.x),16.min(self.output.h-self.position.y)).into())),
            Image::Hidden=>Some(empty),
            Image::Theme(element)=>cursor_envelope(std::iter::once(element.geometry(Scale::from(1.0))),output,empty),
            Image::Client(elements)=>{
                if elements.len()>64{return None;}
                cursor_envelope(elements.iter().map(|e|e.geometry(Scale::from(1.0))),output,empty)
            }
        }
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
    fn animation_boundaries_and_wraparound(){
        let delays=[30,70,100];
        let select=|ms|animation_frame(delays.into_iter(),Duration::from_millis(ms));
        assert_eq!(select(0),(0,Some(Duration::from_millis(30))));
        assert_eq!(select(29),(0,Some(Duration::from_millis(1))));
        assert_eq!(select(30),(1,Some(Duration::from_millis(70))));
        assert_eq!(select(100),(2,Some(Duration::from_millis(100))));
        assert_eq!(select(200),(0,Some(Duration::from_millis(30))));
        assert_eq!(select(430),(1,Some(Duration::from_millis(70))));
        assert_eq!(animation_frame([16].into_iter(),Duration::from_secs(10)),(0,None));
        assert_eq!(animation_frame([0,0].into_iter(),Duration::ZERO),(0,None));
    }
    #[test]
    fn shape_names_include_legacy_fallbacks(){
        assert_eq!(shape_names(CursorIcon::Default),&["default","left_ptr"]);
        assert_eq!(shape_names(CursorIcon::Text),&["text","xterm"]);
        assert_eq!(shape_names(CursorIcon::Pointer),&["pointer","hand2","hand1"]);
        assert_eq!(shape_names(CursorIcon::NwseResize),&["nwse-resize","bd_double_arrow"]);
    }
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
    fn cursor_envelope_clips_and_unions_surface_trees(){
        let rect=|x,y,w,h|Rectangle::new((x,y).into(),(w,h).into());
        let output=rect(0,0,100,80);
        let empty=rect(20,20,1,1);
        assert_eq!(cursor_envelope([rect(-10,-5,30,25),rect(18,15,12,12)],output,empty),Some(rect(0,0,30,27)));
        assert_eq!(cursor_envelope([rect(90,70,32,32)],output,empty),Some(rect(90,70,10,10)));
        assert_eq!(cursor_envelope([rect(i32::MAX-1,0,100,10)],output,empty),Some(empty));
        assert_eq!(cursor_envelope([],output,empty),Some(empty));
        assert_eq!(cursor_envelope([rect(0,0,0,10)],output,empty),Some(empty));
    }
    #[test]
    fn cursor_hotspots_allow_offscreen_origins_without_integer_overflow(){
        assert_eq!(hotspot_origin((10,20).into(),(3,5)),(7,15).into());
        assert_eq!(hotspot_origin((0,0).into(),(8,12)),(-8,-12).into());
        assert_eq!(hotspot_origin((10,10).into(),(i32::MIN,i32::MAX)),(i32::MAX,10-i32::MAX).into());
    }
}
