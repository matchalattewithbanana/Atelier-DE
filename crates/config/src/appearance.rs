//! Native shell appearance, independent of the legacy settings.toml and rice formats.
use anyhow::{bail,Context,Result};
use serde::{Deserialize,Serialize};
use std::{fs,io::{Read,Write},path::{Path,PathBuf}};

#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct Appearance {
    pub version:u32,
    pub accent:String,
    pub panel_height:u32,
    pub panel_spacing:u32,
    pub button_radius:u32,
    pub launcher_icon_size:u32,
    pub show_date:bool,
    pub show_battery:bool,
    pub show_media:bool,
    pub show_notifications:bool,
    pub show_audio:bool,
    pub show_terminal:bool,
    pub dock_enabled:bool,
    pub dock_icon_size:u32,
    pub dock_pins:Vec<String>,
}
impl Default for Appearance{
    fn default()->Self{Self{version:1,accent:String::new(),panel_height:40,panel_spacing:8,button_radius:10,launcher_icon_size:32,
        show_date:true,show_battery:true,show_media:true,show_notifications:true,show_audio:true,show_terminal:true,dock_enabled:false,dock_icon_size:32,dock_pins:Vec::new()}}
}
impl Appearance{
    pub fn validate(&self)->Result<()>{
        if self.version!=1{bail!("unsupported native appearance version");}
        if !self.accent.is_empty() && !(self.accent.len()==7 && self.accent.starts_with('#') && self.accent.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)){
            bail!("accent must be blank (system default) or #RRGGBB");
        }
        for (name,value,min,max) in [("Panel minimum height",self.panel_height,32,80),("Panel spacing",self.panel_spacing,0,24),("Button radius",self.button_radius,0,24),("Launcher icon size",self.launcher_icon_size,16,64),("Dock icon size",self.dock_icon_size,16,64)]{
            if !(min..=max).contains(&value){bail!("{name} must be {min}–{max}");}
        }
        validate_dock_pins(&self.dock_pins)?;
        Ok(())
    }
    pub fn load(path:&Path)->Result<Self>{decode(read(path)?.as_deref())}
    pub fn css(&self)->Result<String>{
        self.validate()?;
        let mut css=format!(".atelier-panel {{ background: @theme_bg_color; padding: 2px; }} .atelier-panel button {{ border-radius: {}px; padding: 3px 12px; }} .atelier-launcher button {{ border-radius: {}px; }}",self.button_radius,self.button_radius);
        if !self.accent.is_empty(){
            let n=u32::from_str_radix(&self.accent[1..],16)?;
            let linear=|v:u32|{let v=v as f64/255.0;if v<=0.04045{v/12.92}else{((v+0.055)/1.055).powf(2.4)}};
            let l=0.2126*linear((n>>16)&255)+0.7152*linear((n>>8)&255)+0.0722*linear(n&255);
            let foreground=if (l+0.05)/0.05>=1.05/(l+0.05){"#000000"}else{"#ffffff"};
            css.push_str(&format!(" .atelier-panel button.suggested-action, .atelier-launcher button.suggested-action {{ background-image: none; background-color: {}; color: {}; }}",self.accent,foreground));
        }
        css.push_str(&format!(" .atelier-dock {{ background: @theme_bg_color; border-radius: {}px; padding: 6px; }} .atelier-dock button {{ border-radius: {}px; padding: 6px; }}",self.button_radius,self.button_radius));
        Ok(css)
    }
}
/// Desktop entry IDs are case-sensitive, not executable names or paths.
pub fn validate_dock_pins(pins:&[String])->Result<()>{
    let mut seen=std::collections::HashSet::new();
    if pins.len()>32{bail!("At most 32 pinned applications");}
    for id in pins{
        if id.len()>256 || !id.ends_with(".desktop") || id.len()<=8
            || !id.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
            || !id.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_'|b'.'))
            || !seen.insert(id){
            bail!("Invalid or duplicate desktop application ID: {id}");
        }
    }
    Ok(())
}
pub fn appearance_path()->Result<PathBuf>{Ok(crate::config_path()?.with_file_name("appearance.toml"))}
fn read(path:&Path)->Result<Option<Vec<u8>>>{
    match fs::symlink_metadata(path){
        Ok(m) if !m.is_file()||m.len()>65536=>bail!("appearance must be a regular file up to 64 KiB"),
        Ok(_)=>{},
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(None),
        Err(e)=>return Err(e.into()),
    }
    let mut bytes=Vec::new();fs::File::open(path)?.take(65537).read_to_end(&mut bytes)?;
    if bytes.len()>65536{bail!("appearance file exceeds size limit");}
    Ok(Some(bytes))
}
fn decode(bytes:Option<&[u8]>)->Result<Appearance>{
    let value=match bytes{Some(b)=>toml::from_str::<Appearance>(std::str::from_utf8(b)?)?,None=>Appearance::default()};
    value.validate()?;Ok(value)
}
fn encoded(value:&Appearance)->Result<Vec<u8>>{value.validate()?;Ok(toml::to_string_pretty(value)?.into_bytes())}
fn replace(path:&Path,bytes:&[u8])->Result<()>{
    let parent=path.parent().context("appearance directory missing")?;
    let mut temp=tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;temp.as_file().sync_all()?;temp.persist(path)?;
    fs::File::open(parent)?.sync_all()?;Ok(())
}
pub struct AppearanceReview{path:PathBuf,bytes:Option<Vec<u8>>,pub value:Appearance}
impl AppearanceReview{
    pub fn load(path:&Path)->Result<Self>{let bytes=read(path)?;let value=decode(bytes.as_deref())?;Ok(Self{path:path.into(),bytes,value})}
    pub fn previous(&self)->Result<Appearance>{
        let bytes=read(&self.path.with_file_name("appearance.previous.toml"))?.context("No previous appearance backup")?;
        decode(Some(&bytes))
    }
    pub fn save(&mut self,next:Appearance)->Result<bool>{
        next.validate()?;
        let parent=self.path.parent().context("appearance directory missing")?;
        fs::create_dir_all(parent)?;
        let lock_path=self.path.with_file_name("appearance.lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|m|!m.is_file()){bail!("unsafe appearance lock");}
        let lock=fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path)?;
        // Standard-library advisory file lock; released when the descriptor drops.
        lock.lock()?;
        if read(&self.path)?!=self.bytes{bail!("Appearance changed elsewhere; reload before saving");}
        if next==self.value{return Ok(false);}
        let bytes=encoded(&next)?;
        let previous=match &self.bytes{Some(b)=>b.clone(),None=>encoded(&self.value)?};
        let backup=self.path.with_file_name("appearance.previous.toml");
        // Refuse unsafe backup destinations rather than overwrite them.
        read(&backup)?;
        replace(&backup,&previous)?;
        replace(&self.path,&bytes)?;
        self.bytes=Some(bytes);self.value=next;Ok(true)
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn reviewed_save_preserves_previous_and_rejects_stale(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("appearance.toml");
        let mut a=AppearanceReview::load(&path).unwrap();let mut stale=AppearanceReview::load(&path).unwrap();
        assert!(!a.save(Appearance::default()).unwrap());
        let next=Appearance{accent:"#123456".into(),show_date:false,..Appearance::default()};
        assert!(a.save(next.clone()).unwrap());
        assert_eq!(Appearance::load(&path).unwrap(),next);
        assert_eq!(a.previous().unwrap(),Appearance::default());
        assert!(stale.save(Appearance{panel_height:55,..Appearance::default()}).is_err());
        assert_eq!(Appearance::load(&path).unwrap(),next);
        let previous=a.previous().unwrap();a.save(previous).unwrap();
        assert_eq!(a.previous().unwrap(),next);
    }
    #[test]
    fn dock_pins_round_trip_and_validate(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("appearance.toml");
        let mut review=AppearanceReview::load(&path).unwrap();
        let next=Appearance{dock_enabled:true,dock_pins:vec!["org.gnome.Nautilus.desktop".into(),"firefox.desktop".into()],dock_icon_size:40,..Appearance::default()};
        review.save(next.clone()).unwrap();assert_eq!(Appearance::load(&path).unwrap(),next);
        for pins in [vec!["/bin/sh".into()],vec!["firefox.desktop".into(),"firefox.desktop".into()],vec![".desktop".into()]]{
            assert!(Appearance{dock_pins:pins,..Appearance::default()}.validate().is_err());
        }
        assert!(!decode(Some(b"version = 1\n")).unwrap().dock_enabled);
    }
    #[test]
    fn validation_prevents_css_injection_and_invalid_geometry(){
        assert!(Appearance{accent:"red;}".into(),..Appearance::default()}.css().is_err());
        assert!(Appearance{panel_height:200,..Appearance::default()}.validate().is_err());
        for (accent,fg) in [("#000000","#ffffff"),("#ffffff","#000000")]{
            assert!(Appearance{accent:accent.into(),..Appearance::default()}.css().unwrap().contains(&format!("color: {fg}")));
        }
    }
}
