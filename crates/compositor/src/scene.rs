//! One front-to-back scene for both GLES backends. Borders are four
//! noninteractive solid elements behind their own window, never a global overlay.
use crate::{state::Atelier,layout::Tile};
use anyhow::{Context,Result};
use smithay::{
    backend::renderer::{
        gles::GlesRenderer,
        element::{render_elements,AsRenderElements,Kind,solid::{SolidColorBuffer,SolidColorRenderElement},surface::WaylandSurfaceRenderElement},
    },
    desktop::layer_map_for_output,
    utils::{Logical,Rectangle,Scale},
    wayland::shell::wlr_layer::Layer,
};
render_elements!{
    pub SceneElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Border=SolidColorRenderElement,
}
pub fn inset_tile(tile:Tile,width:u32)->Tile{
    let inset=(width.min(8) as i32).min((tile.width-1).max(0)/2).min((tile.height-1).max(0)/2);
    Tile{x:tile.x+inset,y:tile.y+inset,width:tile.width-2*inset,height:tile.height-2*inset}
}
/// Clip each outside strip independently, preserving the content rectangle.
/// Use wider intermediate arithmetic for client-supplied window dimensions.
fn strips(rect:Rectangle<i32,Logical>,clip:Rectangle<i32,Logical>,width:u32)->Vec<Rectangle<i32,Logical>>{
    if width==0||rect.size.w<=0||rect.size.h<=0||clip.size.w<=0||clip.size.h<=0{return Vec::new();}
    let x=i64::from(rect.loc.x);let y=i64::from(rect.loc.y);
    let r=x+i64::from(rect.size.w);let b=y+i64::from(rect.size.h);
    let w=i64::from(width.min(8));
    let cx=i64::from(clip.loc.x);let cy=i64::from(clip.loc.y);
    let cr=cx+i64::from(clip.size.w);let cb=cy+i64::from(clip.size.h);
    [(x-w,y-w,r+w,y),(x-w,b,r+w,b+w),(x-w,y,x,b),(r,y,r+w,b)].into_iter().filter_map(|(l,t,r,b)|{
        let (l,t,r,b)=(l.max(cx),t.max(cy),r.min(cr),b.min(cb));
        if l>=r||t>=b{return None;}
        Some(Rectangle::new((i32::try_from(l).ok()?,i32::try_from(t).ok()?).into(),(i32::try_from(r-l).ok()?,i32::try_from(b-t).ok()?).into()))
    }).collect()
}
pub fn elements(state:&Atelier,renderer:&mut GlesRenderer)->Result<Vec<SceneElement>>{
    let output=&state.output;
    let bounds=state.space.output_geometry(output).context("unmapped output")?;
    let scale=output.current_scale().fractional_scale();
    let map=layer_map_for_output(output);
    let mut result=Vec::new();
    // Render layer roots at their actual arranged origin (bbox can include
    // negative subsurface offsets). Match input routing's layer ordering.
    let mut layers=|levels:&[Layer],result:&mut Vec<SceneElement>,renderer:&mut GlesRenderer|{
        for &level in levels{
            for layer in map.layers_on(level).rev(){
                let Some(geometry)=map.layer_geometry(layer)else{continue;};
                let origin=geometry.loc-layer.bbox().loc-bounds.loc;
                result.extend(layer.render_elements::<SceneElement>(renderer,origin.to_physical_precise_round(scale),Scale::from(scale),1.0));
            }
        }
    };
    layers(&[Layer::Overlay,Layer::Top],&mut result,renderer);
    let clip=map.non_exclusive_zone();
    for window in state.space.elements().rev(){
        let Some(geometry)=state.space.element_geometry(window)else{continue;};
        let origin=geometry.loc-window.geometry().loc-bounds.loc;
        result.extend(window.render_elements::<SceneElement>(renderer,origin.to_physical_precise_round(scale),Scale::from(scale),1.0));
        if let Some((border,focused))=state.window_border(window){
            let color=border.color(focused)?;
            for strip in strips(geometry,clip,border.width){
                let buffer=SolidColorBuffer::new(strip.size,color);
                result.push(SceneElement::from(SolidColorRenderElement::from_buffer(&buffer,(strip.loc-bounds.loc).to_physical_precise_round(scale),scale,1.0,Kind::Unspecified)));
            }
        }
    }
    layers(&[Layer::Bottom,Layer::Background],&mut result,renderer);
    Ok(result)
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn ring_is_outside_content_and_clipped(){
        let content=Rectangle::new((2,2).into(),(10,10).into());
        let clip=Rectangle::new((0,0).into(),(20,20).into());
        let ring=strips(content,clip,4);
        assert_eq!(ring.len(),4);
        for (i,strip) in ring.iter().enumerate(){
            assert!(!strip.overlaps(content));assert_eq!(strip.intersection(clip),Some(*strip));
            for other in &ring[i+1..]{assert!(!strip.overlaps(*other));}
        }
        assert!(strips(content,clip,0).is_empty());
    }
    #[test]
    fn tiny_tiles_retain_positive_content(){
        assert_eq!(inset_tile(Tile{x:0,y:0,width:5,height:3},8),Tile{x:1,y:1,width:3,height:1});
        assert_eq!(inset_tile(Tile{x:0,y:0,width:1,height:1},8),Tile{x:0,y:0,width:1,height:1});
    }
}
