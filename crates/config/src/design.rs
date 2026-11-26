//! Shared, toolkit-independent Atelier Classic design vocabulary.
//! Material 3 token values are separated from Atelier's custom RGB role
//! derivation; this module does not claim to implement Material HCT.
use anyhow::Result;
use crate::themes::{rgb,contrast};

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum Corner{None,ExtraSmall,Small,Medium,Large,LargeIncreased,ExtraLarge,ExtraLargeIncreased,ExtraExtraLarge,Full}
impl Corner{
    /// Material logical units, resolved to the actual component dimensions.
    pub fn radius(self,width:f64,height:f64)->f64{
        let limit=width.min(height).max(0.0)/2.0;
        let value=match self{Self::None=>0.0,Self::ExtraSmall=>4.0,Self::Small=>8.0,Self::Medium=>12.0,Self::Large=>16.0,Self::LargeIncreased=>20.0,Self::ExtraLarge=>28.0,Self::ExtraLargeIncreased=>32.0,Self::ExtraExtraLarge=>48.0,Self::Full=>limit};
        value.min(limit)
    }
}
pub fn nested_radius(outer:f64,padding:f64)->f64{(outer-padding.max(0.0)).max(0.0)}

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum TypeRole{DisplayLarge,DisplayMedium,DisplaySmall,HeadlineLarge,HeadlineMedium,HeadlineSmall,TitleLarge,TitleMedium,TitleSmall,BodyLarge,BodyMedium,BodySmall,LabelLarge,LabelMedium,LabelSmall}
#[derive(Debug,Clone,Copy,PartialEq)]
pub struct TypeStyle{pub size:f64,pub line_height:f64,pub tracking:f64,pub weight:u16}
impl TypeRole{
    pub fn style(self,emphasized:bool)->TypeStyle{
        let (size,line_height,tracking,weight,em_tracking,em_weight)=match self{
            Self::DisplayLarge=>(57.,64.,-0.2,400,0.,500),
            Self::DisplayMedium=>(45.,52.,0.,400,0.,500),
            Self::DisplaySmall=>(36.,44.,0.,400,0.,500),
            Self::HeadlineLarge=>(32.,40.,0.,400,0.,500),
            Self::HeadlineMedium=>(28.,36.,0.,400,0.,500),
            Self::HeadlineSmall=>(24.,32.,0.,400,0.,500),
            Self::TitleLarge=>(22.,28.,0.,400,0.,500),
            Self::TitleMedium=>(16.,24.,0.2,500,0.15,700),
            Self::TitleSmall=>(14.,20.,0.1,500,0.1,700),
            Self::BodyLarge=>(16.,24.,0.5,400,0.15,500),
            Self::BodyMedium=>(14.,20.,0.2,400,0.25,500),
            Self::BodySmall=>(12.,16.,0.4,400,0.4,500),
            Self::LabelLarge=>(14.,20.,0.1,500,0.1,700),
            Self::LabelMedium=>(12.,16.,0.5,500,0.5,700),
            Self::LabelSmall=>(11.,16.,0.5,500,0.5,700),
        };
        TypeStyle{size,line_height,tracking:if emphasized{em_tracking}else{tracking},weight:if emphasized{em_weight}else{weight}}
    }
}

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum Scheme{Expressive,Standard}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum Speed{Fast,Default,Slow}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum MotionKind{Spatial,Effects}
#[derive(Debug,Clone,Copy,PartialEq)]
pub struct Spring{damping:f64,stiffness:f64}
impl Scheme{
    /// Compose phone spring tokens, adopted as an Atelier desktop starting
    /// point rather than a published GTK-specific Material specification.
    pub fn spring(self,speed:Speed,kind:MotionKind)->Spring{
        let (damping,stiffness)=match (self,speed,kind){
            (_,Speed::Fast,MotionKind::Effects)=>(1.0,3800.0),
            (_,Speed::Default,MotionKind::Effects)=>(1.0,1600.0),
            (_,Speed::Slow,MotionKind::Effects)=>(1.0,800.0),
            (Self::Expressive,Speed::Fast,MotionKind::Spatial)=>(0.6,800.0),
            (Self::Expressive,Speed::Default,MotionKind::Spatial)=>(0.8,380.0),
            (Self::Expressive,Speed::Slow,MotionKind::Spatial)=>(0.8,200.0),
            (Self::Standard,Speed::Fast,MotionKind::Spatial)=>(0.9,1400.0),
            (Self::Standard,Speed::Default,MotionKind::Spatial)=>(0.9,700.0),
            (Self::Standard,Speed::Slow,MotionKind::Spatial)=>(0.9,300.0),
        };
        Spring{damping,stiffness}
    }
}
impl Spring{
    /// Analytic unit-mass response from rest. Spatial values may overshoot;
    /// effects never do. For a new transition, sample with elapsed seconds.
    /// This is not an interruptible/velocity-preserving animation controller.
    pub fn progress(self,seconds:f64,reduced_motion:bool)->f64{
        if reduced_motion{return 1.0;}
        if seconds.is_nan()||seconds<=0.0{return 0.0;}
        if seconds>=4.0{return 1.0;}
        let omega=self.stiffness.sqrt();
        let z=self.damping;
        let decay=(-z*omega*seconds).exp();
        if z==1.0{return (1.0-decay*(1.0+omega*seconds)).clamp(0.0,1.0);}
        let q=(1.0-z*z).sqrt();
        let angle=omega*q*seconds;
        1.0-decay*(angle.cos()+z/q*angle.sin())
    }
}

