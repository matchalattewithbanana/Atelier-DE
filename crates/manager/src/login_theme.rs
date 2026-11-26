//! Inspect and stage local SDDM theme trees as inert, user-owned data.
//! This module must never run QML, invoke a greeter, write /etc or enable services.
use anyhow::{bail,Context,Result};
use atelier_config::wallpaper::{self,Raster};
use serde::Serialize;
use sha2::{Digest,Sha256};
use std::{collections::BTreeMap,fs::{self,File,OpenOptions},io::{self,IsTerminal,Read,Write},
    os::unix::fs::{MetadataExt,OpenOptionsExt,PermissionsExt},path::{Component,Path,PathBuf}};

const MAX_BYTES:usize=64*1024*1024;
const MAX_FILE:usize=32*1024*1024;
const MAX_ENTRIES:usize=512;
const MAX_DEPTH:usize=8;

fn relative(value:&str)->Result<PathBuf>{
    let path=Path::new(value);
    if value.is_empty()||value.len()>512||value.chars().any(char::is_control)||value.contains('\\')
        ||!path.components().all(|c|matches!(c,Component::Normal(_))){
        bail!("unsafe relative theme path");
    }
    Ok(path.into())
}
fn read_regular(path:&Path,limit:usize)->Result<Vec<u8>>{
    let f=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path)?;
    let m=f.metadata()?;
    if !m.is_file()||m.len()>limit as u64{bail!("nonregular or oversized theme file: {}",path.display());}
    let mut bytes=Vec::new();f.take(limit as u64+1).read_to_end(&mut bytes)?;
    if bytes.len()>limit{bail!("theme file exceeds limit");}
    Ok(bytes)
}

pub(super) struct Snapshot{files:BTreeMap<PathBuf,Vec<u8>>,total:usize,entries:usize}
impl Snapshot{
    pub(super) fn read(root:&Path)->Result<Self>{
        if !root.is_absolute()||!fs::symlink_metadata(root)?.is_dir(){bail!("choose an absolute real SDDM theme directory");}
        let mut s=Self{files:BTreeMap::new(),total:0,entries:0};
        s.walk(root,Path::new(""),0)?;
        Ok(s)
    }
    fn walk(&mut self,root:&Path,dir:&Path,depth:usize)->Result<()>{
        if depth>MAX_DEPTH{bail!("theme directory nesting exceeds limit");}
        for entry in fs::read_dir(root.join(dir))?{
            let entry=entry?;
            self.entries+=1;if self.entries>MAX_ENTRIES{bail!("theme has too many entries");}
            let name=entry.file_name();let name=name.to_str().context("non-UTF8 theme filename")?;
            let rel=dir.join(relative(name)?);
            if rel.to_str().context("non-UTF8 theme path")?.len()>512{bail!("theme path too long");}
            let kind=entry.file_type()?;
            if kind.is_dir(){self.walk(root,&rel,depth+1)?;}
            else if kind.is_file(){
                let bytes=read_regular(&entry.path(),MAX_FILE.min(MAX_BYTES-self.total))?;
                self.total+=bytes.len();self.files.insert(rel,bytes);
            }else{bail!("theme contains a symlink or special file: {}",rel.display());}
        }
        Ok(())
    }
    pub(super) fn fingerprint(&self)->String{
        let mut hash=Sha256::new();
        for (path,bytes) in &self.files{
            let path=path.to_str().expect("validated UTF8").as_bytes();
            hash.update((path.len() as u64).to_le_bytes());hash.update(path);
            hash.update((bytes.len() as u64).to_le_bytes());hash.update(bytes);
        }
        format!("{:x}",hash.finalize())
    }
    /// Used only after the administrator import boundary has validated the
    /// source and created a fresh private destination. No QML is executed.
    pub(super) fn copy_for_greeter(&self,dest:&Path)->Result<()>{
        self.metadata()?;
        fs::create_dir(dest)?;
        for (rel,bytes) in &self.files{
            let path=dest.join(rel);
            let parent=path.parent().context("missing import parent")?;
            fs::create_dir_all(parent)?;
            let mut file=OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
            file.write_all(bytes)?;
            file.set_permissions(fs::Permissions::from_mode(0o644))?;
            file.sync_all()?;
        }
        // Only directories beneath the fresh destination are changed.
        for rel in self.files.keys(){
            for dir in dest.join(rel).parent().context("missing import directory")?.ancestors(){
                if !dir.starts_with(dest){break;}
                fs::set_permissions(dir,fs::Permissions::from_mode(0o755))?;
                File::open(dir)?.sync_all()?;
                if dir==dest{break;}
            }
        }
        Ok(())
    }
    fn metadata(&self)->Result<Metadata>{
        let mut m=Metadata{main:"Main.qml".into(),screenshot:None,name:None};
        if let Some(bytes)=self.files.get(Path::new("metadata.desktop")){
            if bytes.len()>64*1024{bail!("theme metadata exceeds limit");}
            let text=std::str::from_utf8(bytes)?;
            let mut section=false;
            for line in text.lines().map(str::trim){
                if line.starts_with('['){section=line=="[SddmGreeterTheme]";continue;}
                if !section{continue;}
                let Some((key,value))=line.split_once('=')else{continue;};
                let value=value.trim();
                match key.trim(){
                    "MainScript"=>m.main=relative(value)?,
                    "Screenshot" if !value.is_empty()=>m.screenshot=Some(relative(value)?),
                    "Name" if !value.is_empty()&&!value.chars().any(char::is_control)=>m.name=Some(value.chars().take(128).collect()),
                    _=>{},
                }
            }
        }
        if !self.files.contains_key(&m.main)||m.main.extension().and_then(|s|s.to_str())!=Some("qml"){
            bail!("theme does not contain the declared QML entry point");
        }
        Ok(m)
    }
}
struct Metadata{main:PathBuf,screenshot:Option<PathBuf>,name:Option<String>}

