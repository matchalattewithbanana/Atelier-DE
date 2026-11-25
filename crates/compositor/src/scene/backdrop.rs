//! Bounded backdrop passes, separate renderer-owned scratch textures for
//! focused window, panel, dock and widget group. No recursive blur or CPU readback.
use super::SceneElement;
use smithay::{
    backend::{allocator::Fourcc,renderer::{
        Bind,Offscreen,Renderer,Frame,Color32F,
        element::{Id,Kind,Element,RenderElement,texture::TextureRenderElement,utils::RescaleRenderElement},
        gles::{GlesRenderer,GlesFrame,GlesError,GlesTexture,GlesTexProgram,Uniform,UniformName,UniformType,UniformValue,element::TextureShaderElement},
        utils::{draw_render_elements,CommitCounter},
    }},
    utils::{Buffer,Logical,Physical,Rectangle,Scale,Size,Transform},
};
const MAX_PIXELS:i64=4_194_304;
const MAX_OUTPUT_EDGE:i32=8192;
/// Render at half resolution on each axis, rounded up for odd dimensions.
/// Preserve exact source/destination ratios, and cap the allocation itself.
#[derive(Clone,Copy,Debug)]
struct SnapshotPlan{
    output:Size<i32,Physical>,
    scratch:Size<i32,Physical>,
    factor:Scale<f64>,
}
impl SnapshotPlan{
    fn new(output:Size<i32,Physical>)->Option<Self>{
        if !(1..=MAX_OUTPUT_EDGE).contains(&output.w)||!(1..=MAX_OUTPUT_EDGE).contains(&output.h){return None;}
        let scratch:Size<i32,Physical>=((output.w+1)/2,(output.h+1)/2).into();
        if i64::from(scratch.w)*i64::from(scratch.h)>MAX_PIXELS{return None;}
        let factor=Scale::from((f64::from(scratch.w)/f64::from(output.w),f64::from(scratch.h)/f64::from(output.h)));
        Some(Self{output,scratch,factor})
    }

