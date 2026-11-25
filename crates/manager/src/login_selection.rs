//! Narrow, explicit administrator transaction for an already installed SDDM
//! theme. Never launch QML, copy user themes into system directories, alter
//! authentication, switch display managers, or restart a service.
use anyhow::{bail,Context,Result};
use fs2::FileExt;
use serde::{Deserialize,Serialize};
use sha2::{Digest,Sha256};
use std::{fs::{self,File,OpenOptions},io::{self,IsTerminal,Read,Write},
    os::unix::fs::{MetadataExt,OpenOptionsExt,PermissionsExt},path::{Path,PathBuf}};

const MANAGED:&str="99-atelier-theme.conf";
const CHECKPOINT:&str=".atelier-theme-recovery.json";
const LOCK:&str=".atelier-theme.lock";
const MAX_CONFIG:usize=64*1024;
const MAX_JOURNAL:usize=256*1024;

struct Location{dropins:PathBuf,legacy:PathBuf,themes:PathBuf,owner:u32}
impl Location{
    fn system()->Self{Self{dropins:"/etc/sddm.conf.d".into(),legacy:"/etc/sddm.conf".into(),
        themes:"/usr/share/sddm/themes".into(),owner:0}}
    fn safe(&self,m:&fs::Metadata)->bool{m.uid()==self.owner&&m.permissions().mode()&0o022==0}
    fn directory(&self,path:&Path)->Result<()>{
        let m=fs::symlink_metadata(path)?;
        if !m.is_dir()||!self.safe(&m){bail!("unsafe directory: {}",path.display());}Ok(())
    }
    fn protected_ancestors(&self,path:&Path)->Result<()>{
        if !path.is_absolute(){bail!("expected absolute system path");}
        for p in path.ancestors(){self.directory(p)?;}Ok(())
    }
    fn read(&self,path:&Path,limit:usize)->Result<Option<Vec<u8>>>{
        let f=match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path){
            Ok(f)=>f,Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(None),Err(e)=>return Err(e.into()),
        };
        let m=f.metadata()?;
        if !m.is_file()||!self.safe(&m)||m.len()>limit as u64{bail!("unsafe or oversized file: {}",path.display());}
        let mut bytes=Vec::new();f.take(limit as u64+1).read_to_end(&mut bytes)?;
        if bytes.len()>limit{bail!("configuration exceeds size limit");}Ok(Some(bytes))
    }
    fn managed(&self)->Result<Option<Vec<u8>>>{
        let bytes=self.read(&self.dropins.join(MANAGED),MAX_CONFIG)?;
        if let Some(bytes)=&bytes{parse_managed(bytes)?;}
        Ok(bytes)
    }
    fn check_precedence(&self)->Result<()>{
        self.protected_ancestors(&self.dropins)?;
        // The legacy file has highest precedence. Refuse any non-comment
        // content instead of attempting to emulate Qt's full INI semantics.
        if let Some(bytes)=self.read(&self.legacy,MAX_CONFIG)?{
            if std::str::from_utf8(&bytes)?.lines().map(str::trim)
                .any(|line|!line.is_empty()&&!line.starts_with('#')&&!line.starts_with(';')){
                bail!("nonempty /etc/sddm.conf has higher precedence; review it manually first");
            }
        }
        for (index,entry) in fs::read_dir(&self.dropins)?.enumerate(){
            if index>=512{bail!("too many SDDM drop-ins");}
            let entry=entry?;
            let name=entry.file_name();let name=name.to_str().context("non-UTF8 SDDM drop-in name")?;
            if name.ends_with(".conf")&&name!=MANAGED&&name>MANAGED{
                bail!("later SDDM drop-in may override Atelier: {name}");
            }
        }
        Ok(())
    }
    fn theme_fingerprint(&self,name:&str)->Result<String>{
        if !valid_name(name){bail!("theme name must be a local installed directory name");}
        self.protected_ancestors(&self.themes)?;
        let path=self.themes.join(name);self.directory(&path)?;
        let mut count=0;let mut total=0;let mut hash=Sha256::new();
        self.hash_tree(&path,&path,0,&mut count,&mut total,&mut hash)?;
        // Existing local inspection also checks the declared QML entry point.
        let review=crate::login_theme::inspect(&path)?;
        hash.update(review.info.fingerprint.as_bytes());
        Ok(format!("{:x}",hash.finalize()))
    }
    fn hash_tree(&self,base:&Path,dir:&Path,depth:usize,count:&mut usize,total:&mut usize,hash:&mut Sha256)->Result<()>{
        if depth>8{bail!("theme nesting exceeds limit");}
        self.directory(dir)?;
        let mut entries=Vec::new();
        for entry in fs::read_dir(dir)?{
            *count+=1;if *count>512{bail!("theme exceeds entry limit");}
            entries.push(entry?);
        }
        entries.sort_by_key(|e|e.file_name());
        for entry in entries{
            let path=entry.path();
            let m=fs::symlink_metadata(&path)?;
            if !self.safe(&m){bail!("theme contains non-administrator-owned or writable files");}
            let rel=path.strip_prefix(base)?.to_str().context("non-UTF8 theme path")?;
            if rel.len()>512||rel.chars().any(char::is_control){bail!("unsafe theme path");}
            hash.update((rel.len() as u64).to_le_bytes());hash.update(rel.as_bytes());
            hash.update(m.permissions().mode().to_le_bytes());
            if m.is_dir(){self.hash_tree(base,&path,depth+1,count,total,hash)?;}
            else if m.is_file(){
                // Also require readability by an ordinary greeter user.
                if m.permissions().mode()&0o004==0{bail!("theme file is not world-readable: {rel}");}
                let bytes=self.read(&path,(64*1024*1024-*total).min(32*1024*1024))?.context("theme file disappeared")?;
                *total+=bytes.len();hash.update((bytes.len() as u64).to_le_bytes());hash.update(bytes);
            }else{bail!("theme contains a symlink or special file: {rel}");}
        }
        Ok(())
    }
    fn journal(&self)->Result<Option<Checkpoint>>{
        let Some(bytes)=self.read(&self.dropins.join(CHECKPOINT),MAX_JOURNAL)?else{return Ok(None);};
        let c:Checkpoint=serde_json::from_slice(&bytes)?;c.validate()?;Ok(Some(c))
    }
    fn lock(&self)->Result<File>{
        self.protected_ancestors(&self.dropins)?;
        let f=OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(self.dropins.join(LOCK))?;
        let m=f.metadata()?;
        if !m.is_file()||!self.safe(&m){bail!("unsafe SDDM selection lock");}
        FileExt::lock_exclusive(&f)?;Ok(f)
    }
    fn replace(&self,name:&str,bytes:&[u8],mode:u32)->Result<()>{
        let mut temp=tempfile::NamedTempFile::new_in(&self.dropins)?;
        temp.write_all(bytes)?;
        temp.as_file().set_permissions(fs::Permissions::from_mode(mode))?;
        temp.as_file().sync_all()?;
        temp.persist(self.dropins.join(name))?;
        File::open(&self.dropins)?.sync_all()?;Ok(())
    }
    fn restore(&self,previous:Option<&[u8]>)->Result<()>{
        match previous{
            Some(bytes)=>self.replace(MANAGED,bytes,0o644),
            None=>{fs::remove_file(self.dropins.join(MANAGED))?;File::open(&self.dropins)?.sync_all()?;Ok(())}
        }
    }
}

