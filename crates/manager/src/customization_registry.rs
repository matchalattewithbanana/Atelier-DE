//! Reviewed metadata-only registry; never execute manifests or install assets.
use anyhow::{bail,Context,Result};
use fs2::FileExt;
use serde::{Deserialize,Serialize};
use std::{collections::BTreeSet,fs::{self,File,OpenOptions},io::{self,IsTerminal,Read,Write},
    os::unix::fs::{MetadataExt,OpenOptionsExt,PermissionsExt},path::{Path,PathBuf}};
use crate::customization::{Item,CATEGORIES};

const MAX_BYTES:usize=256*1024;
const MAX_MANIFEST:usize=16*1024;
const MAX_RECORDS:usize=64;
const TARGETS:&[&str]=&["atelier","mango","hyprland","niri","other"];

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest{
    schema:u32,
    id:String,
    name:String,
    category:String,
    source:String,
    #[serde(default)]
    targets:Vec<String>,
    #[serde(default)]
    description:String,
}
fn valid_id(id:&str)->bool{
    (2..=48).contains(&id.len())&&id.as_bytes()[0].is_ascii_lowercase()&&
        id.bytes().all(|b|b.is_ascii_lowercase()||b.is_ascii_digit()||matches!(b,b'-'|b'_'))
}
fn clean(value:&str,limit:usize)->bool{
    !value.is_empty()&&value.len()<=limit&&!value.chars().any(char::is_control)
}
impl Manifest{
    fn validate(&self)->Result<()>{
        if self.schema!=1||!valid_id(&self.id)||!clean(&self.name,128)||!CATEGORIES.contains(&self.category.as_str()){
            bail!("invalid custom component schema, ID, name or category");
        }
        if !clean(&self.source,1024)||
            !(Path::new(&self.source).is_absolute()||
                self.source.strip_prefix("https://").is_some_and(|s|!s.is_empty()&&!s.chars().any(char::is_whitespace))){
            bail!("source must be an absolute local path or HTTPS reference");
        }
        if self.description.len()>1024||self.description.chars().any(char::is_control){bail!("invalid description");}
        let mut seen=BTreeSet::new();
        if self.targets.len()>TARGETS.len()||self.targets.iter().any(|t|!TARGETS.contains(&t.as_str())||!seen.insert(t)){
            bail!("invalid or duplicate declared target");
        }
        Ok(())
    }
    fn item(&self)->Item{
        let category=*CATEGORIES.iter().find(|c|**c==self.category.as_str()).expect("validated category");
        Item{id:format!("custom:{}",self.id),category,name:self.name.clone(),source:self.source.clone(),
            declared_targets:self.targets.clone(),scope:"metadata-only",
            install:"not implemented; use separately trusted component-specific workflow",
            activation:"not implemented; registration does not establish compatibility",
            readiness:"Custom reference".into(),detail:format!("{}. Declared targets: {}. No source fetched or executed.",
                self.description,if self.targets.is_empty(){"unspecified".into()}else{self.targets.join(", ")})}
    }
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry{schema:u32,entries:Vec<Manifest>}
impl Default for Registry{fn default()->Self{Self{schema:1,entries:vec![]}}}
impl Registry{
    fn validate(&self)->Result<()>{
        if self.schema!=1||self.entries.len()>MAX_RECORDS{bail!("invalid registry schema or size");}
        let mut ids=BTreeSet::new();
        for entry in &self.entries{
            entry.validate()?;
            if !ids.insert(&entry.id){bail!("duplicate custom component ID");}
        }
        Ok(())
    }
    fn decode(bytes:Option<&[u8]>)->Result<Self>{
        let value=match bytes{Some(bytes)=>serde_json::from_slice::<Self>(bytes)?,None=>Self::default()};
        value.validate()?;Ok(value)
    }
    fn encode(&self)->Result<Vec<u8>>{
        self.validate()?;
        let bytes=serde_json::to_vec_pretty(self)?;
        if bytes.len()>MAX_BYTES{bail!("registry too large");}
        Ok(bytes)
    }
}

fn root()->Result<PathBuf>{
    let base=std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p|p.is_absolute())
        .or_else(||std::env::var_os("HOME").map(PathBuf::from).filter(|p|p.is_absolute()).map(|p|p.join(".config")))
        .context("no absolute user configuration directory")?;
    Ok(base.join("atelier-manager/customizations"))
}
fn owned(m:&fs::Metadata)->bool{
    m.uid()==nix::unistd::Uid::effective().as_raw()&&m.permissions().mode()&0o022==0
}
fn directory(path:&Path)->Result<()>{
    let m=fs::symlink_metadata(path)?;
    if !m.is_dir()||!owned(&m){bail!("unsafe customization registry directory");}
    Ok(())
}
fn read_file(path:&Path,limit:usize,require_owned:bool)->Result<Option<Vec<u8>>>{
    let file=match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path){
        Ok(f)=>f,Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(None),Err(e)=>return Err(e.into()),
    };
    let m=file.metadata()?;
    if !m.is_file()||(require_owned&&!owned(&m))||m.len()>limit as u64{bail!("unsafe or oversized registry/manifest file");}
    let mut bytes=Vec::new();file.take(limit as u64+1).read_to_end(&mut bytes)?;
    if bytes.len()>limit{bail!("registry/manifest exceeds size limit");}
    Ok(Some(bytes))
}
fn read_state(root:&Path,name:&str)->Result<Option<Vec<u8>>>{
    match fs::symlink_metadata(root){
        Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(None),
        Err(e)=>return Err(e.into()),Ok(_)=>directory(root)?,
    }
    read_file(&root.join(name),MAX_BYTES,true)
}
fn replace(root:&Path,name:&str,bytes:&[u8])->Result<()>{
    let mut file=tempfile::NamedTempFile::new_in(root)?;
    file.write_all(bytes)?;file.as_file().sync_all()?;file.persist(root.join(name))?;
    File::open(root)?.sync_all()?;Ok(())
}
pub(super) fn items()->Result<Vec<Item>>{
    Ok(Registry::decode(read_state(&root()?,"registry.json")?.as_deref())?.entries.iter().map(Manifest::item).collect())
}