    /// Repaint the bounding box of all visible destination regions, including
    /// the furthest shader tap and a conservative scratch-space guard for
    /// linear filtering, rounded geometry and mediump texture coordinates.
    /// Keep the texture and projection full-sized: stale pixels outside
    /// this rectangle are never intentionally sampled.
    fn damage(&self,regions:&[Rectangle<i32,Logical>],strength:u32)->Option<Rectangle<i32,Physical>>{
        let mut left=i64::from(self.output.w);
        let mut top=i64::from(self.output.h);
        let mut right=0i64;
        let mut bottom=0i64;
        let mut visible=false;
        let ow=i64::from(self.output.w);let oh=i64::from(self.output.h);
        for rect in regions{
            if rect.size.w<=0||rect.size.h<=0{continue;}
            let x=i64::from(rect.loc.x);let y=i64::from(rect.loc.y);
            let l=x.clamp(0,ow);let t=y.clamp(0,oh);
            let r=(x+i64::from(rect.size.w)).clamp(0,ow);
            let b=(y+i64::from(rect.size.h)).clamp(0,oh);
            if l>=r||t>=b{continue;}
            visible=true;
            left=left.min(l);top=top.min(t);right=right.max(r);bottom=bottom.max(b);
        }
        if !visible{return None;}
        let radius=i64::from(strength.min(16));
        // Shader taps extend by at most radius output pixels on each axis.
        // Integer floor/ceil retains the exact odd-output snapshot scale.
        // Guard is deliberately wider than the bilinear footprint and
        // nominal mediump coordinate error at the bounded texture sizes.
        const GUARD:i64=16;
        let sw=i64::from(self.scratch.w);let sh=i64::from(self.scratch.h);
        let l=(((left-radius).max(0)*sw)/ow-GUARD).max(0);
        let t=(((top-radius).max(0)*sh)/oh-GUARD).max(0);
        let r=((((right+radius).min(ow)*sw+ow-1)/ow)+GUARD).min(sw);
        let b=((((bottom+radius).min(oh)*sh+oh-1)/oh)+GUARD).min(sh);
        Some(Rectangle::new((l as i32,t as i32).into(),((r-l) as i32,(b-t) as i32).into()))
    }
}
/// Scale geometry and source mapping through Smithay. Opaque regions are
/// deliberately not advertised: fractional edge rounding must not hide
/// lower-scene pixels. For rounded windows, also scale the damage mask.
struct SnapshotElement<'a>{
    source:&'a SceneElement,
    scaled:RescaleRenderElement<&'a SceneElement>,
    factor:Scale<f64>,
}
impl<'a> SnapshotElement<'a>{
    fn new(source:&'a SceneElement,factor:Scale<f64>)->Self{
        Self{source,scaled:RescaleRenderElement::from_element(source,(0,0).into(),factor),factor}
    }
}
impl Element for SnapshotElement<'_>{
    fn id(&self)->&Id{self.scaled.id()}
    fn current_commit(&self)->CommitCounter{self.scaled.current_commit()}
    fn src(&self)->Rectangle<f64,Buffer>{self.scaled.src()}
    fn geometry(&self,scale:Scale<f64>)->Rectangle<i32,Physical>{self.scaled.geometry(scale)}
    fn transform(&self)->Transform{self.scaled.transform()}
    fn alpha(&self)->f32{self.scaled.alpha()}
    fn kind(&self)->Kind{self.scaled.kind()}
}
impl RenderElement<GlesRenderer> for SnapshotElement<'_>{
    fn draw(&self,frame:&mut GlesFrame<'_,'_>,src:Rectangle<f64,Buffer>,dst:Rectangle<i32,Physical>,damage:&[Rectangle<i32,Physical>],_:&[Rectangle<i32,Physical>])->Result<(),GlesError>{
        if dst.size.w<=0||dst.size.h<=0{return Ok(());}
        match self.source{
            SceneElement::Rounded(masked)=>masked.draw_scaled(frame,src,dst,damage,self.factor),
            SceneElement::Backdrop(masked)=>masked.draw_scaled(frame,src,dst,damage,self.factor),
            _=>self.scaled.draw(frame,src,dst,damage,&[]),
        }
    }
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub(crate) enum BlurTarget{Focused,Panel,Dock,Widgets}
impl BlurTarget{
    pub const COUNT:usize=4;
    pub fn slot(self)->usize{match self{Self::Focused=>0,Self::Panel=>1,Self::Dock=>2,Self::Widgets=>3}}
    fn name(self)->&'static str{match self{Self::Focused=>"focused",Self::Panel=>"panel",Self::Dock=>"dock",Self::Widgets=>"widgets"}}
}
#[derive(Default)]
pub(super) struct Backdrop{
    program:Option<GlesTexProgram>,
    scratch:[Option<(Size<i32,Physical>,GlesTexture)>;BlurTarget::COUNT],
    disabled:bool,
    logged:[bool;BlurTarget::COUNT],
}
impl Backdrop{
    pub fn retain(&mut self,active:[bool;BlurTarget::COUNT]){
        for (slot,keep) in self.scratch.iter_mut().zip(active){if !keep{*slot=None;}}
    }
    pub fn render(&mut self,renderer:&mut GlesRenderer,size:Size<i32,Physical>,color:[f32;4],behind:&[SceneElement],regions:&[Rectangle<i32,Logical>],strength:u32,target:BlurTarget)->Option<TextureShaderElement>{
        if self.disabled||strength==0{return None;}
        let plan=SnapshotPlan::new(size)?;
        let damage=plan.damage(regions,strength)?;
        match self.try_render(renderer,plan,damage,color,behind,strength,target){
            Ok(element)=>{
                if !self.logged[target.slot()]{println!("Atelier {} backdrop rendered",target.name());self.logged[target.slot()]=true;}
                Some(element)
            }
            Err(error)=>{
                tracing::warn!(?error,?target,"backdrop unavailable; retaining plain transparency");
                self.disabled=true;self.retain([false;BlurTarget::COUNT]);None
            }
        }
    }
    fn try_render(&mut self,renderer:&mut GlesRenderer,plan:SnapshotPlan,damage:Rectangle<i32,Physical>,color:[f32;4],behind:&[SceneElement],strength:u32,target:BlurTarget)->anyhow::Result<TextureShaderElement>{
        if self.program.is_none(){
            self.program=Some(renderer.compile_custom_texture_shader(include_str!("backdrop.frag"),&[UniformName::new("step_uv",UniformType::_2f)])?);
        }
        let size=plan.scratch;
        let slot=&mut self.scratch[target.slot()];
        if slot.as_ref().is_none_or(|(current,_)|*current!=size){
            *slot=None;
            let texture:GlesTexture=renderer.create_buffer(Fourcc::Abgr8888,(size.w,size.h).into())?;
            *slot=Some((size,texture));
        }
        let (_,texture)=slot.as_mut().expect("scratch allocated");
        {
            let mut target=renderer.bind(texture)?;
            // Normal orientation for a texture: y=0 corresponds to its
            // first row. Final presentation uses each backend's transform.
            let mut frame=renderer.render(&mut target,size,Transform::Normal)?;
            frame.clear(Color32F::new(color[0],color[1],color[2],1.0),&[damage])?;
            let scaled:Vec<_>=behind.iter().map(|element|SnapshotElement::new(element,plan.factor))
                .filter(|element|element.geometry(Scale::from(1.0)).overlaps(damage)).collect();
            draw_render_elements(&mut frame,1.0,&scaled,&[damage])?;
            let _fence=frame.finish()?;
        }
        // Same renderer/context, ordered GL commands. The scratch texture
        // is never bound as the destination of its own sampling pass.
        let element=TextureRenderElement::from_static_texture(
            Id::new(),renderer.context_id(),(0.0,0.0),texture.clone(),1,Transform::Normal,
            Some(1.0),Some(Rectangle::from_size((f64::from(size.w),f64::from(size.h)).into())),
            Some((plan.output.w,plan.output.h).into()),None,Kind::Unspecified,
        );
        let radius=strength.min(16) as f32;
        Ok(TextureShaderElement::new(element,self.program.as_ref().expect("program compiled").clone(),
            vec![Uniform::new("step_uv",UniformValue::_2f(radius/plan.output.w as f32,radius/plan.output.h as f32))]))
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn distinct_slots_for_each_role(){
        assert_eq!([BlurTarget::Focused.slot(),BlurTarget::Panel.slot(),BlurTarget::Dock.slot(),BlurTarget::Widgets.slot()],[0,1,2,3]);
        assert_eq!(MAX_PIXELS*4*BlurTarget::COUNT as i64,64*1024*1024);
    }
    #[test]
    fn damage_bounds_panel_and_dock_without_changing_projection(){
        let plan=SnapshotPlan::new((1920,1080).into()).unwrap();
        let panel=Rectangle::new((0,0).into(),(1920,40).into());
        let d=plan.damage(&[panel],16).unwrap();
        assert_eq!(d,Rectangle::new((0,0).into(),(960,44).into()));
        let dock=Rectangle::new((760,1000).into(),(400,80).into());
        let d=plan.damage(&[dock],16).unwrap();
        assert_eq!(d,Rectangle::new((356,476).into(),(248,64).into()));
        assert_eq!(plan.damage(&[Rectangle::from_size((1920,1080).into())],16),
            Some(Rectangle::from_size(plan.scratch)));
    }
    #[test]
    fn damage_covers_odd_output_and_disjoint_regions(){
        let plan=SnapshotPlan::new((1919,1079).into()).unwrap();
        let regions=[Rectangle::new((101,103).into(),(19,25).into()),
            Rectangle::new((200,240).into(),(30,50).into())];
        let d=plan.damage(&regions,16).unwrap();
        for r in regions{
            // Verify conservative coverage of the entire farthest-tap
            // rectangle under the exact anisotropic snapshot ratio.
            let left=(f64::from(r.loc.x-16)*plan.factor.x).floor() as i32;
            let top=(f64::from(r.loc.y-16)*plan.factor.y).floor() as i32;
            let right=(f64::from(r.loc.x+r.size.w+16)*plan.factor.x).ceil() as i32;
            let bottom=(f64::from(r.loc.y+r.size.h+16)*plan.factor.y).ceil() as i32;
            let required=Rectangle::new((left,top).into(),(right-left,bottom-top).into());
            assert_eq!(required.intersection(d),Some(required));
        }
        assert!(plan.damage(&[],8).is_none());
        assert!(plan.damage(&[Rectangle::new((-100,-100).into(),(5,5).into())],8).is_none());
        let edge=Rectangle::new((1918,1078).into(),(1,1).into());
        let d=plan.damage(&[edge],16).unwrap();
        assert_eq!(d.loc.x+d.size.w,960);assert_eq!(d.loc.y+d.size.h,540);
    }
    #[test]
    fn limits_scratch_allocation(){
        let hd=SnapshotPlan::new((1920,1080).into()).unwrap();
        assert_eq!(hd.scratch,(960,540).into());
        let uhd=SnapshotPlan::new((3840,2160).into()).unwrap();
        assert_eq!(uhd.scratch,(1920,1080).into());
        assert!(SnapshotPlan::new((5120,2880).into()).is_some());
        assert!(SnapshotPlan::new((7680,4320).into()).is_none());
        assert!(SnapshotPlan::new((0,100).into()).is_none());
        assert!(SnapshotPlan::new((i32::MAX,i32::MAX).into()).is_none());
        let odd=SnapshotPlan::new((1919,1079).into()).unwrap();
        assert_eq!(odd.scratch,(960,540).into());
        assert!((odd.output.w as f64*odd.factor.x-960.0).abs()<1e-9);
        assert!((odd.output.h as f64*odd.factor.y-540.0).abs()<1e-9);
    }
}
