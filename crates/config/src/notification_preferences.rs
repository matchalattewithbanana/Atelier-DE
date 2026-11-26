//! Local notification policy and script presets. No execution or identity claims.
use anyhow::{bail,Context,Result};
use serde::{Deserialize,Serialize};
use std::{collections::BTreeMap,fs,os::unix::fs::OpenOptionsExt,path::{Path,PathBuf}};
use crate::appearance::{read,replace};

const MAX_BYTES:usize=65536;

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct Preset {
    pub icon: String,
    pub urgency: u8,
    pub timeout_ms: i32,
}
impl Default for Preset{
    fn default()->Self{Self{icon:"dialog-information-symbolic".into(),urgency:1,timeout_ms:5000}}
}
impl Preset{
    pub fn validate(&self)->Result<()>{
        if self.icon.is_empty()||self.icon.len()>128||!self.icon.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_')){
            bail!("preset icon must be a simple theme icon name");
        }
        if self.urgency>2{bail!("preset urgency must be 0, 1 or 2");}
        if self.timeout_ms!=0 && !(1000..=60000).contains(&self.timeout_ms){bail!("preset timeout must be 0 (persistent) or 1000–60000 ms");}
        Ok(())
    }
}
pub fn builtin(name:&str)->Option<Preset>{
    let (icon,urgency,timeout_ms)=match name{
        "info"=>("dialog-information-symbolic",1,5000),
        "success"=>("emblem-ok-symbolic",1,5000),
        "warning"=>("dialog-warning-symbolic",1,8000),
        "error"=>("dialog-error-symbolic",2,0),
        "download"=>("folder-download-symbolic",1,5000),
        "update"=>("system-software-update-symbolic",1,5000),
        _=>return None,
    };
    Some(Preset{icon:icon.into(),urgency,timeout_ms})
}


#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct AppRule {
    pub popups:bool,
    pub history:bool,
}
impl Default for AppRule{
    fn default()->Self{Self{popups:true,history:true}}
}

#[derive(Clone,Debug,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct Preferences{
    applications:BTreeMap<String,AppRule>,
    presets:BTreeMap<String,Preset>,
}
impl Preferences{
    pub fn app(&self,name:&str)->AppRule{self.applications.get(name).copied().unwrap_or_default()}
    pub fn preset(&self,name:&str)->Result<Preset>{
        if let Some(p)=builtin(name){return Ok(p);}
        self.presets.get(name).cloned().with_context(||format!("unknown notification preset: {name}"))
    }
    pub fn decode(bytes:&[u8])->Result<Self>{
        if bytes.len()>MAX_BYTES{bail!("notification preferences exceed 64 KiB");}
        let value:Self=toml::from_str(std::str::from_utf8(bytes)?)?;
        value.validate()?;Ok(value)
    }
    pub fn validate(&self)->Result<()>{
        let value=self;
        if value.applications.len()>128||value.presets.len()>64{bail!("notification preferences allow up to 128 applications and 64 custom presets");}
        for name in value.applications.keys(){
            if name.trim().is_empty()||name.len()>256||name.chars().any(char::is_control){bail!("invalid application name in notification preferences");}
        }
        for (name,preset) in &value.presets{
            if name.is_empty()||name.len()>48||!name.bytes().all(|b|b.is_ascii_lowercase()||b.is_ascii_digit()||b==b'-')||builtin(name).is_some(){
                bail!("invalid or reserved custom notification preset name: {name}");
            }
            preset.validate()?;
        }
        Ok(())
    }
    pub fn applications(&self)->&BTreeMap<String,AppRule>{&self.applications}
    pub fn custom_presets(&self)->&BTreeMap<String,Preset>{&self.presets}
    pub fn set_app(&mut self,name:String,rule:AppRule){self.applications.insert(name,rule);}
    pub fn remove_app(&mut self,name:&str){self.applications.remove(name);}
    pub fn set_preset(&mut self,name:String,preset:Preset){self.presets.insert(name,preset);}
    pub fn remove_preset(&mut self,name:&str){self.presets.remove(name);}
    pub fn load(path:&Path)->Result<Self>{
        decode_optional(read(path)?.as_deref())
    }
}
pub fn path()->Result<PathBuf>{Ok(crate::config_path()?.with_file_name("notifications.toml"))}

fn decode_optional(bytes:Option<&[u8]>)->Result<Preferences>{
    match bytes{Some(bytes)=>Preferences::decode(bytes),None=>Ok(Preferences::default())}
}
fn encode(value:&Preferences)->Result<Vec<u8>>{
    value.validate()?;
    let bytes=toml::to_string_pretty(value)?.into_bytes();
    if bytes.len()>MAX_BYTES{bail!("notification preferences exceed 64 KiB");}
    Ok(bytes)
}

