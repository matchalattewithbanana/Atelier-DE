//! Narrow administrator transactions for trusted SDDM theme imports and
//! selection. Never launch QML, read user-writable import sources as root,
//! alter authentication, switch display managers, or restart a service.
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

struct Location{dropins:PathBuf,legacy:PathBuf,themes:PathBuf,owner:u32,anchor:PathBuf}
impl Location{
    fn system()->Self{Self{dropins:"/etc/sddm.conf.d".into(),legacy:"/etc/sddm.conf".into(),
        themes:"/usr/share/sddm/themes".into(),owner:0,anchor:"/".into()}}
    fn safe(&self,m:&fs::Metadata)->bool{m.uid()==self.owner&&m.permissions().mode()&0o022==0}
    fn directory(&self,path:&Path)->Result<()>{
        let m=fs::symlink_metadata(path)?;
        if !m.is_dir()||!self.safe(&m){bail!("unsafe directory: {}",path.display());}Ok(())
    }
    fn protected_ancestors(&self,path:&Path)->Result<()>{
        if !path.is_absolute()||!path.starts_with(&self.anchor){bail!("path outside protected system boundary");}
        for p in path.ancestors(){self.directory(p)?;if p==self.anchor{return Ok(());}}
        bail!("missing protected boundary")
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
            if name.to_ascii_lowercase().ends_with(".conf")&&name!=MANAGED&&(name>MANAGED||name.to_ascii_lowercase().as_str()>=MANAGED){
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
        self.hash_tree(&path,&path,0,&mut count,&mut total,&mut hash,true)?;
        // Validate only entry-point metadata as administrator. Screenshot
        // decoding belongs exclusively to the unprivileged preview workflow.
        crate::login_theme::validate_entrypoint(&path)?;
        Ok(format!("{:x}",hash.finalize()))
    }
    fn hash_tree(&self,base:&Path,dir:&Path,depth:usize,count:&mut usize,total:&mut usize,hash:&mut Sha256,greeter_access:bool)->Result<()>{
        if depth>8{bail!("theme nesting exceeds limit");}
        self.directory(dir)?;
        if greeter_access&&fs::symlink_metadata(dir)?.permissions().mode()&0o005!=0o005{
            bail!("theme directory is not accessible to the greeter");
        }
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
            if m.is_dir(){self.hash_tree(base,&path,depth+1,count,total,hash,greeter_access)?;}
            else if m.is_file(){
                // Also require readability by an ordinary greeter user.
                if greeter_access&&m.permissions().mode()&0o004==0{bail!("theme file is not world-readable: {rel}");}
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
// The source must already have been placed under administrator control.
// Do not grant the privileged process access to a mutable home-directory
// checkout. Installation and selection are distinct transactions.
struct InstallPlan{
    name:String,source:PathBuf,ownership_hash:String,
    snapshot:crate::login_theme::Snapshot,
}
fn absent(path:&Path)->Result<()>{
    match fs::symlink_metadata(path){
        Err(e) if e.kind()==io::ErrorKind::NotFound=>Ok(()),
        Err(e)=>Err(e.into()),
        Ok(_)=>bail!("destination already exists; choose a new Atelier theme name"),
    }
}
fn import_hash(location:&Location,source:&Path)->Result<String>{
    if !source.is_absolute()||source.canonicalize()?!=source{bail!("choose a canonical protected import directory");}
    location.protected_ancestors(source)?;
    let mut count=0;let mut total=0;let mut hash=Sha256::new();
    location.hash_tree(source,source,0,&mut count,&mut total,&mut hash,false)?;
    Ok(format!("{:x}",hash.finalize()))
}
fn rename_new(source:&Path,dest:&Path)->Result<()>{
    use std::{ffi::CString,os::unix::ffi::OsStrExt};
    let source=CString::new(source.as_os_str().as_bytes())?;
    let dest=CString::new(dest.as_os_str().as_bytes())?;
    if unsafe{libc::renameat2(libc::AT_FDCWD,source.as_ptr(),libc::AT_FDCWD,dest.as_ptr(),libc::RENAME_NOREPLACE)}!=0{
        return Err(io::Error::last_os_error()).context("publish new theme without replacing an existing directory");
    }
    Ok(())
}
impl InstallPlan{
    fn review(location:&Location,name:&str,source:&Path)->Result<Self>{
        if !valid_name(name)||!name.starts_with("atelier-")||name=="atelier-"{bail!("choose a new atelier-NAME theme directory");}
        location.protected_ancestors(&location.themes)?;
        absent(&location.themes.join(name))?;
        let ownership_hash=import_hash(location,source)?;
        let snapshot=crate::login_theme::Snapshot::read(source)?;
        crate::login_theme::validate_entrypoint(source)?;
        if import_hash(location,source)?!=ownership_hash{bail!("source changed during review");}
        Ok(Self{name:name.into(),source:source.into(),ownership_hash,snapshot})
    }
    fn detail(&self,location:&Location)->String{
        format!("Administrator-controlled source: {}\nContent SHA-256: {}\nNew directory: {}\nCopy regular files as 0644, directories as 0755. No overwrite, script execution, theme selection, service restart or authentication changes. QML remains executable code; inspect compatibility and trust independently.",
            self.source.display(),self.snapshot.fingerprint(),location.themes.join(&self.name).display())
    }
    fn commit(self,location:&Location)->Result<PathBuf>{
        location.protected_ancestors(&location.themes)?;
        let dest=location.themes.join(&self.name);
        absent(&dest)?;
        if import_hash(location,&self.source)?!=self.ownership_hash{bail!("source changed since review");}
        let temporary=tempfile::Builder::new().prefix(".atelier-import-").tempdir_in(&location.themes)?;
        let staged=temporary.path().join("theme");
        self.snapshot.copy_for_greeter(&staged)?;
        if crate::login_theme::Snapshot::read(&staged)?.fingerprint()!=self.snapshot.fingerprint(){bail!("staged theme verification failed");}
        rename_new(&staged,&dest)?;
        File::open(&location.themes)?.sync_all().with_context(||format!("theme published at {}, but directory sync failed; inspect before retrying",dest.display()))?;
        Ok(dest)
    }
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
        [action,name,flag,source] if flag=="--from"&&matches!(action.as_str(),"install-plan"|"install")=>{
            if action=="install"{administrator()?;}
            let plan=InstallPlan::review(&location,name,Path::new(source))?;
            println!("{}",plan.detail(&location));
            if action=="install-plan"{println!("Preview only. No changes.");return Ok(());}
            if !confirm(&format!("install trusted {name}"))?{println!("Cancelled.");return Ok(());}
            println!("Installed theme at {}. Selection remains separate.",plan.commit(&location)?.display());Ok(())
        }
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
        _=>bail!("use login-theme install-plan NAME --from DIR | install NAME --from DIR | selection-plan NAME | select NAME | rollback-selection"),
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    fn fixture(root:&Path)->Location{
        let loc=Location{dropins:root.join("conf.d"),legacy:root.join("sddm.conf"),themes:root.join("themes"),owner:nix::unistd::Uid::effective().as_raw(),anchor:root.into()};
        fs::create_dir(&loc.dropins).unwrap();fs::create_dir(&loc.themes).unwrap();loc
    }
    #[test]
    fn installs_reviewed_source_without_selection_or_overwrite(){
        let root=tempfile::tempdir().unwrap();let loc=fixture(root.path());
        let source=root.path().join("import");fs::create_dir(&source).unwrap();
        fs::write(source.join("Main.qml"),b"// inert").unwrap();
        fs::set_permissions(source.join("Main.qml"),fs::Permissions::from_mode(0o600)).unwrap();
        let plan=InstallPlan::review(&loc,"atelier-demo",&source).unwrap();
        let stale=InstallPlan::review(&loc,"atelier-stale",&source).unwrap();
        let dest=plan.commit(&loc).unwrap();
        assert_eq!(fs::read(dest.join("Main.qml")).unwrap(),b"// inert");
        assert_eq!(fs::metadata(dest.join("Main.qml")).unwrap().permissions().mode()&0o777,0o644);
        assert!(loc.theme_fingerprint("atelier-demo").is_ok());
        assert!(loc.managed().unwrap().is_none());
        assert!(InstallPlan::review(&loc,"atelier-demo",&source).is_err());
        fs::write(source.join("Main.qml"),b"changed").unwrap();
        assert!(stale.commit(&loc).is_err());
        assert!(!loc.themes.join("atelier-stale").exists());
        fs::set_permissions(&source,fs::Permissions::from_mode(0o777)).unwrap();
        assert!(InstallPlan::review(&loc,"atelier-bad",&source).is_err());
    }
    #[test]
    fn publish_never_replaces_even_empty_directory(){
        let root=tempfile::tempdir().unwrap();let src=root.path().join("a");let dst=root.path().join("b");
        fs::create_dir(&src).unwrap();fs::create_dir(&dst).unwrap();
        assert!(rename_new(&src,&dst).is_err());
        assert!(src.is_dir()&&dst.is_dir());
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
    fn full_select_and_rollback_with_stale_review_protection(){
        let root=tempfile::tempdir().unwrap();let loc=fixture(root.path());
        for name in ["breeze","other"]{
            let dir=loc.themes.join(name);fs::create_dir(&dir).unwrap();
            fs::set_permissions(&dir,fs::Permissions::from_mode(0o755)).unwrap();
            fs::write(dir.join("Main.qml"),b"// inert").unwrap();
            fs::set_permissions(dir.join("Main.qml"),fs::Permissions::from_mode(0o644)).unwrap();
        }
        let old=Plan::review(&loc,"breeze").unwrap();
        let stale=Plan::review(&loc,"other").unwrap();
        assert!(!loc.dropins.join(MANAGED).exists());
        assert!(old.commit(&loc).unwrap());assert!(stale.commit(&loc).is_err());
        assert!(Plan::review(&loc,"other").unwrap().commit(&loc).unwrap());
        let review=rollback_plan(&loc).unwrap();rollback(&loc,review).unwrap();
        assert_eq!(parse_managed(&loc.managed().unwrap().unwrap()).unwrap(),"breeze");
        assert!(rollback_plan(&loc).is_err());
        fs::write(&loc.legacy,b"[Theme]\nCurrent=other\n").unwrap();
        assert!(Plan::review(&loc,"breeze").is_err());fs::remove_file(&loc.legacy).unwrap();
        fs::write(loc.dropins.join("zz-local.conf"),b"# conflict").unwrap();
        assert!(Plan::review(&loc,"breeze").is_err());
    }
    #[test]
    fn first_selection_rollback_removes_only_override(){
        let root=tempfile::tempdir().unwrap();let loc=fixture(root.path());
        let dir=loc.themes.join("breeze");fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir,fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(dir.join("Main.qml"),b"// inert").unwrap();
        fs::set_permissions(dir.join("Main.qml"),fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Plan::review(&loc,"breeze").unwrap().commit(&loc).unwrap());
        rollback(&loc,rollback_plan(&loc).unwrap()).unwrap();
        assert!(loc.managed().unwrap().is_none());
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