#[derive(Clone,Copy)]
pub(super) enum Action{Register,Remove,Restore}
pub(super) struct Review{
    root:PathBuf,current:Option<Vec<u8>>,previous:Option<Vec<u8>>,next:Registry,
    phrase:String,detail:String,
}
impl Review{
    pub(super) fn detail(&self)->&str{&self.detail}
    pub(super) fn phrase(&self)->&str{&self.phrase}
    pub(super) fn apply(self,typed:&str)->Result<String>{
        if nix::unistd::Uid::effective().is_root(){bail!("run as your desktop user, not root");}
        if typed!=self.phrase{bail!("confirmation does not match reviewed operation");}
        self.commit()
    }
    fn commit(self)->Result<String>{
        fs::create_dir_all(&self.root)?;directory(&self.root)?;
        let lock=OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(self.root.join("registry.lock"))?;
        let m=lock.metadata()?;
        if !m.is_file()||!owned(&m){bail!("unsafe customization registry lock");}
        FileExt::lock_exclusive(&lock)?;
        if read_state(&self.root,"registry.json")?!=self.current||
            read_state(&self.root,"registry.previous.json")?!=self.previous{
            bail!("registry changed after review; review again");
        }
        let next=self.next.encode()?;
        let current=Registry::decode(self.current.as_deref())?;
        if current==self.next{return Ok("No registry change.".into());}
        let backup=match self.current{Some(bytes)=>bytes,None=>current.encode()?};
        replace(&self.root,"registry.previous.json",&backup)?;
        replace(&self.root,"registry.json",&next)?;
        Ok("Registry saved; previous version retained. No component files or session settings changed.".into())
    }
}