#[derive(Debug,Serialize)]
pub(super) struct Inspection{
    pub source:PathBuf,pub name:String,pub fingerprint:String,pub bytes:usize,pub files:Vec<PathBuf>,
    pub main:PathBuf,pub screenshot:Option<PathBuf>,pub preview_status:String,
}
pub(super) struct Review{pub info:Inspection,pub preview:Option<Raster>,_temporary_source:Option<tempfile::TempDir>}
fn inspect_snapshot(source:&Path,s:&Snapshot)->Result<Review>{
    let m=s.metadata()?;
    let screenshot=m.screenshot.or_else(||["preview.png","screenshot.png","preview.jpg","screenshot.jpg","preview.webp"]
        .into_iter().map(PathBuf::from).find(|p|s.files.contains_key(p)));
    let (preview,preview_status)=match &screenshot{
        Some(path)=>match s.files.get(path).context("declared screenshot not present")
            .and_then(|bytes|wallpaper::decode_bytes(bytes,1024)){
            Ok(image)=>(Some(image),"Static theme-provided screenshot; not a running greeter.".into()),
            Err(e)=>(None,format!("Screenshot unavailable: {e:#}")),
        },
        None=>(None,"No supported static screenshot found.".into()),
    };
    let name=m.name.unwrap_or_else(||source.file_name().and_then(|s|s.to_str()).unwrap_or("Theme").chars().filter(|c|!c.is_control()).take(128).collect());
    Ok(Review{info:Inspection{source:source.into(),name,fingerprint:s.fingerprint(),bytes:s.total,
        files:s.files.keys().cloned().collect(),main:m.main,screenshot,preview_status},preview,_temporary_source:None})
}
/// Validate the declared local QML entry point without decoding any raster.
/// Used by the administrator selection review only after ownership checks.
pub(super) fn validate_entrypoint(source:&Path)->Result<()>{
    Snapshot::read(source)?.metadata()?;Ok(())
}
pub(super) fn inspect(source:&Path)->Result<Review>{
    // Normalize the source once. Refuse symlink at the selected root.
    if !source.is_absolute()||!fs::symlink_metadata(source)?.is_dir(){bail!("choose an absolute real SDDM theme directory");}
    let source=source.canonicalize()?;
    inspect_snapshot(&source,&Snapshot::read(&source)?)
}
fn root()->Result<PathBuf>{
    let data=std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p|p.is_absolute())
        .or_else(||std::env::var_os("HOME").map(PathBuf::from).filter(|p|p.is_absolute()).map(|p|p.join(".local/share")))
        .context("no absolute user data directory")?;
    Ok(data.join("atelier-manager/sddm-staged"))
}
impl Review{
    pub(super) fn retain_source(mut self,source:tempfile::TempDir)->Self{self._temporary_source=Some(source);self}
    pub(super) fn phrase(&self)->String{format!("prepare {}",&self.info.fingerprint[..12])}
    pub(super) fn detail(&self)->Result<String>{
        Ok(format!("{}\n\nFiles will be copied to a private user-local staging directory only. No QML execution, system installation or login changes.",serde_json::to_string_pretty(&self.info)?))
    }
    pub(super) fn apply(self,typed:&str)->Result<PathBuf>{
        if nix::unistd::Uid::effective().is_root(){bail!("run as the desktop user, not root");}
        if typed!=self.phrase(){bail!("confirmation does not match review");}
        self.stage_at(&root()?)
    }
    fn stage_at(self,root:&Path)->Result<PathBuf>{
        let snapshot=Snapshot::read(&self.info.source)?;
        if snapshot.fingerprint()!=self.info.fingerprint{bail!("theme changed after review; inspect it again");}
        fs::create_dir_all(root)?;
        let m=fs::symlink_metadata(root)?;
        if !m.is_dir()||m.uid()!=nix::unistd::Uid::effective().as_raw()||m.permissions().mode()&0o022!=0{
            bail!("unsafe SDDM staging directory");
        }
        let pending=tempfile::Builder::new().prefix("theme-").tempdir_in(root)?;
        let theme=pending.path().join("theme");fs::create_dir(&theme)?;
        for (rel,bytes) in snapshot.files{
            let path=theme.join(rel);
            fs::create_dir_all(path.parent().context("missing staging parent")?)?;
            let mut f=OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
            f.write_all(&bytes)?;f.sync_all()?;
        }
        let mut record=OpenOptions::new().write(true).create_new(true).mode(0o600).open(pending.path().join("review.json"))?;
        record.write_all(&serde_json::to_vec_pretty(&self.info)?)?;record.sync_all()?;
        File::open(pending.path())?.sync_all()?;
        Ok(pending.keep())
    }
}
pub(super) fn run(args:&[String])->Result<()>{
    if let [action,id]=args{if action=="pack-prepare"{return crate::login_packs::prepare(id);}}
    if args.first().is_some_and(|a|matches!(a.as_str(),"install-plan"|"install"|"selection-plan"|"select"|"rollback-selection")){
        return crate::login_selection::run(args);
    }
    let (prepare,path)=match args{
        [action,flag,path] if flag=="--from"&&matches!(action.as_str(),"inspect"|"prepare")=>(action=="prepare",path),
        _=>bail!("use login-theme inspect --from DIR | prepare --from DIR | pack-prepare ID"),
    };
    let review=inspect(Path::new(path))?;
    println!("{}",review.detail()?);
    if !prepare{return Ok(());}
    if nix::unistd::Uid::effective().is_root()||!io::stdin().is_terminal()||!io::stdout().is_terminal(){bail!("interactive desktop-user confirmation required");}
    let phrase=review.phrase();print!("Type '{phrase}': ");io::stdout().flush()?;
    let mut typed=String::new();io::stdin().read_line(&mut typed)?;
    if typed.trim()!=phrase{println!("Cancelled. No changes.");return Ok(());}
    println!("Prepared inert theme copy: {}",review.apply(typed.trim())?.display());
    Ok(())
}
#[cfg(test)]
mod tests{
    use super::*;
    fn theme(root:&Path){
        fs::write(root.join("Main.qml"),b"// inert sample").unwrap();
        fs::write(root.join("metadata.desktop"),"[SddmGreeterTheme]\nName=Test\nMainScript=Main.qml\nScreenshot=preview.png\n").unwrap();
        fs::write(root.join("preview.png"),b"not a raster").unwrap();
    }
    #[test]
    fn inspect_stage_and_reject_changed_source(){
        let source=tempfile::tempdir().unwrap();let dest=tempfile::tempdir().unwrap();theme(source.path());
        let first=inspect(source.path()).unwrap();let stale=inspect(source.path()).unwrap();
        assert_eq!(first.info.name,"Test");assert_eq!(first.info.files.len(),3);assert!(first.preview.is_none());
        let staged=first.stage_at(dest.path()).unwrap();
        assert_eq!(fs::read(staged.join("theme/Main.qml")).unwrap(),b"// inert sample");
        assert!(staged.join("review.json").is_file());
        fs::write(source.path().join("Main.qml"),b"changed").unwrap();
        assert!(stale.stage_at(dest.path()).is_err());
        assert_eq!(fs::read_dir(dest.path()).unwrap().count(),1);
    }
    #[test]
    fn rejects_symlink_traversal_and_missing_entrypoint(){
        let dir=tempfile::tempdir().unwrap();theme(dir.path());
        std::os::unix::fs::symlink("Main.qml",dir.path().join("alias.qml")).unwrap();
        assert!(inspect(dir.path()).is_err());fs::remove_file(dir.path().join("alias.qml")).unwrap();
        fs::write(dir.path().join("metadata.desktop"),"[SddmGreeterTheme]\nMainScript=../bad.qml\n").unwrap();
        assert!(inspect(dir.path()).is_err());
        fs::write(dir.path().join("metadata.desktop"),"[SddmGreeterTheme]\nMainScript=missing.qml\n").unwrap();
        assert!(inspect(dir.path()).is_err());
    }
}
