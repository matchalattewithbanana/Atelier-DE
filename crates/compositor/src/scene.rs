//! One front-to-back scene for both GLES backends. Decorations sit behind
//! their own window, never a global overlay.
use crate::{state::Atelier,layout::Tile};
mod backdrop;
pub(crate) use backdrop::BlurTarget;
use anyhow::{Context,Result};
use smithay::{
    backend::renderer::{
        gles::{GlesRenderer,GlesFrame,GlesError,GlesPixelProgram,Uniform,UniformName,UniformType,UniformValue,element::{PixelShaderElement,TextureShaderElement}},
        utils::CommitCounter,
        element::{render_elements,AsRenderElements,Element,RenderElement,Id,Kind,solid::{SolidColorBuffer,SolidColorRenderElement},surface::{WaylandSurfaceRenderElement,render_elements_from_surface_tree}},
    },
    desktop::{layer_map_for_output,PopupManager},
    utils::{Buffer,Logical,Physical,Point,Rectangle,Scale,Transform},
    wayland::{compositor::{with_states,SurfaceAttributes,RectangleKind},shell::wlr_layer::Layer},
};
render_elements!{
    pub SceneElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Border=SolidColorRenderElement,
    Shadow=PixelShaderElement,
    Rounded=Masked<WaylandSurfaceRenderElement<GlesRenderer>>,
    Backdrop=Masked<TextureShaderElement>,
}
/// Non-overlapping integer strips approximating a rounded rectangle at
/// pixel centers. The central band is merged, bounding work by 2*r+1.
fn rounded_regions(rect:Rectangle<i32,Logical>,radius:u32)->Vec<Rectangle<i32,Logical>>{
    if rect.size.w<=0||rect.size.h<=0{return Vec::new();}
    let r=(radius.min(32) as i32).min(rect.size.w/2).min(rect.size.h/2);
    if r==0{return vec![rect];}
    let mut out=Vec::with_capacity((2*r+1) as usize);
    for y in 0..r{
        let dy=f64::from(r-y)-0.5;
        let inset=(f64::from(r)-(f64::from(r*r)-dy*dy).sqrt()-0.5).ceil().max(0.0) as i32;
        let width=rect.size.w-2*inset;
        if width<=0{continue;}
        for row in [y,rect.size.h-1-y]{
            if let (Some(x),Some(y))=(rect.loc.x.checked_add(inset),rect.loc.y.checked_add(row)){
                out.push(Rectangle::new((x,y).into(),(width,1).into()));
            }
        }
    }
    if rect.size.h>2*r{
        if let Some(y)=rect.loc.y.checked_add(r){out.push(Rectangle::new((rect.loc.x,y).into(),(rect.size.w,rect.size.h-2*r).into()));}
    }
    out
}
pub(crate) fn rounded_contains(rect:Rectangle<i32,Logical>,radius:u32,p:Point<f64,Logical>)->bool{
    rounded_regions(rect,radius).iter().any(|r|r.to_f64().contains(p))
}
/// Subtract the disjoint inner strips from an outer shape, without duplicate
/// pixels. Used for the visible outline, not the client's content.
fn subtract_rect(rect:Rectangle<i32,Logical>,cut:Rectangle<i32,Logical>)->Vec<Rectangle<i32,Logical>>{
    let Some(i)=rect.intersection(cut)else{return vec![rect];};
    let x=rect.loc.x;let y=rect.loc.y;
    let right=x+rect.size.w;let bottom=y+rect.size.h;
    let ir=i.loc.x+i.size.w;let ib=i.loc.y+i.size.h;
    [(x,y,right,i.loc.y),(x,ib,right,bottom),(x,i.loc.y,i.loc.x,ib),(ir,i.loc.y,right,ib)].into_iter()
        .filter(|(l,t,r,b)|l<r&&t<b).map(|(l,t,r,b)|Rectangle::new((l,t).into(),(r-l,b-t).into())).collect()
}
fn rounded_border(rect:Rectangle<i32,Logical>,clip:Rectangle<i32,Logical>,radius:u32,width:u32)->Vec<Rectangle<i32,Logical>>{
    let w=width.min(8) as i32;if w==0{return Vec::new();}
    // Intersect with the known output before arithmetic on client geometry.
    let (Some(x),Some(y),Some(sw),Some(sh))=(rect.loc.x.checked_sub(w),rect.loc.y.checked_sub(w),rect.size.w.checked_add(2*w),rect.size.h.checked_add(2*w))else{return Vec::new();};
    if x.checked_add(sw).is_none()||y.checked_add(sh).is_none(){return Vec::new();}
    let expanded=Rectangle::new((x,y).into(),(sw,sh).into());
    let mut regions:Vec<_>=rounded_regions(expanded,radius+width).into_iter().filter_map(|r|r.intersection(clip)).collect();
    for inner in rounded_regions(rect,radius){
        regions=regions.into_iter().flat_map(|r|subtract_rect(r,inner)).collect();
    }
    regions
}
/// Limit draw damage to a shared rounded shape, never advertise pixels in
/// the clipped corners as opaque. Keep source mapping/transform intact.
pub struct Masked<E>{
    inner:E,
    regions:Vec<Rectangle<i32,Logical>>,
    scale:f64,
}
impl<E:Element> Element for Masked<E>{
    fn id(&self)->&Id{self.inner.id()}
    fn current_commit(&self)->CommitCounter{self.inner.current_commit()}
    fn src(&self)->Rectangle<f64,Buffer>{self.inner.src()}
    fn geometry(&self,scale:Scale<f64>)->Rectangle<i32,Physical>{self.inner.geometry(scale)}
    fn transform(&self)->Transform{self.inner.transform()}
    fn alpha(&self)->f32{self.inner.alpha()}
    fn kind(&self)->Kind{self.inner.kind()}
}
impl<E:RenderElement<GlesRenderer>> Masked<E>{
    /// Snapshot draws use a smaller physical output, while the logical
    /// rounded shape is unchanged. Transform the mask by the same scale
    /// as the scene, not by the rounded size of an individual surface.
    fn draw_scaled(&self,frame:&mut GlesFrame<'_,'_>,src:Rectangle<f64,Buffer>,dst:Rectangle<i32,Physical>,damage:&[Rectangle<i32,Physical>],factor:Scale<f64>)->Result<(),GlesError>{
        let mut clipped=Vec::new();
        let scale=Scale::from((self.scale*factor.x,self.scale*factor.y));
        for region in &self.regions{
            let mut region=region.to_physical_precise_round(scale);
            if region.size.w<=0||region.size.h<=0{continue;}
            region.loc-=dst.loc;
            for d in damage{if let Some(hit)=region.intersection(*d){clipped.push(hit);}}
        }
        if clipped.is_empty(){return Ok(());}
        self.inner.draw(frame,src,dst,&clipped,&[])
    }
}
impl<E:RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for Masked<E>{
    fn draw(&self,frame:&mut GlesFrame<'_,'_>,src:Rectangle<f64,Buffer>,dst:Rectangle<i32,Physical>,damage:&[Rectangle<i32,Physical>],_:&[Rectangle<i32,Physical>])->Result<(),GlesError>{
        self.draw_scaled(frame,src,dst,damage,Scale::from(1.0))
    }
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
    backdrop:backdrop::Backdrop,
}
impl SceneShaders{
    fn shadow(&mut self,renderer:&mut GlesRenderer)->Option<&GlesPixelProgram>{
        self.shadow.get_or_insert_with(||{
            match renderer.compile_custom_pixel_shader(include_str!("shadow.frag"),&[
                UniformName::new("outline",UniformType::_4f),
                UniformName::new("radius",UniformType::_1f),
            ]){
                Ok(program)=>{println!("Atelier smooth shadow shader ready");Some(program)},
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
#[derive(Clone)]
struct BlurRequest{
    target:BlurTarget,index:usize,geometry:Rectangle<i32,Logical>,radius:u32,strength:u32,
    explicit_regions:Option<Vec<Rectangle<i32,Logical>>>,
}
/// Only accept a small, additive, non-overlapping region. Never interpret
/// absent/default input regions as the full widget group. The namespace
/// merely opts into this rendering convention, not a privileged protocol.
fn widget_regions(rects:&[(RectangleKind,Rectangle<i32,Logical>)],origin:Point<i32,Logical>,geometry:Rectangle<i32,Logical>,bounds:Rectangle<i32,Logical>)->Option<Vec<Rectangle<i32,Logical>>>{
    if rects.is_empty()||rects.len()>256{return None;}
    let mut result=Vec::new();
    let clip=geometry.intersection(bounds)?;
    for (kind,rect) in rects{
        if !matches!(kind,RectangleKind::Add)||rect.size.w<=0||rect.size.h<=0{return None;}
        let x=rect.loc.x.checked_add(origin.x)?;let y=rect.loc.y.checked_add(origin.y)?;
        x.checked_add(rect.size.w)?;y.checked_add(rect.size.h)?;
        let rect=Rectangle::new((x,y).into(),rect.size);
        if let Some(hit)=rect.intersection(clip){
            if result.iter().any(|other:&Rectangle<i32,Logical>|other.overlaps(hit)){return None;}
            result.push(hit);
        }
    }
    (!result.is_empty()).then_some(result)
}
fn add_blur(requests:&mut Vec<BlurRequest>,request:BlurRequest){
    // One request per role (including duplicate client namespaces).
    // Since discovery is front-to-back, keep the frontmost eligible surface.
    if !requests.iter().any(|r|r.target==request.target){requests.push(request);}
}
pub fn elements(state:&Atelier,renderer:&mut GlesRenderer,shaders:&mut SceneShaders,color:[f32;4])->Result<Vec<SceneElement>>{
    let output=&state.output;
    let bounds=state.space.output_geometry(output).context("unmapped output")?;
    let scale=output.current_scale().fractional_scale();
    let map=layer_map_for_output(output);
    let mut result=Vec::new();
    let mut blurs=Vec::new();
    // Render layer roots at their actual arranged origin (bbox can include
    // negative subsurface offsets). Match input routing's layer ordering.
    let layers=|levels:&[Layer],result:&mut Vec<SceneElement>,renderer:&mut GlesRenderer,blurs:&mut Vec<BlurRequest>|{
        for &level in levels{
            for layer in map.layers_on(level).rev(){
                let Some(geometry)=map.layer_geometry(layer)else{continue;};
                let origin=geometry.loc-layer.bbox().loc-bounds.loc;
                let first=result.len();
                result.extend(layer.render_elements::<SceneElement>(renderer,origin.to_physical_precise_round(scale),Scale::from(scale),1.0));
                // Restrict blur to known shell namespaces and mapped content.
                // Namespace is a rendering hint, not an authorization claim.
                if result.len()>first&&geometry.overlaps(bounds){
                    if let Some((target,strength,radius))=state.shell_backdrop(layer.namespace()){
                        let explicit_regions=if target==BlurTarget::Widgets{
                            let surface_origin=geometry.loc-layer.bbox().loc;
                            let regions=with_states(layer.wl_surface(),|states|{
                                let mut cached=states.cached_state.get::<SurfaceAttributes>();
                                let current=cached.current();
                                current.input_region.as_ref().and_then(|region|widget_regions(&region.rects,surface_origin,geometry,bounds))
                            });
                            if regions.is_none(){continue;}
                            regions
                        }else{None};
                        add_blur(blurs,BlurRequest{target,index:result.len(),geometry,radius,strength,explicit_regions});
                    }
                }
            }
        }
    };
    layers(&[Layer::Overlay,Layer::Top],&mut result,renderer,&mut blurs);
    let clip=map.non_exclusive_zone();
    for window in state.space.elements().rev(){
        let Some(geometry)=state.space.element_geometry(window)else{continue;};
        let origin=geometry.loc-window.geometry().loc-bounds.loc;
        let radius=state.window_corner_radius(window);
        let alpha=state.window_alpha(window);
        if radius>0{
            if let Some(toplevel)=window.toplevel(){
                let surface=toplevel.wl_surface();
                let location=origin.to_physical_precise_round(scale);
                // Reuse Smithay's popup placement, but mask only the root
                // surface tree; menus outside the parent remain visible.
                for (popup,offset) in PopupManager::popups_for_surface(surface){
                    let offset=(window.geometry().loc+offset-popup.geometry().loc).to_physical_precise_round(scale);
                    result.extend(render_elements_from_surface_tree::<GlesRenderer,SceneElement>(renderer,popup.wl_surface(),location+offset,Scale::from(scale),alpha,Kind::Unspecified));
                }
                let regions:Vec<_>=rounded_regions(geometry,radius).into_iter().map(|mut r|{r.loc-=bounds.loc;r}).collect();
                let surfaces=render_elements_from_surface_tree::<GlesRenderer,WaylandSurfaceRenderElement<GlesRenderer>>(renderer,surface,location,Scale::from(scale),alpha,Kind::Unspecified);
                for inner in surfaces{result.push(SceneElement::from(Masked{inner,regions:regions.clone(),scale}));}
            }
        }else{
            result.extend(window.render_elements::<SceneElement>(renderer,origin.to_physical_precise_round(scale),Scale::from(scale),alpha));
        }
        if let Some((border,focused))=state.window_border(window){
            let color=border.color(focused)?;
            let strips=if radius>0{rounded_border(geometry,clip,radius,border.width)}else{strips(geometry,clip,border.width)};
            for strip in strips{
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
        let strength=state.window_backdrop_blur(window);
        if strength>0&&geometry.overlaps(bounds){
            add_blur(&mut blurs,BlurRequest{target:BlurTarget::Focused,index:result.len(),geometry,radius,strength,explicit_regions:None});
        }
    }
    layers(&[Layer::Bottom,Layer::Background],&mut result,renderer,&mut blurs);
    // Prepare from the ORIGINAL unblurred scene. Each pass gets a distinct
    // scratch texture; no feedback, overwritten snapshots or recursive blur.
    let mut insertions=Vec::new();
    let mut active=[false;BlurTarget::COUNT];
    if scale==1.0{
        let size=bounds.size.to_physical(1);
        for request in blurs{
            if let Some(inner)=shaders.backdrop.render(renderer,size,color,&result[request.index..],request.strength,request.target){
                active[request.target.slot()]=true;
                let regions=request.explicit_regions.unwrap_or_else(||rounded_regions(request.geometry,request.radius)).into_iter().filter_map(|r|r.intersection(bounds))
                    .map(|mut r|{r.loc-=bounds.loc;r}).collect();
                insertions.push((request.index,SceneElement::from(Masked{inner,regions,scale})));
            }
        }
    }
    shaders.backdrop.retain(active);
    // Reverse insertion preserves the original indices for all requests.
    for (index,element) in insertions.into_iter().rev(){result.insert(index,element);}
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
    fn rounded_clip_and_border_are_disjoint_and_hit_test_matches(){
        let rect=Rectangle::new((10,10).into(),(80,60).into());let clip=Rectangle::new((0,0).into(),(120,100).into());
        let regions=rounded_regions(rect,16);assert!(regions.len()<=33);
        assert!(!rounded_contains(rect,16,(10.5,10.5).into()));
        assert!(rounded_contains(rect,16,(50.0,10.5).into()));
        assert!(rounded_contains(rect,16,(50.0,40.0).into()));
        assert!(rounded_contains(rect,0,(10.5,10.5).into()));
        let border=rounded_border(rect,clip,16,3);
        for (i,r) in regions.iter().enumerate(){assert!(regions[i+1..].iter().all(|o|!r.overlaps(*o)));assert!(border.iter().all(|b|!r.overlaps(*b)));}
        for (i,r) in border.iter().enumerate(){assert_eq!(r.intersection(clip),Some(*r));assert!(border[i+1..].iter().all(|b|!r.overlaps(*b)));}
        let tiny=Rectangle::new((0,0).into(),(1,1).into());assert_eq!(rounded_regions(tiny,24),vec![tiny]);
    }
    #[test]
    fn widget_blur_regions_preserve_gaps_and_reject_complex_input(){
        let geometry=Rectangle::new((10,20).into(),(100,100).into());
        let bounds=Rectangle::new((0,0).into(),(200,200).into());
        let first=Rectangle::new((0,0).into(),(100,40).into());
        let second=Rectangle::new((0,50).into(),(100,40).into());
        let rects=[(RectangleKind::Add,first),(RectangleKind::Add,second)];
        let regions=widget_regions(&rects,geometry.loc,geometry,bounds).unwrap();
        assert_eq!(regions.len(),2);
        assert!(!regions.iter().any(|r|r.contains((50,65).into())));
        assert!(widget_regions(&[],geometry.loc,geometry,bounds).is_none());
        assert!(widget_regions(&[(RectangleKind::Subtract,first)],geometry.loc,geometry,bounds).is_none());
        assert!(widget_regions(&[(RectangleKind::Add,first),(RectangleKind::Add,first)],geometry.loc,geometry,bounds).is_none());
    }
    #[test]
    fn blur_requests_are_bounded_and_preserve_scene_indices(){
        let geometry=Rectangle::new((0,0).into(),(50,40).into());
        let mut requests=Vec::new();
        for (target,index) in [(BlurTarget::Panel,2),(BlurTarget::Dock,4),(BlurTarget::Panel,6),(BlurTarget::Focused,9)]{
            add_blur(&mut requests,BlurRequest{target,index,geometry,radius:0,strength:8,explicit_regions:None});
        }
        assert_eq!(requests.iter().map(|r|r.index).collect::<Vec<_>>(),vec![2,4,9]);
        let mut scene=vec![0;10];for request in requests.into_iter().rev(){scene.insert(request.index,request.index);}
        assert_eq!(scene.iter().filter(|&&i|i!=0).copied().collect::<Vec<_>>(),vec![2,4,9]);
    }
    #[test]
    fn tiny_tiles_retain_positive_content(){
        assert_eq!(inset_tile(Tile{x:0,y:0,width:5,height:3},8),Tile{x:1,y:1,width:3,height:1});
        assert_eq!(inset_tile(Tile{x:0,y:0,width:1,height:1},8),Tile{x:0,y:0,width:1,height:1});
    }
}