fn review_at(root:PathBuf,action:Action,value:&str)->Result<Review>{
    let current=read_state(&root,"registry.json")?;
    let previous=read_state(&root,"registry.previous.json")?;
    let mut next=Registry::decode(current.as_deref())?;
    // Invalid existing backup must not be silently overwritten.
    if let Some(bytes)=&previous{Registry::decode(Some(bytes))?;}
    let (phrase,detail)=match action{
        Action::Register=>{
            let path=Path::new(value);
            if !path.is_absolute(){bail!("manifest path must be absolute");}
            let bytes=read_file(path,MAX_MANIFEST,false)?.context("manifest not found")?;
            let entry:Manifest=serde_json::from_slice(&bytes)?;
            entry.validate()?;
            if next.entries.iter().any(|e|e.id==entry.id){bail!("ID already registered; unregister explicitly first");}
            let phrase=format!("register custom:{}",entry.id);
            let detail=format!("Register metadata only:\n{}",serde_json::to_string_pretty(&entry)?);
            next.entries.push(entry);(phrase,detail)
        }
        Action::Remove=>{
            let id=value.strip_prefix("custom:").context("use custom:ID from catalog")?;
            let index=next.entries.iter().position(|e|e.id==id).context("custom component not registered")?;
            let entry=next.entries.remove(index);
            (format!("unregister custom:{id}"),format!("Remove reference only:\n{}",serde_json::to_string_pretty(&entry)?))
        }
        Action::Restore=>{
            let bytes=previous.as_ref().context("no previous registry backup")?;
            next=Registry::decode(Some(bytes))?;
            ("restore registry".into(),format!("Restore these references:\n{}",serde_json::to_string_pretty(&next)?))
        }
    };
    next.entries.sort_by(|a,b|a.id.cmp(&b.id));next.encode()?;
    Ok(Review{root,current,previous,next,phrase,detail})
}
pub(super) fn review(action:Action,value:&str)->Result<Review>{review_at(root()?,action,value)}
pub(super) fn run(args:&[String])->Result<()>{
    if nix::unistd::Uid::effective().is_root(){bail!("run as your desktop user, not root");}
    if !io::stdin().is_terminal()||!io::stdout().is_terminal(){bail!("interactive registry review required");}
    let (action,value)=match args{
        [a,flag,path] if a=="register"&&flag=="--from"=>(Action::Register,path.as_str()),
        [a,id] if a=="unregister"=>(Action::Remove,id.as_str()),
        [a] if a=="restore-registry"=>(Action::Restore,""),
        _=>bail!("use customizations register --from FILE | unregister custom:ID | restore-registry"),
    };
    let review=review(action,value)?;
    println!("{}\nNo download, installation, activation or service change.",review.detail());
    print!("Type '{}': ",review.phrase());io::stdout().flush()?;
    let mut typed=String::new();io::stdin().read_line(&mut typed)?;
    if typed.trim()!=review.phrase(){println!("Cancelled. No changes.");return Ok(());}
    println!("{}",review.apply(typed.trim())?);Ok(())
}

#[cfg(test)]
mod tests{
    use super::*;
    fn manifest()->Manifest{Manifest{schema:1,id:"my-pointer".into(),name:"My pointer".into(),category:"cursor".into(),
        source:"https://example.org/theme".into(),targets:vec!["atelier".into()],description:"User reference".into()}}
    #[test]
    fn validates_metadata_and_rejects_execution_fields(){
        let mut m=manifest();assert!(m.validate().is_ok());
        m.id="../escape".into();assert!(m.validate().is_err());m=manifest();
        m.targets.push("atelier".into());assert!(m.validate().is_err());m=manifest();
        m.source="file:relative".into();assert!(m.validate().is_err());m=manifest();
        let mut v=serde_json::to_value(m).unwrap();v["install_command"]="echo bad".into();
        assert!(serde_json::from_value::<Manifest>(v).is_err());
    }
    #[test]
    fn register_remove_restore_and_stale_review(){
        let dir=tempfile::tempdir().unwrap();let root=dir.path().join("registry");
        let path=dir.path().join("manifest.json");fs::write(&path,serde_json::to_vec(&manifest()).unwrap()).unwrap();
        let value=path.to_str().unwrap();
        let review=review_at(root.clone(),Action::Register,value).unwrap();
        let stale=review_at(root.clone(),Action::Register,value).unwrap();
        assert!(!root.exists());review.commit().unwrap();assert!(stale.commit().is_err());
        let state=Registry::decode(read_state(&root,"registry.json").unwrap().as_deref()).unwrap();
        assert_eq!(state.entries[0].item().id,"custom:my-pointer");
        assert!(review_at(root.clone(),Action::Register,value).is_err());
        review_at(root.clone(),Action::Remove,"custom:my-pointer").unwrap().commit().unwrap();
        assert!(Registry::decode(read_state(&root,"registry.json").unwrap().as_deref()).unwrap().entries.is_empty());
        review_at(root.clone(),Action::Restore,"").unwrap().commit().unwrap();
        assert_eq!(Registry::decode(read_state(&root,"registry.json").unwrap().as_deref()).unwrap().entries,vec![manifest()]);
    }
    #[test]
    fn refuses_symlink_state(){
        let dir=tempfile::tempdir().unwrap();let root=dir.path();
        let file=root.join("outside");fs::write(&file,b"{}").unwrap();
        std::os::unix::fs::symlink(&file,root.join("registry.json")).unwrap();
        assert!(read_state(root,"registry.json").is_err());
    }
}
