//! Bounded backdrop passes, separate renderer-owned scratch textures for
//! focused window, panel and dock. No recursive blur or CPU readback.
use super::SceneElement;
use smithay::{
    backend::{allocator::Fourcc,renderer::{
        Bind,Offscreen,Renderer,Frame,Color32F,
        element::{Id,Kind,texture::TextureRenderElement},
        gles::{GlesRenderer,GlesTexture,GlesTexProgram,Uniform,UniformName,UniformType,UniformValue,element::TextureShaderElement},
        utils::draw_render_elements,
    }},
    utils::{Physical,Rectangle,Size,Transform},
};
const MAX_PIXELS:i64=4_194_304;
fn supported(size:Size<i32,Physical>)->bool{
    size.w>0&&size.h>0&&i64::from(size.w)*i64::from(size.h)<=MAX_PIXELS
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub(crate) enum BlurTarget{Focused,Panel,Dock}
impl BlurTarget{
    pub const COUNT:usize=3;
    pub fn slot(self)->usize{match self{Self::Focused=>0,Self::Panel=>1,Self::Dock=>2}}
    fn name(self)->&'static str{match self{Self::Focused=>"focused",Self::Panel=>"panel",Self::Dock=>"dock"}}
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
    pub fn render(&mut self,renderer:&mut GlesRenderer,size:Size<i32,Physical>,color:[f32;4],behind:&[SceneElement],strength:u32,target:BlurTarget)->Option<TextureShaderElement>{
        if self.disabled||strength==0||!supported(size){return None;}
        match self.try_render(renderer,size,color,behind,strength,target){
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
    fn try_render(&mut self,renderer:&mut GlesRenderer,size:Size<i32,Physical>,color:[f32;4],behind:&[SceneElement],strength:u32,target:BlurTarget)->anyhow::Result<TextureShaderElement>{
        if self.program.is_none(){
            self.program=Some(renderer.compile_custom_texture_shader(include_str!("backdrop.frag"),&[UniformName::new("step_uv",UniformType::_2f)])?);
        }
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
            let damage=Rectangle::from_size(size);
            frame.clear(Color32F::new(color[0],color[1],color[2],1.0),&[damage])?;
            draw_render_elements(&mut frame,1.0,behind,&[damage])?;
            let _fence=frame.finish()?;
        }
        // Same renderer/context, ordered GL commands. The scratch texture
        // is never bound as the destination of its own sampling pass.
        let element=TextureRenderElement::from_static_texture(
            Id::new(),renderer.context_id(),(0.0,0.0),texture.clone(),1,Transform::Normal,
            Some(1.0),None,Some((size.w,size.h).into()),None,Kind::Unspecified,
        );
        let radius=strength.min(16) as f32;
        Ok(TextureShaderElement::new(element,self.program.as_ref().expect("program compiled").clone(),
            vec![Uniform::new("step_uv",UniformValue::_2f(radius/size.w as f32,radius/size.h as f32))]))
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn distinct_slots_for_each_role(){
        assert_eq!([BlurTarget::Focused.slot(),BlurTarget::Panel.slot(),BlurTarget::Dock.slot()],[0,1,2]);
        assert_eq!(MAX_PIXELS*4*BlurTarget::COUNT as i64,48*1024*1024);
    }
    #[test]
    fn limits_scratch_allocation(){
        assert!(supported((2560,1440).into()));assert!(!supported((3840,2160).into()));
        assert!(!supported((0,100).into()));assert!(!supported((i32::MAX,i32::MAX).into()));
    }
}
