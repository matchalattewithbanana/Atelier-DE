//! Native shell appearance, independent of the legacy settings.toml and rice formats.
use anyhow::{bail,Context,Result};
use serde::{Deserialize,Serialize};
use std::{fs,io::{Read,Write},os::unix::fs::OpenOptionsExt,path::{Path,PathBuf},time::{Duration,Instant}};

#[derive(Debug,Default,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum WallpaperFit { #[default] Cover, Contain, Stretch }

#[derive(Debug,Default,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum WidgetPosition { #[default] TopLeft, TopRight }

#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct DesktopWidgets {
    pub enabled:bool,
    pub position:WidgetPosition,
    pub width:u32,
    pub clock:bool,
    pub calendar:bool,
    pub media:bool,
    pub system:bool,
}
impl Default for DesktopWidgets {
    fn default()->Self{Self{enabled:false,position:WidgetPosition::TopLeft,width:280,clock:true,calendar:true,media:false,system:false}}
}
impl DesktopWidgets {
    pub fn validate(&self)->Result<()>{
        if !(220..=360).contains(&self.width){bail!("widget group width must be 220–360");}
        if self.enabled && !(self.clock||self.calendar||self.media||self.system){bail!("enable at least one widget or disable the group");}
        Ok(())
    }
}

/// Logical-pixel spacing for native grid layout.
#[derive(Debug,Default,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct WindowSpacing {
    pub inner_gap:u32,
    pub outer_gap:u32,
}
impl WindowSpacing {
    pub fn validate(&self)->Result<()>{
        if self.inner_gap>48||self.outer_gap>64{bail!("window gaps exceed 48 inner / 64 outer logical pixels");}
        Ok(())
    }
}

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
    pub dock_auto_hide:bool,
    pub dock_hide_fullscreen:bool,
    pub dock_icon_size:u32,
    pub dock_pins:Vec<String>,
    pub wallpaper_path:String,
    pub wallpaper_fit:WallpaperFit,
    pub widgets:DesktopWidgets,
    pub window_spacing:WindowSpacing,
}
impl Default for Appearance{
    fn default()->Self{Self{version:1,accent:String::new(),panel_height:40,panel_spacing:8,button_radius:10,launcher_icon_size:32,
        show_date:true,show_battery:true,show_media:true,show_notifications:true,show_audio:true,show_terminal:true,dock_enabled:false,dock_auto_hide:false,dock_hide_fullscreen:true,dock_icon_size:32,dock_pins:Vec::new(),wallpaper_path:String::new(),wallpaper_fit:WallpaperFit::Cover,widgets:DesktopWidgets::default(),window_spacing:WindowSpacing::default()}}
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
        self.widgets.validate()?;
        self.window_spacing.validate()?;
        if !self.wallpaper_path.is_empty() && (!Path::new(&self.wallpaper_path).is_absolute() || self.wallpaper_path.len()>4096 || self.wallpaper_path.chars().any(char::is_control)) { bail!("wallpaper requires an absolute local image path"); }
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
            css.push_str(&format!(" .atelier-panel button.suggested-action, .atelier-launcher button.suggested-action, .atelier-dock button.suggested-action {{ background-image: none; background-color: {}; color: {}; }}",self.accent,foreground));
        }
        css.push_str(&format!(" .atelier-dock {{ background: @theme_bg_color; border-radius: {}px; padding: 6px; }} .atelier-dock button {{ border-radius: {}px; padding: 6px; }}",self.button_radius,self.button_radius));
        css.push_str(" .atelier-dock button.running-task { border-bottom: 2px solid currentColor; }");
        css.push_str(&format!(" .atelier-widget {{ background: @theme_bg_color; border-radius: {}px; padding: 12px; }} .atelier-widget button {{ border-radius: {}px; }}",self.button_radius,self.button_radius));
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
    let file=match fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path){
        Ok(file)=>file,
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(None),
        Err(e)=>return Err(e.into()),
    };
    let meta=file.metadata()?;
    if !meta.is_file()||meta.len()>65536{bail!("appearance must be a regular file up to 64 KiB");}
    let mut bytes=Vec::new();file.take(65537).read_to_end(&mut bytes)?;
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
/// Rate-limited observer of the native appearance profile. A bad or missing
/// replacement never instructs a running consumer to discard its last valid
/// settings. Changed invalid bytes and persistent read errors are reported
/// once; valid replacements resume observation automatically.
pub struct AppearanceMonitor{
    path:PathBuf,
    last_check:Option<Instant>,
    observed:Option<Option<Vec<u8>>>,
    last_error:Option<String>,
    had_file:bool,
    current:Option<Appearance>,
}
impl AppearanceMonitor{
    pub fn new(path:PathBuf)->Self{
        Self{path,last_check:None,observed:None,last_error:None,had_file:false,current:None}
    }
    pub fn poll(&mut self)->Result<Option<Appearance>>{
        if self.last_check.is_some_and(|t|t.elapsed()<Duration::from_millis(500)){return Ok(None);}
        self.last_check=Some(Instant::now());
        self.check()
    }
    fn check(&mut self)->Result<Option<Appearance>>{
        let bytes=match read(&self.path){
            Ok(bytes)=>{self.last_error=None;bytes}
            Err(error)=>{
                let message=error.to_string();
                if self.last_error.as_ref()==Some(&message){return Ok(None);}
                self.last_error=Some(message);return Err(error);
            }
        };
        if self.observed.as_ref()==Some(&bytes){return Ok(None);}
        self.observed=Some(bytes.clone());
        if bytes.is_none()&&self.had_file{bail!("appearance disappeared; retaining last valid settings");}
        let value=decode(bytes.as_deref())?;
        self.had_file|=bytes.is_some();
        if self.current.as_ref()==Some(&value){return Ok(None);}
        self.current=Some(value.clone());
        Ok(Some(value))
    }
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
        let lock=fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .mode(0o600).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(lock_path)?;
        if !lock.metadata()?.is_file(){bail!("unsafe appearance lock");}
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
    fn observer_keeps_last_valid_and_recovers(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("appearance.toml");
        let mut monitor=AppearanceMonitor::new(path.clone());
        assert_eq!(monitor.check().unwrap(),Some(Appearance::default()));
        assert_eq!(monitor.check().unwrap(),None);
        let mut review=AppearanceReview::load(&path).unwrap();
        let next=Appearance{accent:"#123456".into(),window_spacing:WindowSpacing{inner_gap:8,outer_gap:12},..Appearance::default()};
        review.save(next.clone()).unwrap();
        assert_eq!(monitor.check().unwrap(),Some(next.clone()));
        fs::write(&path,"accent='not a color'").unwrap();
        assert!(monitor.check().is_err());assert_eq!(monitor.check().unwrap(),None);
        assert_eq!(monitor.current.as_ref(),Some(&next));
        fs::remove_file(&path).unwrap();assert!(monitor.check().is_err());
        assert_eq!(monitor.check().unwrap(),None);
        replace(&path,&encoded(&Appearance::default()).unwrap()).unwrap();
        assert_eq!(monitor.check().unwrap(),Some(Appearance::default()));
    }
    #[test]
    fn appearance_reader_refuses_symlink(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("appearance.toml");
        let target=dir.path().join("other.toml");fs::write(&target,"version=1").unwrap();
        std::os::unix::fs::symlink(&target,&path).unwrap();assert!(Appearance::load(&path).is_err());
    }
    #[test]
    fn window_spacing_round_trip(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("appearance.toml");
        let mut review=AppearanceReview::load(&path).unwrap();
        let mut next=review.value.clone();next.window_spacing=WindowSpacing{inner_gap:12,outer_gap:20};
        review.save(next.clone()).unwrap();assert_eq!(Appearance::load(&path).unwrap(),next);
        assert!(WindowSpacing{inner_gap:49,outer_gap:0}.validate().is_err());
        assert_eq!(decode(Some(b"version=1\n")).unwrap().window_spacing,WindowSpacing::default());
    }
    #[test]
    fn widget_preferences_round_trip_and_reject_empty_groups(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("appearance.toml");
        let mut review=AppearanceReview::load(&path).unwrap();
        let mut next=review.value.clone();next.wallpaper_path="/home/user/wall.png".into();
        next.widgets=DesktopWidgets{enabled:true,position:WidgetPosition::TopRight,width:320,clock:false,calendar:true,media:true,system:true};
        review.save(next.clone()).unwrap();
        assert_eq!(Appearance::load(&path).unwrap(),next);
        assert!(!review.previous().unwrap().widgets.enabled);
        let invalid=DesktopWidgets{enabled:true,clock:false,calendar:false,media:false,system:false,..DesktopWidgets::default()};
        assert!(invalid.validate().is_err());
        assert!(DesktopWidgets{width:500,..DesktopWidgets::default()}.validate().is_err());
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
