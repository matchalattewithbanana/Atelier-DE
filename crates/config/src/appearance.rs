//! Native shell appearance, independent of the legacy settings.toml and rice formats.
use anyhow::{bail,Context,Result};
use serde::{Deserialize,Serialize};
use std::{fs,io::{Read,Write},os::unix::fs::OpenOptionsExt,path::{Path,PathBuf},time::{Duration,Instant}};

#[derive(Debug,Default,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum WallpaperFit { #[default] Cover, Contain, Stretch }

/// An ordered local playlist; absent means static wallpaper or solid color.
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallpaperSlideshow{
    pub images:Vec<String>,
    pub interval_minutes:u32,
}
impl WallpaperSlideshow{
    pub fn validate(&self)->Result<()>{
        if !(1..=1440).contains(&self.interval_minutes)||!(2..=32).contains(&self.images.len()){
            bail!("slideshow needs 2–32 images and an interval of 1–1440 minutes");
        }
        let mut seen=std::collections::HashSet::new();
        for path in &self.images{
            if path.len()>1024||!Path::new(path).is_absolute()||path.chars().any(char::is_control)||!seen.insert(path){
                bail!("slideshow requires unique absolute image paths up to 1024 bytes");
            }
        }
        Ok(())
    }
}

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

/// Solid window outlines in native logical pixels. Zero preserves the old layout.
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct WindowBorder {
    pub width:u32,
    pub focused:String,
    pub unfocused:String,
}
impl Default for WindowBorder{
    fn default()->Self{Self{width:0,focused:"#91a8ff".into(),unfocused:"#586174".into()}}
}
impl WindowBorder{
    pub fn validate(&self)->Result<()>{
        if self.width>8{bail!("border width must be 0–8 logical pixels");}
        for value in [&self.focused,&self.unfocused]{
            if value.len()!=7||!value.starts_with('#')||!value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit){bail!("border colors must be #RRGGBB");}
        }
        Ok(())
    }
    pub fn color(&self,focused:bool)->Result<[f32;4]>{
        self.validate()?;
        let value=if focused{&self.focused}else{&self.unfocused};
        let rgb=u32::from_str_radix(&value[1..],16)?;
        Ok([((rgb>>16)&255) as f32/255.0,((rgb>>8)&255) as f32/255.0,(rgb&255) as f32/255.0,1.0])
    }
}
/// Optional bounded ambient shadow outside a normal window's outline.
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct WindowShadow{
    pub radius:u32,
    pub opacity:u32,
}
impl Default for WindowShadow{
    fn default()->Self{Self{radius:0,opacity:32}}
}
impl WindowShadow{
    pub fn validate(&self)->Result<()>{
        if self.radius>32||self.opacity>60{bail!("shadow radius must be 0–32 pixels and opacity 0–60 percent");}
        Ok(())
    }
    pub fn is_default(&self)->bool{*self==Self::default()}
}

/// Percentage opacity. Shell values affect backgrounds only, not text/icons.
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct Transparency{
    pub focused:u32,
    pub unfocused:u32,
    pub panel:u32,
    pub dock:u32,
    pub widgets:u32,
}
impl Default for Transparency{
    fn default()->Self{Self{focused:100,unfocused:100,panel:100,dock:100,widgets:100}}
}
impl Transparency{
    pub fn validate(&self)->Result<()>{
        if [self.focused,self.unfocused].iter().any(|n|!(70..=100).contains(n))
            ||[self.panel,self.dock,self.widgets].iter().any(|n|!(65..=100).contains(n)){
            bail!("window opacity must be 70–100%, shell background opacity 65–100%");
        }
        Ok(())
    }
    pub fn is_default(&self)->bool{*self==Self::default()}
    pub fn window_alpha(&self,focused:bool,special:bool)->f32{
        if special{1.0}else{(if focused{self.focused}else{self.unfocused}) as f32/100.0}
    }
}

