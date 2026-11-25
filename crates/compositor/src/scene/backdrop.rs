//! First bounded backdrop pass. Renderer-owned scratch texture, one focused
//! window, no recursive blurring, no CPU readback.
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
#[derive(Default)]
pub(super) struct Backdrop{
    program:Option<GlesTexProgram>,
    scratch:Option<(Size<i32,Physical>,GlesTexture)>,
    disabled:bool,
    logged:bool,
}
impl Backdrop{
    pub fn release(&mut self){self.scratch=None;}
    pub fn render(&mut self,renderer:&mut GlesRenderer,size:Size<i32,Physical>,color:[f32;4],behind:&[SceneElement],strength:u32)->Option<TextureShaderElement>{
        if self.disabled||strength==0||!supported(size){self.release();return None;}
        match self.try_render(renderer,size,color,behind,strength){
            Ok(element)=>{
                if !self.logged{println!("Atelier focused backdrop rendered");self.logged=true;}
                Some(element)
            }
            Err(error)=>{
                tracing::warn!(?error,"focused backdrop unavailable; retaining plain transparency");
                self.disabled=true;self.release();None
            }
        }
    }
    fn try_render(&mut self,renderer:&mut GlesRenderer,size:Size<i32,Physical>,color:[f32;4],behind:&[SceneElement],strength:u32)->anyhow::Result<TextureShaderElement>{
        if self.program.is_none(){
            self.program=Some(renderer.compile_custom_texture_shader(include_str!("backdrop.frag"),&[UniformName::new("step_uv",UniformType::_2f)])?);
        }
        if self.scratch.as_ref().is_none_or(|(current,_)|*current!=size){
            self.release();
            let texture:GlesTexture=renderer.create_buffer(Fourcc::Abgr8888,(size.w,size.h).into())?;
            self.scratch=Some((size,texture));
        }
        let (_,texture)=self.scratch.as_mut().expect("scratch allocated");
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
    fn limits_scratch_allocation(){
        assert!(supported((2560,1440).into()));assert!(!supported((3840,2160).into()));
        assert!(!supported((0,100).into()));assert!(!supported((i32::MAX,i32::MAX).into()));
    }
}
