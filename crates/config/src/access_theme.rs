//! Portable appearance for a future native greeter and ordinary session locker.
//! A theme can describe pixels, never authentication, commands or privileges.
//! The emergency recovery locker deliberately does not consume these files.
use anyhow::{bail,Context,Result};
use serde::{Deserialize,Serialize};
use std::{io::Write,path::Path};
use crate::themes::{rgb,contrast};

const FORMAT:&str="atelier-access-theme";
const MAX_BYTES:usize=16384;

#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum Layout{Centered,Split,Editorial,Bento}
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum Typography{Sans,Monospace,Display}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Palette{
    pub background:String,pub panel:String,pub text:String,pub muted:String,pub accent:String,
}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme{
    pub format:String,pub version:u32,pub name:String,
    pub layout:Layout,pub typography:Typography,pub palette:Palette,
    pub corner_radius:u32,pub clock:bool,pub date:bool,pub motion_ms:u32,
}
/// Coordinates are fractions of a 1000 × 1000 logical canvas, not pixels.
/// The trusted host owns authentication controls within the required card.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct Rect{pub x:u32,pub y:u32,pub w:u32,pub h:u32}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct Geometry{pub auth:Rect,pub clock:Rect}
impl Layout{
    pub fn geometry(self,compact:bool)->Geometry{
        let rect=|x,y,w,h|Rect{x,y,w,h};
        if compact{return Geometry{auth:rect(80,390,840,480),clock:rect(80,100,840,230)};}
        match self{
            Self::Centered=>Geometry{auth:rect(320,390,360,450),clock:rect(320,100,360,240)},
            Self::Split=>Geometry{auth:rect(570,200,360,600),clock:rect(70,200,450,600)},
            Self::Editorial=>Geometry{auth:rect(620,410,320,420),clock:rect(60,120,510,600)},
            Self::Bento=>Geometry{auth:rect(510,120,430,760),clock:rect(60,120,390,760)},
        }
    }
}
pub const PRESETS:&[(&str,&str)]=&[
    ("classic","Atelier Classic"),("sekaipop","SekaiPop"),("monogram","Monogram"),("industrial","Industrial"),
];
pub fn preset(id:&str)->Result<Theme>{
    let (name,layout,typography,colors,radius)=match id{
        "classic"=>("Atelier Classic",Layout::Centered,Typography::Sans,["#101522","#202b40","#f2f5ff","#bdc8db","#abc4ff"],24),
        "sekaipop"=>("SekaiPop",Layout::Split,Typography::Sans,["#24182c","#392543","#fff1fb","#e1c4e6","#ffb6e5"],28),
        "monogram"=>("Monogram",Layout::Bento,Typography::Monospace,["#e7e7e7","#fafafa","#151515","#454545","#a22424"],12),
        "industrial"=>("Industrial",Layout::Editorial,Typography::Display,["#171717","#292522","#fff0dd","#d7c3ad","#ffb064"],4),
        _=>bail!("unknown native access theme"),
    };
    let [background,panel,text,muted,accent]=colors.map(str::to_owned);
    let theme=Theme{format:FORMAT.into(),version:1,name:name.into(),layout,typography,
        palette:Palette{background,panel,text,muted,accent},corner_radius:radius,clock:true,date:true,motion_ms:160};
    theme.validate()?;Ok(theme)
}
impl Theme{
    pub fn validate(&self)->Result<()>{
        if self.format!=FORMAT||self.version!=1||self.name.trim().is_empty()||self.name.len()>80||
            self.name.chars().any(|c|c.is_control()||matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')){
            bail!("unsupported theme format, version or name");
        }
        if self.corner_radius>48||self.motion_ms>400{bail!("radius or motion outside supported bounds");}
        let surfaces=[rgb(&self.palette.background)?,rgb(&self.palette.panel)?];
        for value in [&self.palette.text,&self.palette.muted,&self.palette.accent]{
            let fg=rgb(value)?;
            if surfaces.iter().any(|bg|contrast(fg,*bg)<4.5){bail!("theme foreground colors require 4.5:1 contrast against both surfaces");}
        }
        Ok(())
    }
    pub fn parse(text:&str)->Result<Self>{
        if text.len()>MAX_BYTES{bail!("access theme exceeds 16 KiB");}
        let theme:Self=serde_json::from_str(text)?;
        theme.validate()?;Ok(theme)
    }
    pub fn json(&self)->Result<String>{self.validate()?;Ok(serde_json::to_string_pretty(self)?)}
    pub fn import(path:&Path)->Result<Self>{
        if !path.is_absolute()||path.extension().and_then(|s|s.to_str())!=Some("json"){bail!("use an absolute .json path");}
        use std::{fs::OpenOptions,io::Read,os::unix::fs::OpenOptionsExt};
        let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path)?;
        let metadata=file.metadata()?;
        if !metadata.is_file()||metadata.len()>MAX_BYTES as u64{bail!("invalid access theme file");}
        let mut bytes=Vec::new();file.take(MAX_BYTES as u64+1).read_to_end(&mut bytes)?;
        Self::parse(std::str::from_utf8(&bytes)?)
    }
    pub fn export(&self,path:&Path)->Result<()>{
        if !path.is_absolute()||path.extension().and_then(|s|s.to_str())!=Some("json"){bail!("use an absolute .json path");}
        let text=self.json()?;
        let parent=path.parent().context("missing theme parent")?;
        let mut tmp=tempfile::NamedTempFile::new_in(parent)?;
        tmp.write_all(text.as_bytes())?;tmp.as_file().sync_all()?;
        tmp.persist_noclobber(path).context("theme destination exists or cannot be created")?;
        std::fs::File::open(parent)?.sync_all()?;Ok(())
    }
    pub fn motion(&self,reduced_motion:bool)->u32{if reduced_motion{0}else{self.motion_ms}}
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn presets_round_trip_and_keep_required_controls(){
        for (id,_) in PRESETS{
            let theme=preset(id).unwrap();assert_eq!(Theme::parse(&theme.json().unwrap()).unwrap(),theme);
            for compact in [false,true]{
                let g=theme.layout.geometry(compact);
                for r in [g.auth,g.clock]{assert!(r.w>=320&&r.h>=230&&r.x+r.w<=1000&&r.y+r.h<=1000);}
                assert!(g.auth.x>=g.clock.x+g.clock.w||g.clock.x>=g.auth.x+g.auth.w||g.auth.y>=g.clock.y+g.clock.h||g.clock.y>=g.auth.y+g.auth.h);
            }
            assert_eq!(theme.motion(true),0);
        }
    }
    #[test]
    fn rejects_executable_fields_paths_unreadable_colors_and_unknown_versions(){
        let theme=preset("classic").unwrap();
        for key in ["command","pam_service","auth","wallpaper_path","unlock","script"]{
            let mut json=serde_json::to_value(&theme).unwrap();json[key]="/tmp/evil".into();assert!(Theme::parse(&json.to_string()).is_err());
        }
        let mut bad=theme.clone();bad.palette.text=bad.palette.background.clone();assert!(bad.json().is_err());
        bad=theme.clone();bad.version=2;assert!(bad.json().is_err());
        bad=theme;bad.motion_ms=401;assert!(bad.json().is_err());
        assert!(Theme::parse(&" ".repeat(MAX_BYTES+1)).is_err());
    }
    #[test]
    fn import_export_refuses_overwrite_and_symlink(){
        let root=tempfile::tempdir().unwrap();let path=root.path().join("theme.json");
        let theme=preset("industrial").unwrap();theme.export(&path).unwrap();
        assert_eq!(Theme::import(&path).unwrap(),theme);assert!(theme.export(&path).is_err());
        let link=root.path().join("alias.json");std::os::unix::fs::symlink(path, &link).unwrap();
        assert!(Theme::import(&link).is_err());
    }
}