/// Local accessibility preference, deliberately not part of portable themes.
#[derive(Debug,Default,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct Motion{
    pub opening_fade_ms:u32,
    pub reduced_motion:bool,
}
impl Motion{
    pub fn validate(&self)->Result<()>{
        if self.opening_fade_ms!=0&&!(80..=400).contains(&self.opening_fade_ms){bail!("opening fade must be off (0) or 80–400 ms");}
        Ok(())
    }
    pub fn opening_duration(&self)->Duration{
        Duration::from_millis(if self.reduced_motion{0}else{u64::from(self.opening_fade_ms.min(400))})
    }
    pub fn is_default(&self)->bool{*self==Self::default()}
}

/// Local cursor preferences; blank theme preserves the session environment.
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct CursorPreferences{
    pub theme:String,
    pub size:u32,
}
impl Default for CursorPreferences{
    fn default()->Self{Self{theme:String::new(),size:24}}
}
impl CursorPreferences{
    pub fn validate(&self)->Result<()>{
        if self.theme.len()>128||self.theme=="."||self.theme==".."||
            !self.theme.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_'|b'.'|b' ')){
            bail!("cursor theme must be a local theme name, not a path");
        }
        if !(8..=128).contains(&self.size){bail!("cursor size must be 8–128");}
        Ok(())
    }
    pub fn is_default(&self)->bool{self==&Self::default()}
    /// Explicit preferences take precedence; inherit the environment if unset.
    pub fn apply_to(&self,command:&mut std::process::Command){
        if !self.theme.is_empty(){
            command.env("XCURSOR_THEME",&self.theme).env("XCURSOR_SIZE",self.size.to_string());
        }
    }
}
pub const CURSOR_PRESETS:&[&str]=&[
    "Bibata-Modern-Ice","Bibata-Modern-Classic","Bibata-Modern-Amber",
    "Bibata-Original-Ice","Bibata-Original-Classic","Bibata-Original-Amber",
    "Bibata-Modern-Ice-Right","Bibata-Modern-Classic-Right","Bibata-Modern-Amber-Right",
    "Bibata-Original-Ice-Right","Bibata-Original-Classic-Right","Bibata-Original-Amber-Right",
    "GoogleDot-Black","GoogleDot-White","GoogleDot-Blue","GoogleDot-Red",
];

/// Presentation preferences only; never start a daemon or enable popups.
#[derive(Debug,Default,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum NotificationStyle { #[default] LaunchDefault, Island, Floating, Compact }

#[derive(Debug,Default,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum NotificationPosition { #[default] Automatic, TopLeft, TopCenter, TopRight }

#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct NotificationAppearance {
    pub style: NotificationStyle,
    pub position: NotificationPosition,
    /// Empty follows the panel; otherwise an exact connector name.
    pub output: String,
    pub hide_when_session_unavailable: bool,
    pub width: u32,
    pub max_cards: u32,
    pub timeout_seconds: u32,
    pub animation_ms: u32,
    pub radius: u32,
    pub opacity: u32,
}
impl Default for NotificationAppearance {
    fn default()->Self{Self{style:NotificationStyle::LaunchDefault,position:NotificationPosition::Automatic,output:String::new(),hide_when_session_unavailable:false,width:340,max_cards:3,timeout_seconds:5,animation_ms:160,radius:16,opacity:100}}
}
impl NotificationAppearance {
    pub fn is_default(&self)->bool{*self==Self::default()}
    pub fn validate(&self)->Result<()>{
        for (name,value,min,max) in [("Notification width",self.width,260,480),("Notification stack",self.max_cards,1,3),("Notification timeout",self.timeout_seconds,2,15),("Notification radius",self.radius,0,32),("Notification opacity",self.opacity,85,100)]{
            if !(min..=max).contains(&value){bail!("{name} must be {min}–{max}");}
        }
        if self.output.len()>128 || !self.output.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_'|b'.')){bail!("notification output must be empty or a connector name");}
        if self.animation_ms!=0 && !(80..=400).contains(&self.animation_ms){bail!("notification animation must be off (0) or 80–400 ms");}
        Ok(())
    }
    pub fn transition_ms(&self,motion:Motion)->u32{if motion.reduced_motion{0}else{self.animation_ms}}
    pub fn style_for_launch(&self,island:bool)->NotificationStyle{
        match self.style{NotificationStyle::LaunchDefault if island=>NotificationStyle::Island,NotificationStyle::LaunchDefault=>NotificationStyle::Floating,style=>style}
    }
    pub fn position_for_style(&self,style:NotificationStyle)->NotificationPosition{
        match self.position{NotificationPosition::Automatic if style==NotificationStyle::Island=>NotificationPosition::TopCenter,NotificationPosition::Automatic=>NotificationPosition::TopRight,position=>position}
    }
}