/// A validated RGB color. Semantic role pairs are explicit so callers do
/// not accidentally put a container color on text or reuse a fixed accent.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct Color([u8;3]);
impl Color{
    pub fn parse(value:&str)->Result<Self>{Ok(Self(rgb(value)?))}
    pub fn rgb(self)->[u8;3]{self.0}
    pub fn hex(self)->String{format!("#{:02x}{:02x}{:02x}",self.0[0],self.0[1],self.0[2])}
    pub fn contrast(self,other:Self)->f64{contrast(self.0,other.0)}
    fn blend(self,other:Self,t:f64)->Self{
        Self(std::array::from_fn(|i|(f64::from(self.0[i])*(1.0-t)+f64::from(other.0[i])*t).round() as u8))
    }
    /// Highest-contrast black or white foreground for this fill.
    pub fn contrasting_text(self)->Self{
        let black=Self([0;3]);let white=Self([255;3]);
        if black.contrast(self)>=white.contrast(self){black}else{white}
    }
    fn readable_on(self,background:Self)->Self{
        if self.contrast(background)>=4.5{self}else{background.contrasting_text()}
    }
    fn tinted(self,accent:Self,fg:Self,mut amount:f64)->Self{
        for _ in 0..16{
            let candidate=self.blend(accent,amount);
            if candidate.contrast(fg)>=4.5{return candidate;}
            amount*=0.5;
        }
        self
    }
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct Pair{pub fill:Color,pub on:Color}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct AccentRoles{pub primary:Pair,pub container:Pair,pub strong_container:Pair}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct Roles{
    pub surface:Pair,pub surface_container:Pair,pub muted:Color,
    pub primary:AccentRoles,pub secondary:AccentRoles,pub tertiary:AccentRoles,pub error:Pair,
}
impl Roles{
    /// Preserve existing user palette values. Secondary/tertiary tinting
    /// uses Atelier's RGB blends, not the official Material HCT algorithm.
    pub fn from_palette(surface:&str,container:&str,text:&str,muted:&str,accent:&str)->Result<Self>{
        let (surface,container,text,muted,accent)=(Color::parse(surface)?,Color::parse(container)?,Color::parse(text)?,Color::parse(muted)?,Color::parse(accent)?);
        let pair=|fill:Color,preferred:Color|Pair{fill,on:preferred.readable_on(fill)};
        let roles=|accent:Color|{
            let fg=text.readable_on(surface);
            AccentRoles{primary:pair(accent,text),container:pair(surface.tinted(accent,fg,0.23),fg),strong_container:pair(surface.tinted(accent,fg,0.46),fg)}
        };
        let secondary=accent.blend(Color([122,209,186]),0.45);
        let tertiary=accent.blend(Color([214,163,224]),0.45);
        let error=if surface.contrast(Color([255;3]))>surface.contrast(Color([0;3])){Color([255,180,171])}else{Color([186,26,26])};
        Ok(Self{surface:pair(surface,text),surface_container:pair(container,text),muted:muted.readable_on(container),primary:roles(accent),secondary:roles(secondary),tertiary:roles(tertiary),error:pair(error,text)})
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn official_corner_and_type_values(){
        assert_eq!(Corner::ExtraLargeIncreased.radius(200.,200.),32.);
        assert_eq!(Corner::Full.radius(80.,40.),20.);
        assert_eq!(nested_radius(28.,12.),16.);
        let base=TypeRole::LabelLarge.style(false);let emph=TypeRole::LabelLarge.style(true);
        assert_eq!((base.size,base.line_height,base.weight),(14.,20.,500));
        assert_eq!((emph.size,emph.line_height,emph.weight),(14.,20.,700));
    }
    #[test]
    fn spatial_overshoots_but_effects_are_monotonic(){
        for scheme in [Scheme::Expressive,Scheme::Standard]{
            for speed in [Speed::Fast,Speed::Default,Speed::Slow]{
                let effects=scheme.spring(speed,MotionKind::Effects);
                let mut last=0.0;
                for i in 0..1000{
                    let next=effects.progress(f64::from(i)/1000.,false);
                    assert!(next>=last&&next<=1.0);last=next;
                }
                assert_eq!(effects.progress(0.0,true),1.0);
                assert_eq!(effects.progress(4.0,false),1.0);
            }
        }
        let spring=Scheme::Expressive.spring(Speed::Fast,MotionKind::Spatial);
        assert!((0..1000).any(|i|spring.progress(f64::from(i)/1000.,false)>1.0));
    }
    #[test]
    fn pairs_remain_readable_even_for_extreme_custom_colors(){
        for surface in ["#ffffff","#000000","#777777"]{
            for accent in ["#000000","#ffffff","#ffff00","#0000ff"]{
                let r=Roles::from_palette(surface,surface,"#777777","#777777",accent).unwrap();
                for p in [r.surface,r.surface_container,r.primary.primary,r.primary.container,r.primary.strong_container,r.secondary.primary,r.secondary.container,r.secondary.strong_container,r.tertiary.primary,r.tertiary.container,r.tertiary.strong_container,r.error]{
                    assert!(p.fill.contrast(p.on)>=4.5);
                }
            }
        }
    }
}
