//! Optional antialiased group composition for rounded ordinary windows.
//! Keep a strict aggregate pixel/pass budget, and fall back to the existing
//! disjoint damage strips on unsupported sizes or renderer errors.
use smithay::{
    backend::{allocator::Fourcc,renderer::{
        Bind,Offscreen,Renderer,Frame,Color32F,
        element::{Id,Kind,surface::{render_elements_from_surface_tree,WaylandSurfaceRenderElement},texture::TextureRenderElement},
        gles::{GlesRenderer,GlesTexture,GlesTexProgram,Uniform,UniformName,UniformType,UniformValue,element::TextureShaderElement},
        utils::draw_render_elements,
    }},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical,Physical,Point,Rectangle,Size,Transform},
};

const MAX_WINDOWS:usize=4;
const MAX_PIXELS:i64=4_194_304;
const MAX_EDGE:i32=4096;

fn pixels(size:Size<i32,Logical>)->Option<i64>{
    if !(1..=MAX_EDGE).contains(&size.w)||!(1..=MAX_EDGE).contains(&size.h){return None;}
    let pixels=i64::from(size.w)*i64::from(size.h);
    (pixels<=MAX_PIXELS).then_some(pixels)
}
#[derive(Default)]
struct Budget{used:usize,pixels:i64}
impl Budget{
    fn allows(&self,size:Size<i32,Logical>)->Option<i64>{
        let count=pixels(size)?;
        (self.used<MAX_WINDOWS&&self.pixels+count<=MAX_PIXELS).then_some(count)
    }
    fn consume(&mut self,count:i64){self.used+=1;self.pixels+=count;}
}
struct Slot{size:Size<i32,Physical>,texture:GlesTexture}
#[derive(Default)]
pub(super) struct RoundedWindows{
    program:Option<GlesTexProgram>,
    slots:Vec<Slot>,
    budget:Budget,
    disabled:bool,
    logged:bool,
}
impl RoundedWindows{
    pub fn begin_frame(&mut self){self.budget=Budget::default();}
    pub fn end_frame(&mut self){self.slots.truncate(self.budget.used);}
    pub fn render(&mut self,renderer:&mut GlesRenderer,surface:&WlSurface,area:Rectangle<i32,Logical>,geometry_origin:Point<i32,Logical>,radius:u32,alpha:f32)->Option<TextureShaderElement>{
        if self.disabled||radius==0{return None;}
        let count=self.budget.allows(area.size)?;
        match self.try_render(renderer,surface,area,geometry_origin,radius,alpha){
            Ok(Some(element))=>{
                self.budget.consume(count);
                if !self.logged{println!("Atelier antialiased rounded window rendered");self.logged=true;}
                Some(element)
            }
            Ok(None)=>None,
            Err(error)=>{
                tracing::warn!(?error,"rounded group composition unavailable; retaining pixel-aligned clipping");
                self.disabled=true;
                // Earlier elements in this frame own texture references.
                // Release the cache without invalidating them.
                self.slots.clear();
                None
            }
        }
    }
    fn try_render(&mut self,renderer:&mut GlesRenderer,surface:&WlSurface,area:Rectangle<i32,Logical>,geometry_origin:Point<i32,Logical>,radius:u32,alpha:f32)->anyhow::Result<Option<TextureShaderElement>>{
        let local=Point::<i32,Logical>::from((0,0))-geometry_origin;
        let surfaces=render_elements_from_surface_tree::<GlesRenderer,WaylandSurfaceRenderElement<GlesRenderer>>(renderer,surface,local.to_physical_precise_round(1.0),1.0,1.0,Kind::Unspecified);
        if surfaces.is_empty(){return Ok(None);}
        if self.program.is_none(){
            self.program=Some(renderer.compile_custom_texture_shader(include_str!("rounded.frag"),&[
                UniformName::new("shape_size",UniformType::_2f),
                UniformName::new("corner_radius",UniformType::_1f),
            ])?);
        }
        let size=area.size.to_physical(1);
        let index=self.budget.used;
        // Drop unused old slots if a resized/reordered window needs space.
        // The current frame's already used slots are never replaced.
        if self.slots.get(index).is_none_or(|slot|slot.size!=size){
            self.slots.truncate(index);
            let texture:GlesTexture=renderer.create_buffer(Fourcc::Abgr8888,(size.w,size.h).into())?;
            self.slots.push(Slot{size,texture});
        }
        let slot=&mut self.slots[index];
        {
            let mut target=renderer.bind(&mut slot.texture)?;
            let mut frame=renderer.render(&mut target,size,Transform::Normal)?;
            let damage=Rectangle::from_size(size);
            frame.clear(Color32F::new(0.0,0.0,0.0,0.0),&[damage])?;
            draw_render_elements(&mut frame,1.0,&surfaces,&[damage])?;
            let _fence=frame.finish()?;
        }
        // Parent and subsurfaces are now composited once. Apply global
        // window opacity and the rounded mask to their premultiplied result.
        let inner=TextureRenderElement::from_static_texture(
            Id::new(),renderer.context_id(),area.loc.to_f64().to_physical(1.0),
            slot.texture.clone(),1,Transform::Normal,Some(alpha),None,Some(area.size),None,Kind::Unspecified,
        );
        let radius=radius.min(24).min((size.w/2) as u32).min((size.h/2) as u32) as f32;
        Ok(Some(TextureShaderElement::new(inner,self.program.as_ref().expect("program compiled").clone(),vec![
            Uniform::new("shape_size",UniformValue::_2f(size.w as f32,size.h as f32)),
            Uniform::new("corner_radius",UniformValue::_1f(radius)),
        ])))
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn aggregate_budget_bounds_passes_and_pixels(){
        let mut b=Budget::default();
        let full=b.allows((1920,1080).into()).unwrap();
        b.consume(full);b.consume(b.allows((1920,1080).into()).unwrap());
        assert!(b.allows((400,400).into()).is_none());
        assert!(b.allows((0,30).into()).is_none());
        assert!(pixels((8192,1).into()).is_none());
        assert!(pixels((3840,2160).into()).is_none());
        let mut b=Budget::default();
        for _ in 0..MAX_WINDOWS{let n=b.allows((32,32).into()).unwrap();b.consume(n);}
        assert!(b.allows((1,1).into()).is_none());
    }
}