#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct Appearance {
    pub version:u32,
    pub accent:String,
    #[serde(skip_serializing_if="CursorPreferences::is_default")]
    pub cursor:CursorPreferences,
    pub panel_height:u32,
    pub panel_spacing:u32,
    pub button_radius:u32,
    pub launcher_icon_size:u32,
    pub show_date:bool,
    pub show_battery:bool,
    pub show_media:bool,
    pub show_notifications:bool,
    #[serde(skip_serializing_if="NotificationAppearance::is_default")]
    pub notifications:NotificationAppearance,
    pub show_audio:bool,
    pub show_terminal:bool,
    pub dock_enabled:bool,
    pub dock_auto_hide:bool,
    pub dock_hide_fullscreen:bool,
    pub dock_icon_size:u32,
    pub dock_pins:Vec<String>,
    pub wallpaper_path:String,
    pub wallpaper_fit:WallpaperFit,
    pub wallpaper_blur:u32,
    pub wallpaper_slideshow:Option<WallpaperSlideshow>,
    pub widgets:DesktopWidgets,
    pub window_spacing:WindowSpacing,
    pub window_border:WindowBorder,
    pub window_shadow:WindowShadow,
    pub window_corner_radius:u32,
    #[serde(skip_serializing_if="crate::WindowRules::is_empty")]
    pub window_rules:crate::WindowRules,
    pub backdrop_blur:u32,
    pub panel_backdrop_blur:u32,
    pub dock_backdrop_blur:u32,
    pub widget_backdrop_blur:u32,
    pub transparency:Transparency,
    #[serde(skip_serializing_if="Motion::is_default")]
    pub motion:Motion,
    pub shell_palette:Option<crate::themes::ShellPalette>,
}
impl Default for Appearance{
    fn default()->Self{Self{version:1,accent:String::new(),cursor:CursorPreferences::default(),panel_height:40,panel_spacing:8,button_radius:10,launcher_icon_size:32,
        show_date:true,show_battery:true,show_media:true,show_notifications:true,notifications:NotificationAppearance::default(),show_audio:true,show_terminal:true,dock_enabled:false,dock_auto_hide:false,dock_hide_fullscreen:true,dock_icon_size:32,dock_pins:Vec::new(),wallpaper_path:String::new(),wallpaper_fit:WallpaperFit::Cover,wallpaper_blur:0,wallpaper_slideshow:None,widgets:DesktopWidgets::default(),window_spacing:WindowSpacing::default(),window_border:WindowBorder::default(),window_shadow:WindowShadow::default(),window_corner_radius:0,window_rules:crate::WindowRules::default(),backdrop_blur:0,panel_backdrop_blur:0,dock_backdrop_blur:0,widget_backdrop_blur:0,transparency:Transparency::default(),motion:Motion::default(),shell_palette:None}}
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
        self.cursor.validate()?;
        validate_dock_pins(&self.dock_pins)?;
        self.widgets.validate()?;
        self.window_spacing.validate()?;
        self.window_border.validate()?;
        self.window_shadow.validate()?;
        self.window_rules.validate()?;
        if [self.backdrop_blur,self.panel_backdrop_blur,self.dock_backdrop_blur,self.widget_backdrop_blur].into_iter().any(|value|value>16){bail!("backdrop blur must be 0–16 logical pixels");}
        if self.window_corner_radius>24{bail!("window corner radius must be 0–24 logical pixels");}
        self.transparency.validate()?;
        self.motion.validate()?;
        self.notifications.validate()?;
        if self.wallpaper_blur>24{bail!("wallpaper blur must be 0–24");}
        if let Some(slideshow)=&self.wallpaper_slideshow{slideshow.validate()?;}
        if let Some(palette)=&self.shell_palette{palette.validate()?;}
        if !self.wallpaper_path.is_empty() && (!Path::new(&self.wallpaper_path).is_absolute() || self.wallpaper_path.len()>4096 || self.wallpaper_path.chars().any(char::is_control)) { bail!("wallpaper requires an absolute local image path"); }
        Ok(())
    }
    pub fn load(path:&Path)->Result<Self>{decode(read(path)?.as_deref())}
    pub fn css(&self)->Result<String>{
        self.validate()?;
        let mut css=format!(".atelier-panel {{ background: @theme_bg_color; padding: 2px; }} .atelier-panel button {{ border-radius: {}px; padding: 3px 12px; }} .atelier-launcher button {{ border-radius: {}px; }}",self.button_radius,self.button_radius);
        if !self.accent.is_empty(){
            let foreground=crate::design::Color::parse(&self.accent)?.contrasting_text().hex();
            css.push_str(&format!(" .atelier-panel button.suggested-action, .atelier-launcher button.suggested-action, .atelier-dock button.suggested-action {{ background-image: none; background-color: {}; color: {}; }}",self.accent,foreground));
        }
        css.push_str(&format!(" .atelier-dock {{ background: @theme_bg_color; border-radius: {}px; padding: 6px; }} .atelier-dock button {{ border-radius: {}px; padding: 6px; }}",self.button_radius,self.button_radius));
        css.push_str(" .atelier-dock button.running-task { border-bottom: 2px solid currentColor; }");
        css.push_str(&format!(" .atelier-widget {{ background: @theme_bg_color; border-radius: {}px; padding: 12px; }} .atelier-widget button {{ border-radius: {}px; }}",self.button_radius,self.button_radius));
        if let Some(palette)=&self.shell_palette{css.push_str(&palette.css()?);}
        // Explicit transparent layer roots avoid an opaque GTK window backing.
        css.push_str(" window.atelier-panel-layer, window.atelier-dock-layer, window.atelier-widget-layer { background: transparent; }");
        let color=self.shell_palette.as_ref().map_or("@theme_bg_color",|p|p.surface.as_str());
        for (class,opacity) in [("atelier-panel",self.transparency.panel),("atelier-dock",self.transparency.dock),("atelier-widget",self.transparency.widgets)]{
            if opacity<100{
                css.push_str(&format!(" .{class} {{ background-image: none; background-color: alpha({color}, {:.2}); }}",opacity as f64/100.0));
            }
        }
        let notification_bg=format!("alpha({color}, {:.2})",self.notifications.opacity as f64/100.0);
        css.push_str(&format!(" window.atelier-notification-layer {{ background: transparent; }} .atelier-notification-card {{ background: {notification_bg}; border-radius: {}px; padding: 12px; }} .atelier-notification-pill {{ background: {notification_bg}; border-radius: 24px; padding: 6px 20px; font-weight: bold; }} .atelier-notification-card button {{ border-radius: {}px; }} .atelier-notification-compact {{ padding: 6px 10px; }}", self.notifications.radius, self.button_radius));
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
pub(crate) fn read(path:&Path)->Result<Option<Vec<u8>>>{
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
fn encoded(value:&Appearance)->Result<Vec<u8>>{
    value.validate()?;
    let bytes=toml::to_string_pretty(value)?.into_bytes();
    if bytes.len()>65536{bail!("appearance exceeds 64 KiB after serialization");}
    Ok(bytes)
}
pub(crate) fn replace(path:&Path,bytes:&[u8])->Result<()>{
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
    fn transparency_and_blur_validation(){
        let a=Appearance{transparency:Transparency{focused:95,unfocused:80,panel:75,dock:85,widgets:90},wallpaper_blur:12,..Appearance::default()};
        assert_eq!(a.transparency.window_alpha(false,false),0.8);
        assert_eq!(a.transparency.window_alpha(false,true),1.0);
        assert!(a.css().unwrap().contains("alpha(@theme_bg_color, 0.75)"));
        assert!(Appearance{wallpaper_blur:25,..a.clone()}.validate().is_err());
        assert!(Transparency{focused:69,..a.transparency}.validate().is_err());
        assert!(Transparency{dock:101,..a.transparency}.validate().is_err());
        let bytes=encoded(&a).unwrap();assert_eq!(decode(Some(&bytes)).unwrap(),a);
    }
    #[test]
    fn backdrop_blur_roundtrip(){
        let a=Appearance{backdrop_blur:12,panel_backdrop_blur:8,dock_backdrop_blur:16,widget_backdrop_blur:10,..Appearance::default()};
        assert_eq!(decode(Some(&encoded(&a).unwrap())).unwrap(),a);
        assert!(Appearance{panel_backdrop_blur:17,..a.clone()}.validate().is_err());
        assert!(Appearance{dock_backdrop_blur:17,..a.clone()}.validate().is_err());
        assert!(Appearance{widget_backdrop_blur:17,..a.clone()}.validate().is_err());
        assert!(Appearance{backdrop_blur:17,..a}.validate().is_err());
    }
    #[test]
    fn application_rules_roundtrip_and_theme_preservation(){
        let mut a=Appearance::default();
        assert!(a.window_rules.is_empty());
        a.window_rules.insert("org.example.Video".into(),crate::WindowRule{focused_opacity:Some(100),corner_radius:Some(0),disable_shadow:true,..Default::default()}).unwrap();
        let saved=encoded(&a).unwrap();
        assert_eq!(decode(Some(&saved)).unwrap(),a);
        let theme=crate::themes::PortableTheme::capture("test".into(),&Appearance::default());
        assert_eq!(theme.applied(&a).unwrap().window_rules,a.window_rules);
        assert!(!theme.json().unwrap().contains("org.example.Video"));
    }
    #[test]
    fn motion_is_bounded_local_and_reduced_motion_wins(){
        let a=Appearance{motion:Motion{opening_fade_ms:180,reduced_motion:true},..Appearance::default()};
        assert_eq!(a.motion.opening_duration(),Duration::ZERO);
        assert_eq!(decode(Some(&encoded(&a).unwrap())).unwrap(),a);
        assert!(Motion{opening_fade_ms:79,reduced_motion:false}.validate().is_err());
        assert!(Motion{opening_fade_ms:401,reduced_motion:false}.validate().is_err());
        assert_eq!(Motion{opening_fade_ms:180,reduced_motion:false}.opening_duration(),Duration::from_millis(180));
        let theme=crate::themes::PortableTheme::capture("motion test".into(),&Appearance::default());
        assert_eq!(theme.applied(&a).unwrap().motion,a.motion);
        assert!(!theme.json().unwrap().contains("reduced_motion"));
    }
    #[test]
    fn corner_radius_roundtrip(){
        let a=Appearance{window_corner_radius:16,..Appearance::default()};
        assert_eq!(decode(Some(&encoded(&a).unwrap())).unwrap(),a);
        assert!(Appearance{window_corner_radius:25,..a}.validate().is_err());
    }
    #[test]
    fn shadow_preferences_round_trip(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("appearance.toml");
        let mut review=AppearanceReview::load(&path).unwrap();
        let mut next=review.value.clone();next.window_shadow=WindowShadow{radius:24,opacity:40};
        review.save(next.clone()).unwrap();assert_eq!(Appearance::load(&path).unwrap(),next);
        assert!(WindowShadow{radius:33,opacity:40}.validate().is_err());
        assert!(WindowShadow{radius:16,opacity:61}.validate().is_err());
    }
    #[test]
    fn border_validation_and_round_trip(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("appearance.toml");
        let mut review=AppearanceReview::load(&path).unwrap();
        let border=WindowBorder{width:3,focused:"#ff0000".into(),unfocused:"#00FF00".into()};
        assert_eq!(border.color(true).unwrap(),[1.0,0.0,0.0,1.0]);
        assert_eq!(border.color(false).unwrap(),[0.0,1.0,0.0,1.0]);
        let mut next=review.value.clone();next.window_border=border.clone();
        review.save(next.clone()).unwrap();assert_eq!(Appearance::load(&path).unwrap(),next);
        assert!(WindowBorder{width:9,..border.clone()}.validate().is_err());
        assert!(WindowBorder{focused:"red".into(),..border}.validate().is_err());
    }
    #[test]
    fn slideshow_preferences_roundtrip(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("appearance.toml");
        let mut review=AppearanceReview::load(&path).unwrap();
        let mut next=review.value.clone();
        next.wallpaper_slideshow=Some(WallpaperSlideshow{images:vec!["/tmp/a.png".into(),"/tmp/b.jpg".into()],interval_minutes:5});
        review.save(next.clone()).unwrap();assert_eq!(Appearance::load(&path).unwrap(),next);
        let mut invalid=next.wallpaper_slideshow.unwrap();invalid.images[1]=invalid.images[0].clone();assert!(invalid.validate().is_err());
        invalid.images[1]="/tmp/c.png".into();invalid.interval_minutes=0;assert!(invalid.validate().is_err());
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

#[cfg(test)]
mod cursor_tests{
    use super::*;
    #[test]
    fn cursor_preferences_are_backward_compatible_and_validate(){
        let old:Appearance=toml::from_str("version = 1").unwrap();
        assert_eq!(old.cursor,CursorPreferences::default());
        for theme in CURSOR_PRESETS{assert!(CursorPreferences{theme:(*theme).into(),size:24}.validate().is_ok());}
        for theme in ["../escape","/absolute",".","..","bad\nname"]{assert!(CursorPreferences{theme:theme.into(),size:24}.validate().is_err());}
        assert!(CursorPreferences{theme:"default".into(),size:129}.validate().is_err());
        let mut appearance=old;appearance.cursor=CursorPreferences{theme:"GoogleDot-Blue".into(),size:32};
        let decoded:Appearance=toml::from_str(&toml::to_string(&appearance).unwrap()).unwrap();
        assert_eq!(decoded.cursor,appearance.cursor);
        let mut command=std::process::Command::new("test");decoded.cursor.apply_to(&mut command);
        assert!(command.get_envs().any(|(key,value)|key=="XCURSOR_THEME"&&value.is_some_and(|v|v=="GoogleDot-Blue")));
    }
}

#[cfg(test)]
mod notification_tests{
    use super::*;
    #[test]
    fn legacy_defaults_and_local_preferences_round_trip(){
        let old=decode(Some(b"version=1\n")).unwrap();
        assert_eq!(old.notifications,NotificationAppearance::default());
        let mut next=old;
        next.notifications=NotificationAppearance{style:NotificationStyle::Compact,position:NotificationPosition::TopLeft,output:"DP-2".into(),hide_when_session_unavailable:true,width:420,max_cards:2,timeout_seconds:8,animation_ms:200,radius:24,opacity:90};
        assert_eq!(decode(Some(&encoded(&next).unwrap())).unwrap(),next);
        assert!(next.css().unwrap().contains("border-radius: 24px"));
        assert_eq!(next.notifications.transition_ms(Motion{opening_fade_ms:0,reduced_motion:true}),0);
    }
    #[test]
    fn invalid_notification_geometry_is_rejected(){
        let mut n=NotificationAppearance::default();n.width=1000;assert!(n.validate().is_err());
        n=NotificationAppearance::default();n.max_cards=0;assert!(n.validate().is_err());
        n=NotificationAppearance::default();n.opacity=10;assert!(n.validate().is_err());
        n=NotificationAppearance::default();n.animation_ms=20;assert!(n.validate().is_err());
        n=NotificationAppearance::default();n.output="../DP-1".into();assert!(n.validate().is_err());
        n.output="eDP-1".into();assert!(n.validate().is_ok());
        let n=NotificationAppearance::default();
        assert_eq!(n.style_for_launch(true),NotificationStyle::Island);
        assert_eq!(n.style_for_launch(false),NotificationStyle::Floating);
        assert_eq!(n.position_for_style(NotificationStyle::Island),NotificationPosition::TopCenter);
    }
}
