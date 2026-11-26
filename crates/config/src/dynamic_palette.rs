//! HCT/CAM16 generation for both shell and access-theme drafts.
//! Keep the third-party scheme API and legacy palette projection here.
use anyhow::{bail,Result};
use material_color_utilities_rust::{DynamicScheme,Hct};
use crate::{access_theme::Palette,design::Color,themes::ShellPalette};

#[derive(Debug,Default,Clone,Copy,PartialEq,Eq)]
pub enum Variant{#[default] TonalSpot,Expressive,Vibrant,Monochrome}
impl Variant{
    pub const OPTIONS:&'static [(&'static str,&'static str)]=&[
        ("tonal-spot","Tonal spot"),("expressive","Expressive"),("vibrant","Vibrant"),("monochrome","Monochrome"),
    ];
    pub fn parse(value:&str)->Result<Self>{
        Ok(match value{
            "tonal-spot"=>Self::TonalSpot,"expressive"=>Self::Expressive,"vibrant"=>Self::Vibrant,"monochrome"=>Self::Monochrome,
            _=>bail!("unknown Material palette variant"),
        })
    }
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum Contrast{Normal,High}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct Options{pub variant:Variant,pub light:bool,pub contrast:Contrast}

fn hex(argb:u32)->String{format!("#{:06x}",argb&0x00ff_ffff)}
/// Both legacy surfaces share one foreground. Prefer the Material role, but
/// repair any mismatch introduced by projecting onto the existing schema.
fn foreground(preferred:u32,backgrounds:&[Color],minimum:f64)->Result<String>{
    for candidate in [hex(preferred),"#000000".into(),"#ffffff".into()]{
        let color=Color::parse(&candidate)?;
        if backgrounds.iter().all(|bg|color.contrast(*bg)>=minimum){return Ok(candidate);}
    }
    bail!("generated surfaces cannot share a readable foreground")
}
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct Generated{
    pub access:Palette,
    pub outline:String,
    // These retain the actual scheme colors for future semantic-role-aware
    // renderers. Legacy exports intentionally store only their own fields.
    pub secondary:String,pub tertiary:String,
}
impl Generated{
    pub fn shell(&self)->ShellPalette{
        ShellPalette{surface:self.access.background.clone(),elevated:self.access.panel.clone(),text:self.access.text.clone(),muted:self.access.muted.clone()}
    }
}
pub fn generate(seed:[u8;3],options:Options)->Result<Generated>{
    let source=0xff00_0000|(u32::from(seed[0])<<16)|(u32::from(seed[1])<<8)|u32::from(seed[2]);
    let hct=Hct::from_int(source);
    let contrast=if options.contrast==Contrast::High{1.0}else{0.0};
    let dark=!options.light;
    let scheme=match options.variant{
        Variant::TonalSpot=>DynamicScheme::tonal_spot(hct,dark,contrast),
        Variant::Expressive=>DynamicScheme::expressive(hct,dark,contrast),
        Variant::Vibrant=>DynamicScheme::vibrant(hct,dark,contrast),
        Variant::Monochrome=>DynamicScheme::monochrome(hct,dark,contrast),
    };
    let background=hex(scheme.surface());
    let panel=hex(scheme.surface_container());
    let backgrounds=[Color::parse(&background)?,Color::parse(&panel)?];
    let minimum=if options.contrast==Contrast::High{7.0}else{4.5};
    let text=foreground(scheme.on_surface(),&backgrounds,minimum)?;
    let muted=foreground(scheme.on_surface_variant(),&backgrounds,minimum)?;
    let accent=foreground(scheme.primary(),&backgrounds,minimum)?;
    Ok(Generated{access:Palette{background,panel,text,muted,accent},outline:hex(scheme.outline()),secondary:hex(scheme.secondary()),tertiary:hex(scheme.tertiary())})
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn schemes_project_into_both_existing_theme_formats(){
        for seed in [[0,0,0],[255,255,255],[255,0,0],[0,255,0],[0,0,255],[103,80,164]]{
            for (id,_) in Variant::OPTIONS{
                for light in [false,true]{
                    for contrast in [Contrast::Normal,Contrast::High]{
                        let options=Options{variant:Variant::parse(id).unwrap(),light,contrast};
                        let generated=generate(seed,options).unwrap();
                        assert_eq!(generated,generate(seed,options).unwrap());
                        generated.shell().validate().unwrap();
                        let mut theme=crate::access_theme::preset("classic").unwrap();theme.palette=generated.access;
                        theme.validate().unwrap();
                        let min=if contrast==Contrast::High{7.0}else{4.5};
                        for bg in [&theme.palette.background,&theme.palette.panel]{
                            for fg in [&theme.palette.text,&theme.palette.muted,&theme.palette.accent]{
                                assert!(Color::parse(bg).unwrap().contrast(Color::parse(fg).unwrap())>=min);
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn generation_changes_with_seed_and_variant(){
        let options=Options{variant:Variant::TonalSpot,light:false,contrast:Contrast::Normal};
        let blue=generate([40,90,220],options).unwrap();
        assert_ne!(blue,generate([220,50,80],options).unwrap());
        assert_ne!(blue,generate([40,90,220],Options{variant:Variant::Expressive,..options}).unwrap());
        assert!(Variant::parse("anything").is_err());
    }
}
