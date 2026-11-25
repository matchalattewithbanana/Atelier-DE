//! Native software cursor composition, separate from desktop scene effects.
use crate::state::Atelier;
use smithay::{
    backend::renderer::{
        element::{Kind,surface::{render_elements_from_surface_tree,WaylandSurfaceRenderElement}},
        gles::{GlesRenderer,GlesFrame,GlesError},utils::draw_render_elements,Color32F,Frame,
    },
    input::pointer::{CursorImageStatus,CursorImageSurfaceData},
    reexports::wayland_server::Resource,
    utils::{Physical,Point,Rectangle,Size,Scale},
    wayland::compositor::with_states,
};

enum Image {
    Default,
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

impl NativeCursor {
    pub fn prepare(state:&Atelier,renderer:&mut GlesRenderer,output:Size<i32,Physical>)->Self{
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
            _=>Image::Default,
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
    fn cursor_hotspots_allow_offscreen_origins_without_integer_overflow(){
        assert_eq!(hotspot_origin((10,20).into(),(3,5)),(7,15).into());
        assert_eq!(hotspot_origin((0,0).into(),(8,12)),(-8,-12).into());
        assert_eq!(hotspot_origin((10,10).into(),(i32::MIN,i32::MAX)),(i32::MAX,10-i32::MAX).into());
    }
}
