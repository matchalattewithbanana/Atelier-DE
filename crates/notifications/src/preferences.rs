//! Local notification policy and script presets. No execution or identity claims.
use anyhow::{bail,Context,Result};
use serde::Deserialize;
use std::{collections::BTreeMap,fs::OpenOptions,io::Read,os::unix::fs::OpenOptionsExt,path::{Path,PathBuf}};
use crate::presets::{self,Preset};

const MAX_BYTES:usize=65536;

#[derive(Clone,Copy,Debug,PartialEq,Eq,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct AppRule {
    pub popups:bool,
    pub history:bool,
}
impl Default for AppRule{
    fn default()->Self{Self{popups:true,history:true}}
}

#[derive(Clone,Debug,Default,PartialEq,Eq,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct Preferences{
    applications:BTreeMap<String,AppRule>,
    presets:BTreeMap<String,Preset>,
}
impl Preferences{
    pub fn app(&self,name:&str)->AppRule{self.applications.get(name).copied().unwrap_or_default()}
    pub fn preset(&self,name:&str)->Result<Preset>{
        if let Some(p)=presets::builtin(name){return Ok(p);}
        self.presets.get(name).cloned().with_context(||format!("unknown notification preset: {name}"))
    }
    pub fn decode(bytes:&[u8])->Result<Self>{
        if bytes.len()>MAX_BYTES{bail!("notification preferences exceed 64 KiB");}
        let value:Self=toml::from_str(std::str::from_utf8(bytes)?)?;
        if value.applications.len()>128||value.presets.len()>64{bail!("notification preferences allow up to 128 applications and 64 custom presets");}
        for name in value.applications.keys(){
            if name.trim().is_empty()||name.len()>256||name.chars().any(char::is_control){bail!("invalid application name in notification preferences");}
        }
        for (name,preset) in &value.presets{
            if name.is_empty()||name.len()>48||!name.bytes().all(|b|b.is_ascii_lowercase()||b.is_ascii_digit()||b==b'-')||presets::builtin(name).is_some(){
                bail!("invalid or reserved custom notification preset name: {name}");
            }
            preset.validate()?;
        }
        Ok(value)
    }
    pub fn load(path:&Path)->Result<Self>{
        let file=match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path){
            Ok(file)=>file,
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(Self::default()),
            Err(error)=>return Err(error.into()),
        };
        let metadata=file.metadata()?;
        if !metadata.is_file()||metadata.len()>MAX_BYTES as u64{bail!("notification preferences must be a regular file up to 64 KiB");}
        let mut bytes=Vec::new();file.take(MAX_BYTES as u64+1).read_to_end(&mut bytes)?;
        Self::decode(&bytes)
    }
}
pub fn path()->Result<PathBuf>{Ok(atelier_config::config_path()?.with_file_name("notifications.toml"))}

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
