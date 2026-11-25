//! One front-to-back scene for both GLES backends. Decorations sit behind
//! their own window, never a global overlay.
use crate::{state::Atelier,layout::Tile};
use anyhow::{Context,Result};
use smithay::{
    backend::renderer::{
        gles::{GlesRenderer,GlesPixelProgram,Uniform,UniformName,UniformType,UniformValue,element::PixelShaderElement},
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
    Shadow=PixelShaderElement,
}
pub fn inset_tile(tile:Tile,width:u32)->Tile{
    let inset=(width.min(8) as i32).min((tile.width-1).max(0)/2).min((tile.height-1).max(0)/2);
    Tile{x:tile.x+inset,y:tile.y+inset,width:tile.width-2*inset,height:tile.height-2*inset}
}
/// Clip each outside strip independently, preserving the content rectangle.
/// Use wider intermediate arithmetic for client-supplied window dimensions.
fn strips(rect:Rectangle<i32,Logical>,clip:Rectangle<i32,Logical>,width:u32)->Vec<Rectangle<i32,Logical>>{
    ring(rect,clip,0,width.min(8))
}
/// Nonoverlapping ring between two expansions of the content rectangle.
fn ring(rect:Rectangle<i32,Logical>,clip:Rectangle<i32,Logical>,inner:u32,outer:u32)->Vec<Rectangle<i32,Logical>>{
    if outer<=inner||outer>40||rect.size.w<=0||rect.size.h<=0||clip.size.w<=0||clip.size.h<=0{return Vec::new();}
    let x=i64::from(rect.loc.x);let y=i64::from(rect.loc.y);
    let r=x+i64::from(rect.size.w);let b=y+i64::from(rect.size.h);
    let i=i64::from(inner);let w=i64::from(outer);
    let cx=i64::from(clip.loc.x);let cy=i64::from(clip.loc.y);
    let cr=cx+i64::from(clip.size.w);let cb=cy+i64::from(clip.size.h);
    [(x-w,y-w,r+w,y-i),(x-w,b+i,r+w,b+w),(x-w,y-i,x-i,b+i),(r+i,y-i,r+w,b+i)].into_iter().filter_map(|(l,t,r,b)|{
        let (l,t,r,b)=(l.max(cx),t.max(cy),r.min(cr),b.min(cb));
        if l>=r||t>=b{return None;}
        Some(Rectangle::new((i32::try_from(l).ok()?,i32::try_from(t).ok()?).into(),(i32::try_from(r-l).ok()?,i32::try_from(b-t).ok()?).into()))
    }).collect()
}
/// At most eight disjoint rings (32 solid elements) per window, with a
/// quadratic alpha falloff. No framebuffer readback, texture allocation or
/// blur pass. Black is already premultiplied at any alpha.
fn shadow_bands(rect:Rectangle<i32,Logical>,clip:Rectangle<i32,Logical>,border:u32,shadow:atelier_config::WindowShadow)->Vec<(Rectangle<i32,Logical>,f32)>{
    let radius=shadow.radius.min(32);let opacity=shadow.opacity.min(60) as f32/100.0;
    if radius==0||opacity==0.0{return Vec::new();}
    let bands=radius.min(8);let border=border.min(8);
    let mut result=Vec::with_capacity((bands*4) as usize);
    for band in 0..bands{
        let inner=band*radius/bands;let outer=(band+1)*radius/bands;
        let midpoint=(inner+outer) as f32/(2.0*radius as f32);
        let alpha=opacity*(1.0-midpoint).powi(2);
        result.extend(ring(rect,clip,border+inner,border+outer).into_iter().map(|r|(r,alpha)));
    }
    result
}
/// Owned by one renderer for its lifetime. Compile once on first use and
/// retain the banded path if the driver's shader compiler rejects the program.
#[derive(Default)]
pub struct SceneShaders{
    shadow:Option<Option<GlesPixelProgram>>,
}
impl SceneShaders{
    fn shadow(&mut self,renderer:&mut GlesRenderer)->Option<&GlesPixelProgram>{
        self.shadow.get_or_insert_with(||{
            match renderer.compile_custom_pixel_shader(include_str!("shadow.frag"),&[
                UniformName::new("outline",UniformType::_4f),
                UniformName::new("radius",UniformType::_1f),
            ]){
                Ok(program)=>Some(program),
                Err(error)=>{tracing::warn!(?error,"smooth shadow shader unavailable; using banded shadows");None}
            }
        }).as_ref()
    }
}
/// Outline bounds relative to the individual clipped strip's local coordinates.
/// Wider arithmetic avoids overflow from client-provided dimensions.
fn relative_outline(content:Rectangle<i32,Logical>,strip:Rectangle<i32,Logical>,border:u32)->[f32;4]{
    let x=i64::from(content.loc.x)-i64::from(strip.loc.x);
    let y=i64::from(content.loc.y)-i64::from(strip.loc.y);
    let b=i64::from(border.min(8));
    [(x-b) as f32,(y-b) as f32,(x+i64::from(content.size.w)+b) as f32,(y+i64::from(content.size.h)+b) as f32]
}
pub fn elements(state:&Atelier,renderer:&mut GlesRenderer,shaders:&mut SceneShaders)->Result<Vec<SceneElement>>{
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
        result.extend(window.render_elements::<SceneElement>(renderer,origin.to_physical_precise_round(scale),Scale::from(scale),state.window_alpha(window)));
        if let Some((border,focused))=state.window_border(window){
            let color=border.color(focused)?;
            for strip in strips(geometry,clip,border.width){
                let buffer=SolidColorBuffer::new(strip.size,color);
                result.push(SceneElement::from(SolidColorRenderElement::from_buffer(&buffer,(strip.loc-bounds.loc).to_physical_precise_round(scale),scale,1.0,Kind::Unspecified)));
            }
        }
        if let Some(shadow)=state.window_shadow(window){
            let border=state.window_border(window).map_or(0,|(b,_)|b.width);
            if let Some(program)=shaders.shadow(renderer){
                // Draw only outside the outline: four clipped strips, not the
                // entire window interior. Euclidean distance rounds the shadow
                // corners without changing client content or hit testing.
                for strip in ring(geometry,clip,border,border+shadow.radius){
                    let [l,t,r,b]=relative_outline(geometry,strip,border);
                    let area=Rectangle::new(strip.loc-bounds.loc,strip.size);
                    result.push(SceneElement::from(PixelShaderElement::new(
                        program.clone(),area,None,shadow.opacity as f32/100.0,
                        vec![Uniform::new("outline",UniformValue::_4f(l,t,r,b)),
                             Uniform::new("radius",UniformValue::_1f(shadow.radius as f32))],
                        Kind::Unspecified,
                    )));
                }
            }else{
                for (strip,alpha) in shadow_bands(geometry,clip,border,shadow){
                    let buffer=SolidColorBuffer::new(strip.size,[0.0,0.0,0.0,alpha]);
                    result.push(SceneElement::from(SolidColorRenderElement::from_buffer(&buffer,(strip.loc-bounds.loc).to_physical_precise_round(scale),scale,1.0,Kind::Unspecified)));
                }
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
    fn shadows_are_disjoint_clipped_and_fade_outwards(){
        let content=Rectangle::new((30,30).into(),(60,40).into());
        let clip=Rectangle::new((0,0).into(),(120,100).into());
        let border=strips(content,clip,3);
        let shadow=shadow_bands(content,clip,3,atelier_config::WindowShadow{radius:32,opacity:60});
        assert!(!shadow.is_empty()&&shadow.len()<=32);
        for (i,(r,alpha)) in shadow.iter().enumerate(){
            assert!(*alpha>0.0&&*alpha<=0.6);assert!(!r.overlaps(content));assert_eq!(r.intersection(clip),Some(*r));
            assert!(border.iter().all(|b|!b.overlaps(*r)));
            assert!(shadow[i+1..].iter().all(|(other,a)|!other.overlaps(*r)&&*a<=*alpha));
        }
        assert!(shadow_bands(content,clip,3,atelier_config::WindowShadow::default()).is_empty());
    }
    #[test]
    fn gpu_strip_coordinates_preserve_clipped_outline(){
        let content=Rectangle::new((20,30).into(),(60,40).into());
        let clip=Rectangle::new((0,10).into(),(100,100).into());
        let strips=ring(content,clip,2,26);
        assert_eq!(strips.len(),4);
        for (i,strip) in strips.iter().enumerate(){
            let [l,t,r,b]=relative_outline(content,*strip,2);
            assert_eq!(r-l,64.0);assert_eq!(b-t,44.0);
            assert_eq!(l+strip.loc.x as f32,18.0);assert_eq!(t+strip.loc.y as f32,28.0);
            assert_eq!(strip.intersection(clip),Some(*strip));
            assert!(!strip.overlaps(content));
            assert!(strips[i+1..].iter().all(|other|!strip.overlaps(*other)));
        }
    }
    #[test]
    fn tiny_tiles_retain_positive_content(){
        assert_eq!(inset_tile(Tile{x:0,y:0,width:5,height:3},8),Tile{x:1,y:1,width:3,height:1});
        assert_eq!(inset_tile(Tile{x:0,y:0,width:1,height:1},8),Tile{x:0,y:0,width:1,height:1});
    }
}
