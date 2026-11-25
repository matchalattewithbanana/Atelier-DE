//! Read-only prerequisites for external Wayland lockers. Protocol advertisement
//! alone never establishes correct authentication, coverage or fail-closed behavior.
use anyhow::{bail,Context,Result};
use serde::Serialize;
use std::{env,fs,io::{self,Read},os::unix::fs::{FileTypeExt,MetadataExt,PermissionsExt},
    path::{Path,PathBuf},process::{Command,Stdio}};

const INFO:&str="/usr/bin/wayland-info";
const TIMEOUT:&str="/usr/bin/timeout";
const MAX_OUTPUT:u64=512*1024;
const LOCK:&str="ext_session_lock_manager_v1";
const LAYER:&str="zwlr_layer_shell_v1";
const LOCKERS:&[&str]=&["swaylock","hyprlock","gtklock"];

pub(super) fn is_locker(id:&str)->bool{LOCKERS.contains(&id)}
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize)]
#[serde(rename_all="kebab-case")]
pub enum ProbeState{Present,Absent,Unknown}
#[derive(Debug,Serialize)]
pub struct Protocols{
    pub session_lock:ProbeState,
    pub layer_shell:ProbeState,
    pub outputs:Option<usize>,
    pub detail:String,
}
impl Protocols{
    fn unknown(detail:impl Into<String>)->Self{
        Self{session_lock:ProbeState::Unknown,layer_shell:ProbeState::Unknown,outputs:None,detail:detail.into()}
    }
}
#[derive(Debug,Serialize)]
pub struct Locker{
    pub id:&'static str,
    pub executable:ProbeState,
    pub pam_service:ProbeState,
    pub session_lock:ProbeState,
    pub layer_shell:ProbeState,
    pub assessment:&'static str,
}
#[derive(Debug,Serialize)]
pub struct Report{
    pub schema:u32,
    pub display:Option<String>,
    pub protocols:Protocols,
    pub lockers:Vec<Locker>,
    pub note:&'static str,
}
fn protected(path:&Path,executable:bool)->Result<()>{
    let path=path.canonicalize()?;
    let m=fs::metadata(&path)?;
    if !m.is_file()||m.uid()!=0||m.permissions().mode()&0o022!=0
        ||(executable&&m.permissions().mode()&0o111==0){bail!("unprotected system file");}
    for parent in path.parent().context("missing parent")?.ancestors(){
        let m=fs::metadata(parent)?;
        if !m.is_dir()||m.uid()!=0||m.permissions().mode()&0o022!=0{bail!("unprotected system path");}
    }
    Ok(())
}
fn file_state(paths:&[PathBuf],executable:bool)->ProbeState{
    for p in paths{
        match fs::symlink_metadata(p){
            Ok(_)=>return if protected(p,executable).is_ok(){ProbeState::Present}else{ProbeState::Unknown},
            Err(e) if e.kind()==io::ErrorKind::NotFound=>continue,
            Err(_)=>return ProbeState::Unknown,
        }
    }
    ProbeState::Absent
}
fn parse_registry(text:&str)->Result<Protocols>{
    let mut lock=false;let mut layer=false;let mut outputs=0usize;let mut compositor=false;
    for line in text.lines(){
        // wayland-info prints registry globals as "interface: 'NAME', version: N, name: N".
        // Only count actual global lines, not arbitrary descriptive text.
        let Some(rest)=line.strip_prefix("interface: '")else{continue;};
        let (interface,rest)=rest.split_once("', version: ").context("malformed global line")?;
        let (version,id)=rest.split_once(", name: ").context("malformed global version")?;
        let version:u32=version.trim().parse()?;let id:u32=id.trim().parse()?;
        if version==0||id==0{bail!("invalid global advertisement");}
        match interface{
            LOCK=>lock=true,LAYER=>layer=true,"wl_output"=>outputs+=1,"wl_compositor"=>compositor=true,_=>{},
        }
    }
    if !compositor{bail!("no compositor global in probe response");}
    let state=|found|if found{ProbeState::Present}else{ProbeState::Absent};
    Ok(Protocols{session_lock:state(lock),layer_shell:state(layer),outputs:Some(outputs),
        detail:"Registry advertisements from the current Wayland connection only.".into()})
}
fn socket(display:&str)->Result<PathBuf>{
    if display.is_empty()||display.len()>108||display.chars().any(char::is_control){bail!("invalid display name");}
    let display=Path::new(display);
    let socket=if display.is_absolute(){display.to_owned()}else{
        if display.components().count()!=1||!matches!(display.components().next(),Some(std::path::Component::Normal(_))){
            bail!("invalid relative display name");
        }
        let dir=PathBuf::from(env::var_os("XDG_RUNTIME_DIR").context("missing XDG_RUNTIME_DIR")?);
        if !dir.is_absolute(){bail!("invalid runtime directory");}
        let m=fs::symlink_metadata(&dir)?;
        if !m.is_dir()||m.uid()!=nix::unistd::Uid::effective().as_raw()||m.permissions().mode()&0o077!=0{
            bail!("runtime directory is not private and owned by the desktop user");
        }
        dir.join(display)
    };
    let m=fs::symlink_metadata(&socket)?;
    if !m.file_type().is_socket()||m.uid()!=nix::unistd::Uid::effective().as_raw(){bail!("display socket is not owned by the desktop user");}
    Ok(socket)
}
fn probe(display:&str)->Result<Protocols>{
    protected(Path::new(INFO),true).context("wayland-info unavailable or untrusted; install wayland-utils through reviewed packages")?;
    protected(Path::new(TIMEOUT),true).context("timeout unavailable or untrusted")?;
    let path=socket(display)?;
    let mut child=Command::new(TIMEOUT).args(["--kill-after=1s","5s",INFO])
        .env("WAYLAND_DISPLAY",path).env_remove("WAYLAND_SOCKET").env("LC_ALL","C")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let mut bytes=Vec::new();
    let read=match child.stdout.take(){
        Some(out)=>out.take(MAX_OUTPUT+1).read_to_end(&mut bytes),
        None=>Err(io::Error::other("missing registry output")),
    };
    let status=child.wait()?;
    read?;
    if !status.success()||bytes.len() as u64>MAX_OUTPUT{bail!("registry query failed or exceeded its output bound");}
    parse_registry(std::str::from_utf8(&bytes)?)
}
fn assess(exe:ProbeState,pam:ProbeState,lock:ProbeState,layer:ProbeState,needs_layer:bool)->&'static str{
    if exe==ProbeState::Absent||pam==ProbeState::Absent||lock==ProbeState::Absent||(needs_layer&&layer==ProbeState::Absent){
        "Prerequisites missing"
    }else if exe!=ProbeState::Present||pam!=ProbeState::Present||lock!=ProbeState::Present||(needs_layer&&layer!=ProbeState::Present){
        "Prerequisites unknown"
    }else{"Prerequisites detected; NOT validated for locking"}
}
pub(super) fn check()->Result<Report>{
    if nix::unistd::Uid::effective().is_root(){bail!("run locker diagnostics as the desktop user, not root");}
    let display=env::var("WAYLAND_DISPLAY").ok().filter(|s|!s.is_empty());
    let protocols=match &display{
        Some(display)=>probe(display).unwrap_or_else(|e|Protocols::unknown(format!("Registry probe unavailable: {e:#}"))),
        None=>Protocols::unknown("No explicit WAYLAND_DISPLAY; not probing an implicit session."),
    };
    let lockers=LOCKERS.iter().map(|&id|{
        let executable=file_state(&[Path::new("/usr/bin").join(id)],true);
        let pam_service=file_state(&["/etc/pam.d","/usr/lib/pam.d","/usr/share/pam.d"].map(|p|Path::new(p).join(id)),false);
        Locker{id,executable,pam_service,session_lock:protocols.session_lock,layer_shell:protocols.layer_shell,
            assessment:assess(executable,pam_service,protocols.session_lock,protocols.layer_shell,id=="gtklock")}
    }).collect();
    Ok(Report{schema:1,display:display.map(|s|s.chars().filter(|c|!c.is_control()).take(108).collect()),protocols,lockers,
        note:"Read-only diagnostics of the connected compositor. Nested sessions may not secure the host. No PAM authentication test, lock attempt, configuration write or activation. Native Atelier locking remains unsupported until secure protocol, output coverage, input isolation and crash recovery are validated."})
}
pub(super) fn run(args:&[String])->Result<()>{
    let json=match args{
        [a] if a=="check"=>false,
        [a,b] if a=="check"&&b=="--json"=>true,
        _=>bail!("use locker check [--json]"),
    };
    let report=check()?;
    if json{println!("{}",serde_json::to_string_pretty(&report)?);}
    else{
        println!("Screen locker prerequisites — NOT a lock test\nDisplay: {}\n{}",report.display.as_deref().unwrap_or("(none)"),report.protocols.detail);
        println!("Session lock: {:?}; layer shell: {:?}; outputs: {:?}",report.protocols.session_lock,report.protocols.layer_shell,report.protocols.outputs);
        for l in report.lockers{println!("{} — {}\n  Executable: {:?}; PAM file: {:?}",l.id,l.assessment,l.executable,l.pam_service);}
        println!("{}",report.note);
    }
    Ok(())
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn only_real_globals_count(){
        let p=parse_registry("interface: 'wl_compositor', version: 6, name: 1\ninterface: 'wl_output', version: 4, name: 2\n  ext_session_lock_manager_v1\n").unwrap();
        assert_eq!(p.session_lock,ProbeState::Absent);assert_eq!(p.outputs,Some(1));
        let p=parse_registry("interface: 'wl_compositor', version: 6, name: 1\ninterface: 'ext_session_lock_manager_v1', version: 1, name: 2\ninterface: 'zwlr_layer_shell_v1', version: 4, name: 3\n").unwrap();
        assert_eq!(p.session_lock,ProbeState::Present);assert_eq!(p.layer_shell,ProbeState::Present);
        assert!(parse_registry("").is_err());assert!(parse_registry("interface: 'wl_compositor', version: bad, name: 1\n").is_err());
    }
    #[test]
    fn never_claims_secure_lock_readiness(){
        use ProbeState::*;
        assert_eq!(assess(Present,Present,Present,Present,true),"Prerequisites detected; NOT validated for locking");
        assert_eq!(assess(Present,Present,Absent,Present,false),"Prerequisites missing");
        assert_eq!(assess(Present,Present,Present,Absent,true),"Prerequisites missing");
        assert_eq!(assess(Present,Present,Unknown,Present,false),"Prerequisites unknown");
    }
}