fn valid_name(name:&str)->bool{
    !name.is_empty()&&name.len()<=128&&name!="."&&name!=".."&&
        name.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_'|b'.'))
}
fn managed_bytes(name:&str)->Vec<u8>{
    format!("# Managed by Atelier; use login-theme rollback-selection\n[Theme]\nThemeDir=/usr/share/sddm/themes\nCurrent={name}\n").into_bytes()
}
fn parse_managed(bytes:&[u8])->Result<&str>{
    let text=std::str::from_utf8(bytes)?;
    let name=text.lines().last().and_then(|s|s.strip_prefix("Current=")).context("unknown managed config format")?;
    if !valid_name(name)||managed_bytes(name)!=bytes{bail!("existing Atelier drop-in has unexpected content; refusing overwrite");}
    Ok(name)
}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint{schema:u32,previous:Option<Vec<u8>>,selected:Vec<u8>}
impl Checkpoint{
    fn validate(&self)->Result<()>{
        if self.schema!=1{bail!("unknown SDDM recovery format");}
        parse_managed(&self.selected)?;
        if let Some(bytes)=&self.previous{parse_managed(bytes)?;}
        Ok(())
    }
}
struct Plan{name:String,fingerprint:String,previous:Option<Vec<u8>>}
impl Plan{
    fn review(location:&Location,name:&str)->Result<Self>{
        location.check_precedence()?;
        let previous=location.managed()?;
        let fingerprint=location.theme_fingerprint(name)?;
        Ok(Self{name:name.into(),fingerprint,previous})
    }
    fn detail(&self)->Result<String>{
        let before=self.previous.as_deref().map(parse_managed).transpose()?.unwrap_or("(no Atelier override)");
        Ok(format!("Installed theme: {}\nTheme fingerprint: {}\nPrevious Atelier selection: {}\nChange only /etc/sddm.conf.d/{}\nSet ThemeDir=/usr/share/sddm/themes and Current={}\nNo service restart, login-manager switch, QML execution or PAM change.\nTrust and greeter compatibility must be checked independently.",self.name,self.fingerprint,before,MANAGED,self.name))
    }
    fn commit(self,location:&Location)->Result<bool>{
        let _lock=location.lock()?;
        let now=Self::review(location,&self.name)?;
        if now.previous!=self.previous||now.fingerprint!=self.fingerprint{bail!("configuration or theme changed; review again");}
        let selected=managed_bytes(&self.name);
        if self.previous.as_deref()==Some(selected.as_slice()){return Ok(false);}
        if let Some(old)=location.journal()?{
            // Only replace an earlier checkpoint if its transaction is in a
            // recognized before/after state, not an externally modified state.
            if self.previous.as_deref()!=Some(old.selected.as_slice())&&self.previous!=old.previous{
                bail!("existing recovery checkpoint disagrees with current selection");
            }
        }
        let checkpoint=Checkpoint{schema:1,previous:self.previous,selected};
        // Checkpoint first: interruption before the config write leaves an
        // identifiable previous state, never an unjournaled new selection.
        location.replace(CHECKPOINT,&serde_json::to_vec_pretty(&checkpoint)?,0o600)?;
        location.replace(MANAGED,&checkpoint.selected,0o644)?;
        Ok(true)
    }
}
fn rollback_plan(location:&Location)->Result<Checkpoint>{
    location.protected_ancestors(&location.dropins)?;
    let checkpoint=location.journal()?.context("no Atelier selection checkpoint")?;
    let current=location.managed()?;
    if current.as_deref()!=Some(checkpoint.selected.as_slice()){
        if current==checkpoint.previous{bail!("previous selection is already present; nothing to restore");}
        bail!("managed configuration changed since selection; refusing rollback");
    }
    Ok(checkpoint)
}
fn rollback(location:&Location,review:Checkpoint)->Result<()>{
    let _lock=location.lock()?;
    if rollback_plan(location)?!=review{bail!("recovery changed after review");}
    // Refuse restoring a removed or modified-ownership prior theme.
    if let Some(bytes)=&review.previous{location.theme_fingerprint(parse_managed(bytes)?)?;}
    location.restore(review.previous.as_deref())
}
fn administrator()->Result<()>{
    if !nix::unistd::Uid::effective().is_root(){bail!("requires a separately installed, administrator-owned Manager binary run as root; no automatic sudo");}
    if !io::stdin().is_terminal()||!io::stdout().is_terminal(){bail!("interactive administrator review required");}
    let exe=std::env::current_exe()?.canonicalize()?;
    let loc=Location::system();loc.protected_ancestors(exe.parent().context("missing executable parent")?)?;
    let m=fs::symlink_metadata(&exe)?;
    if !m.is_file()||!loc.safe(&m){bail!("refusing privileged execution from an untrusted binary location");}
    Ok(())
}
fn confirm(phrase:&str)->Result<bool>{
    print!("Type '{phrase}': ");io::stdout().flush()?;
    let mut typed=String::new();io::stdin().read_line(&mut typed)?;Ok(typed.trim()==phrase)
}
pub(super) fn run(args:&[String])->Result<()>{
    let location=Location::system();
    match args{
        [action,name] if action=="selection-plan"=>{
            println!("{}",Plan::review(&location,name)?.detail()?);
            println!("Preview only. No changes.");Ok(())
        }
        [action,name] if action=="select"=>{
            administrator()?;
            let plan=Plan::review(&location,name)?;
            println!("{}",plan.detail()?);
            println!("Only proceed for a theme you trust, after checking greeter compatibility and preparing spare TTY recovery.");
            if !confirm(&format!("select trusted {name}"))?{println!("Cancelled.");return Ok(());}
            println!("{}",if plan.commit(&location)?{"Saved reviewed override and recovery checkpoint. Applies on a future greeter start."}else{"Selection already saved."});Ok(())
        }
        [action] if action=="rollback-selection"=>{
            administrator()?;
            let review=rollback_plan(&location)?;
            println!("Restore prior Atelier override: {}. No service restart.",review.previous.as_deref().map(parse_managed).transpose()?.unwrap_or("(remove Atelier override)"));
            if !confirm("restore login selection")?{println!("Cancelled.");return Ok(());}
            rollback(&location,review)?;println!("Previous Atelier selection restored.");Ok(())
        }
        _=>bail!("use login-theme selection-plan NAME | select NAME | rollback-selection"),
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    fn fixture(root:&Path)->Location{
        let loc=Location{dropins:root.join("conf.d"),legacy:root.join("sddm.conf"),themes:root.join("themes"),owner:nix::unistd::Uid::effective().as_raw()};
        fs::create_dir(&loc.dropins).unwrap();fs::create_dir(&loc.themes).unwrap();loc
    }
    #[test]
    fn managed_format_is_exact(){
        assert_eq!(parse_managed(&managed_bytes("breeze")).unwrap(),"breeze");
        for name in ["../evil","a/b","a\n[Autologin]","..",""]{assert!(!valid_name(name));}
        let mut bytes=managed_bytes("breeze");bytes.extend(b"extra\n");assert!(parse_managed(&bytes).is_err());
    }
    #[test]
    fn journal_and_atomic_restore_preserve_other_files(){
        let root=tempfile::tempdir().unwrap();let loc=fixture(root.path());
        fs::write(loc.dropins.join("10-other.conf"),b"[General]\nNumlock=on\n").unwrap();
        let old=managed_bytes("breeze");let new=managed_bytes("other");
        let journal=Checkpoint{schema:1,previous:Some(old.clone()),selected:new.clone()};
        loc.replace(CHECKPOINT,&serde_json::to_vec(&journal).unwrap(),0o600).unwrap();
        loc.replace(MANAGED,&new,0o644).unwrap();
        assert_eq!(loc.journal().unwrap(),Some(journal));assert_eq!(loc.managed().unwrap(),Some(new));
        loc.restore(Some(&old)).unwrap();assert_eq!(loc.managed().unwrap(),Some(old));
        loc.restore(None).unwrap();assert_eq!(loc.managed().unwrap(),None);
        assert_eq!(fs::read(loc.dropins.join("10-other.conf")).unwrap(),b"[General]\nNumlock=on\n");
    }
    #[test]
    fn refuses_symlink_and_foreign_managed_config(){
        let root=tempfile::tempdir().unwrap();let loc=fixture(root.path());
        fs::write(loc.dropins.join(MANAGED),b"[Theme]\nCurrent=external\n").unwrap();
        assert!(loc.managed().is_err());fs::remove_file(loc.dropins.join(MANAGED)).unwrap();
        std::os::unix::fs::symlink("/etc/passwd",loc.dropins.join(MANAGED)).unwrap();
        assert!(loc.managed().is_err());
    }
}
