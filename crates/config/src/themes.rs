//! Portable visual settings only: never import machine-local paths, pins or commands.
use crate::{Appearance,WindowBorder,WindowSpacing,WindowShadow,Transparency};
use anyhow::{bail,Context,Result};
use serde::{Deserialize,Serialize};
use std::{fs,io::Write,path::Path};

fn rgb(value:&str)->Result<[u8;3]>{
    if value.len()!=7||!value.starts_with('#')||!value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit){bail!("expected #RRGGBB color");}
    let n=u32::from_str_radix(&value[1..],16)?;
    Ok([(n>>16) as u8,(n>>8) as u8,n as u8])
}
fn luminance(c:[u8;3])->f64{
    let lin=|n:u8|{let v=f64::from(n)/255.0;if v<=0.04045{v/12.92}else{((v+0.055)/1.055).powf(2.4)}};
    0.2126*lin(c[0])+0.7152*lin(c[1])+0.0722*lin(c[2])
}
fn contrast(a:[u8;3],b:[u8;3])->f64{
    let (a,b)=(luminance(a),luminance(b));(a.max(b)+0.05)/(a.min(b)+0.05)
}
fn hex(c:[u8;3])->String{format!("#{:02x}{:02x}{:02x}",c[0],c[1],c[2])}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellPalette{
    pub surface:String,
    pub elevated:String,
    pub text:String,
    pub muted:String,
}
impl ShellPalette{
    pub fn validate(&self)->Result<()>{
        let surfaces=[rgb(&self.surface)?,rgb(&self.elevated)?];
        for value in [&self.text,&self.muted]{
            let fg=rgb(value)?;
            if surfaces.iter().any(|bg|contrast(fg,*bg)<4.5){bail!("palette text and muted text require 4.5:1 contrast against both surfaces");}
        }
        Ok(())
    }
    pub fn css(&self)->Result<String>{
        self.validate()?;
        Ok(format!(" .atelier-panel, .atelier-dock, .atelier-widget, .atelier-launcher {{ background: {}; color: {}; }} \
            .atelier-panel label, .atelier-dock label, .atelier-widget label, .atelier-launcher label {{ color: {}; }} \
            .atelier-panel .dim-label, .atelier-dock .dim-label, .atelier-widget .dim-label, .atelier-launcher .dim-label {{ color: {}; }} \
            .atelier-panel button:not(.suggested-action), .atelier-dock button:not(.suggested-action), .atelier-widget button:not(.suggested-action), .atelier-launcher button:not(.suggested-action) {{ background-image: none; background-color: {}; color: {}; }} \
            .atelier-panel button:not(.suggested-action) label, .atelier-dock button:not(.suggested-action) label, .atelier-widget button:not(.suggested-action) label, .atelier-launcher button:not(.suggested-action) label {{ color: {}; }} \
            .atelier-panel button.suggested-action label, .atelier-dock button.suggested-action label, .atelier-launcher button.suggested-action label {{ color: inherit; }}",
            self.surface,self.text,self.text,self.muted,self.elevated,self.text,self.text))
    }
}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableTheme{
    pub format:String,
    pub version:u32,
    pub name:String,
    pub accent:String,
    pub shell_palette:Option<ShellPalette>,
    pub button_radius:u32,
    pub panel_height:u32,
    pub panel_spacing:u32,
    pub launcher_icon_size:u32,
    pub dock_icon_size:u32,
    pub window_spacing:WindowSpacing,
    pub window_border:WindowBorder,
    #[serde(default,skip_serializing_if="WindowShadow::is_default")]
    pub window_shadow:WindowShadow,
    #[serde(default,skip_serializing_if="is_zero")]
    pub window_corner_radius:u32,
    #[serde(default,skip_serializing_if="is_zero")]
    pub backdrop_blur:u32,
    #[serde(default,skip_serializing_if="is_zero")]
    pub panel_backdrop_blur:u32,
    #[serde(default,skip_serializing_if="is_zero")]
    pub dock_backdrop_blur:u32,
    #[serde(default,skip_serializing_if="Transparency::is_default")]
    pub transparency:Transparency,
}
fn is_zero(value:&u32)->bool{*value==0}
impl PortableTheme{
    pub fn capture(name:String,a:&Appearance)->Self{
        Self{format:"atelier-native-theme".into(),version:1,name,accent:a.accent.clone(),shell_palette:a.shell_palette.clone(),
            button_radius:a.button_radius,panel_height:a.panel_height,panel_spacing:a.panel_spacing,launcher_icon_size:a.launcher_icon_size,dock_icon_size:a.dock_icon_size,
            window_spacing:a.window_spacing,window_border:a.window_border.clone(),window_shadow:a.window_shadow,window_corner_radius:a.window_corner_radius,backdrop_blur:a.backdrop_blur,panel_backdrop_blur:a.panel_backdrop_blur,dock_backdrop_blur:a.dock_backdrop_blur,transparency:a.transparency}
    }
    pub fn applied(&self,base:&Appearance)->Result<Appearance>{
        if self.format!="atelier-native-theme"||self.version!=1||self.name.trim().is_empty()||self.name.len()>80||self.name.chars().any(char::is_control){bail!("unsupported theme format, version or name");}
        let mut a=base.clone();
        a.accent=self.accent.clone();a.shell_palette=self.shell_palette.clone();a.button_radius=self.button_radius;
        a.panel_height=self.panel_height;a.panel_spacing=self.panel_spacing;a.launcher_icon_size=self.launcher_icon_size;a.dock_icon_size=self.dock_icon_size;
        a.window_spacing=self.window_spacing;a.window_border=self.window_border.clone();a.window_shadow=self.window_shadow;a.window_corner_radius=self.window_corner_radius;a.backdrop_blur=self.backdrop_blur;a.panel_backdrop_blur=self.panel_backdrop_blur;a.dock_backdrop_blur=self.dock_backdrop_blur;a.transparency=self.transparency;a.validate()?;Ok(a)
    }
    pub fn parse(text:&str)->Result<Self>{
        if text.len()>65536{bail!("theme exceeds 64 KiB");}
        let theme:Self=serde_json::from_str(text)?;theme.applied(&Appearance::default())?;Ok(theme)
    }
    pub fn json(&self)->Result<String>{self.applied(&Appearance::default())?;Ok(serde_json::to_string_pretty(self)?)}
    pub fn import(path:&Path)->Result<Self>{
        validate_path(path)?;
        let bytes=crate::appearance::read(path)?.context("theme file not found")?;
        Self::parse(std::str::from_utf8(&bytes)?)
    }
    pub fn export(&self,path:&Path)->Result<()>{
        validate_path(path)?;
        let text=self.json()?;
        let parent=path.parent().context("theme parent directory missing")?;
        let mut tmp=tempfile::NamedTempFile::new_in(parent)?;tmp.write_all(text.as_bytes())?;tmp.as_file().sync_all()?;
        tmp.persist_noclobber(path).context("theme destination exists or cannot be created")?;
        fs::File::open(parent)?.sync_all()?;Ok(())
    }
}
fn validate_path(path:&Path)->Result<()>{
    if !path.is_absolute()||path.extension().and_then(|s|s.to_str())!=Some("json"){bail!("use an absolute .json theme path");}Ok(())
}
/// Derive restrained tinted surfaces and a readable foreground, using the
/// sampled color as an accent. This does not change the wallpaper or geometry.
pub fn from_seed(seed:[u8;3],light:bool,base:&Appearance)->Result<PortableTheme>{
    let mix=|a:[u8;3],b:[u8;3],ratio:f64|std::array::from_fn(|i|((f64::from(a[i])*(1.0-ratio)+f64::from(b[i])*ratio).round()) as u8);
    let (surface,elevated,text,muted,accent)=if light{
        (mix([250;3],seed,0.055),mix([238;3],seed,0.09),[23;3],[70;3],mix(seed,[0;3],0.34))
    }else{
        (mix([18;3],seed,0.09),mix([30;3],seed,0.13),[245;3],[186;3],mix(seed,[255;3],0.35))
    };
    let mut theme=PortableTheme::capture(if light{"Wallpaper light".into()}else{"Wallpaper dark".into()},base);
    theme.accent=hex(accent);theme.shell_palette=Some(ShellPalette{surface:hex(surface),elevated:hex(elevated),text:hex(text),muted:hex(muted)});
    theme.window_border.focused=hex(accent);
    theme.window_border.unfocused=hex(mix(seed,if light{[80;3]}else{[150;3]},0.65));
    theme.applied(base)?;Ok(theme)
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn generated_palettes_are_readable(){
        for seed in [[0,0,0],[255,255,255],[255,0,0],[0,255,0],[0,0,255],[120,80,180]]{
            for light in [true,false]{let theme=from_seed(seed,light,&Appearance::default()).unwrap();theme.shell_palette.unwrap().validate().unwrap();}
        }
    }
    #[test]
    fn portable_roundtrip_preserves_local_preferences(){
        let mut base=Appearance::default();base.wallpaper_path="/tmp/image.png".into();base.dock_pins=vec!["foot.desktop".into()];base.widgets.enabled=true;
        base.window_shadow=WindowShadow{radius:20,opacity:45};base.transparency.unfocused=85;base.wallpaper_blur=8;
        let theme=from_seed([100,150,200],false,&base).unwrap();
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("theme.json");
        theme.export(&path).unwrap();assert!(theme.export(&path).is_err());
        let loaded=PortableTheme::import(&path).unwrap();assert_eq!(loaded,theme);
        let applied=loaded.applied(&base).unwrap();assert_eq!(applied.wallpaper_path,base.wallpaper_path);assert_eq!(applied.dock_pins,base.dock_pins);assert_eq!(applied.widgets,base.widgets);assert_eq!(applied.window_shadow,base.window_shadow);assert_eq!(applied.transparency,base.transparency);assert_eq!(applied.wallpaper_blur,base.wallpaper_blur);
        let mut json=serde_json::to_value(&theme).unwrap();json["wallpaper_path"]=serde_json::json!("/tmp/other.png");assert!(PortableTheme::parse(&json.to_string()).is_err());
        let mut bad=theme;bad.shell_palette.as_mut().unwrap().text=bad.shell_palette.as_ref().unwrap().surface.clone();assert!(bad.json().is_err());
    }
}