/// Reviewed edits use the same bounded file access and atomic replacement as
/// appearance settings. Refuse a stale draft, preserve the previous bytes.
pub struct Review{path:PathBuf,bytes:Option<Vec<u8>>,pub value:Preferences}
impl Review{
    pub fn load(path:&Path)->Result<Self>{
        let bytes=read(path)?;let value=decode_optional(bytes.as_deref())?;
        Ok(Self{path:path.into(),bytes,value})
    }
    pub fn previous(&self)->Result<Preferences>{
        let bytes=read(&self.path.with_file_name("notifications.previous.toml"))?.context("No previous notification preferences")?;
        Preferences::decode(&bytes)
    }
    pub fn save(&mut self,next:Preferences)->Result<bool>{
        let bytes=encode(&next)?;
        fs::create_dir_all(self.path.parent().context("notification preferences directory missing")?)?;
        let lock_path=self.path.with_file_name("notifications.lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|m|!m.is_file()){bail!("unsafe notification preferences lock");}
        let lock=fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .mode(0o600).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(lock_path)?;
        if !lock.metadata()?.is_file(){bail!("unsafe notification preferences lock");}
        lock.lock()?;
        if read(&self.path)?!=self.bytes{bail!("Notification preferences changed elsewhere; reload before saving");}
        if next==self.value{return Ok(false);}
        let previous=match &self.bytes{Some(bytes)=>bytes.clone(),None=>encode(&self.value)?};
        let backup=self.path.with_file_name("notifications.previous.toml");
        read(&backup)?;
        replace(&backup,&previous)?;replace(&self.path,&bytes)?;
        self.bytes=Some(bytes);self.value=next;Ok(true)
    }
}

/// Keep the last valid settings when a live edit is malformed or unreadable.
/// A missing file is a deliberate return to defaults. Report an unchanged
/// error only once to avoid filling the daemon log.
pub struct Monitor{
    path:PathBuf,
    current:Preferences,
    error:Option<String>,
}
impl Monitor{
    pub fn start(path:PathBuf)->Result<Self>{
        let current=Preferences::load(&path)?;
        Ok(Self{path,current,error:None})
    }
    pub fn initial(&self)->Preferences{self.current.clone()}
    pub fn poll(&mut self)->Result<Option<Preferences>>{
        match Preferences::load(&self.path){
            Ok(next)=>{
                self.error=None;
                if self.current==next{return Ok(None);}
                self.current=next.clone();Ok(Some(next))
            }
            Err(error)=>{
                let message=error.to_string();
                if self.error.as_ref()==Some(&message){return Ok(None);}
                self.error=Some(message);Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn exact_application_rules_and_custom_presets(){
        let p=Preferences::decode(br#"
[applications."Example App"]
popups=false
history=false

[presets.backup]
icon="document-save-symbolic"
urgency=1
timeout_ms=8000
"#).unwrap();
        assert!(!p.app("Example App").popups);
        assert!(!p.app("Example App").history);
        assert_eq!(p.app("example app"),AppRule::default());
        assert_eq!(p.preset("backup").unwrap().timeout_ms,8000);
        assert_eq!(p.preset("success").unwrap().icon,"emblem-ok-symbolic");
    }
    #[test]
    fn invalid_configs_are_rejected(){
        for input in [
            "[presets.success]\nicon='x'\n",
            "[presets.backup]\nicon='/tmp/icon'\n",
            "[presets.backup]\nurgency=3\n",
            "[presets.backup]\ntimeout_ms=-1\n",
            "[applications.a]\ncommand='sh'\n",
            "[presets.backup]\ncommand='sh'\n",
        ]{assert!(Preferences::decode(input.as_bytes()).is_err(),"{input}");}
        assert!(Preferences::decode(&vec![0;MAX_BYTES+1]).is_err());
    }
    #[test]
    fn reviewed_save_preserves_previous_and_rejects_stale_drafts(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("notifications.toml");
        let mut review=Review::load(&path).unwrap();let mut stale=Review::load(&path).unwrap();
        let mut next=review.value.clone();next.set_app("Example".into(),AppRule{popups:false,history:true});
        assert!(review.save(next.clone()).unwrap());
        assert_eq!(Preferences::load(&path).unwrap(),next);
        assert_eq!(review.previous().unwrap(),Preferences::default());
        assert!(stale.save(Preferences::default()).is_err());
        let previous=review.previous().unwrap();review.save(previous).unwrap();
        assert_eq!(review.previous().unwrap(),next);
    }
    #[test]
    fn monitor_keeps_last_good_rules_and_resets_on_removal(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("notifications.toml");
        let mut monitor=Monitor::start(path.clone()).unwrap();
        std::fs::write(&path,"[applications.app]\npopups=false\n").unwrap();
        assert!(!monitor.poll().unwrap().unwrap().app("app").popups);
        std::fs::write(&path,"broken=").unwrap();
        assert!(monitor.poll().is_err());
        assert!(monitor.poll().unwrap().is_none());
        assert!(!monitor.initial().app("app").popups);
        std::fs::remove_file(&path).unwrap();
        assert!(monitor.poll().unwrap().unwrap().app("app").popups);
    }
}
