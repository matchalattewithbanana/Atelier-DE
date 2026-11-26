//! Internal recovery-process lifetime management. No production invocation
//! until a dedicated authentication implementation and hardware handoff exist.
use crate::state::Atelier;
use anyhow::{bail,Context,Result};
use smithay::reexports::wayland_server::backend::ClientId;
use std::{fs,net::Shutdown,os::{fd::OwnedFd,unix::{fs::MetadataExt,net::UnixStream}},
    path::Path,process::{Child,Command,Stdio},time::{Duration,Instant}};

const EXECUTABLE:&str="/usr/lib/atelier/atelier-recovery-locker";
const PAM:&str="/etc/pam.d/atelier-recovery-locker";
const ADMISSION_TIMEOUT:Duration=Duration::from_secs(10);

/// Refuse non-root-owned or writable files, symlinks and unsafe ancestors.
/// A root-owned executable is a prerequisite, not proof of authentication.
fn protected(path:&Path,executable:bool,owner:u32,anchor:&Path)->Result<()>{
    if !path.is_absolute()||!path.starts_with(anchor){bail!("invalid protected path");}
    let m=fs::symlink_metadata(path)?;
    if !m.is_file()||m.uid()!=owner||m.mode()&0o022!=0||
        (executable&&(m.mode()&0o111==0||m.mode()&0o6000!=0)){
        bail!("untrusted recovery file: {}",path.display());
    }
    for parent in path.parent().context("missing parent")?.ancestors(){
        let m=fs::symlink_metadata(parent)?;
        if !m.is_dir()||m.uid()!=owner||m.mode()&0o022!=0{
            bail!("untrusted recovery directory: {}",parent.display());
        }
        if parent==anchor{return Ok(());}
    }
    bail!("protected path outside anchor")
}

struct Process{
    child:Child,
    // Retain a duplicate of the private connection solely to shut down
    // all inherited copies, including copies left in a forked helper.
    shutdown:UnixStream,
}
impl Process{
    fn spawn(mut command:Command,endpoint:UnixStream)->Result<Self>{
        let shutdown=endpoint.try_clone()?;
        let fd:OwnedFd=endpoint.into();
        command.stdin(Stdio::from(fd)).stdout(Stdio::null()).stderr(Stdio::null());
        command.env_clear().env("PATH","/usr/bin:/bin").env("WAYLAND_SOCKET","0")
            .current_dir("/");
        let child=command.spawn().context("start private recovery process")?;
        Ok(Self{child,shutdown})
    }
    fn close(&mut self){
        let _=self.shutdown.shutdown(Shutdown::Both);
        // try_wait reaps an exited child, so never signal a reused PID.
        if !matches!(self.child.try_wait(),Ok(Some(_))){
            let _=self.child.kill();
            let _=self.child.wait();
        }
    }
}
impl Drop for Process{fn drop(&mut self){self.close();}}

#[derive(Debug,PartialEq,Eq)]
pub(crate) enum Status{Pending,Confirmed,Stopped}

/// A supervisor never accepts exit status or a readiness pipe as an unlock
/// signal. Only the actual authorized Wayland protocol transition can unlock.
/// The embedding event loop must poll and explicitly stop on teardown.
#[allow(dead_code)]
pub(crate) struct Supervisor{
    client:ClientId,
    process:Option<Process>,
    deadline:Instant,
}
#[allow(dead_code)]
impl Supervisor{
    pub(crate) fn launch(state:&mut Atelier)->Result<Self>{
        if rustix::process::geteuid().is_root(){bail!("recovery locker must run as the desktop user");}
        protected(Path::new(EXECUTABLE),true,0,Path::new("/"))?;
        protected(Path::new(PAM),false,0,Path::new("/"))?;
        // No shell, user configuration, environment overrides or external
        // locker selection. In particular swaylock SIGUSR1 is not suitable
        // as this authority without additional signal isolation.
        let (client,endpoint)=state.recovery_connection()?;
        match Process::spawn(Command::new(EXECUTABLE),endpoint){
            Ok(process)=>Ok(Self{client,process:Some(process),deadline:Instant::now()+ADMISSION_TIMEOUT}),
            Err(error)=>{state.revoke_recovery(&client);Err(error)}
        }
    }
    pub(crate) fn poll(&mut self,state:&mut Atelier)->Result<Status>{
        let Some(process)=self.process.as_mut()else{return Ok(Status::Stopped);};
        match process.child.try_wait(){
            Ok(Some(_))=>{self.stop(state);return Ok(Status::Stopped);}
            Err(error)=>{self.stop(state);return Err(error).context("poll recovery child");}
            Ok(None)=>{}
        }
        if state.recovery_confirmed(&self.client){return Ok(Status::Confirmed);}
        if !state.recovery_pending(&self.client)||Instant::now()>=self.deadline{
            self.stop(state);return Ok(Status::Stopped);
        }
        Ok(Status::Pending)
    }
    pub(crate) fn stop(&mut self,state:&mut Atelier){
        // Revoke before closing the channel. A queued unlock from a failed
        // process must not gain authority during a subsequent dispatch.
        state.revoke_recovery(&self.client);
        self.process.take();
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    use std::{io::Read,os::unix::fs::{symlink,PermissionsExt}};
    #[test]
    fn trust_check_rejects_writable_files_and_symlinks(){
        let dir=tempfile::tempdir().unwrap();
        let owner=rustix::process::geteuid().as_raw();
        let file=dir.path().join("locker");
        fs::write(&file,b"test").unwrap();
        fs::set_permissions(&file,fs::Permissions::from_mode(0o700)).unwrap();
        assert!(protected(&file,true,owner,dir.path()).is_ok());
        fs::set_permissions(&file,fs::Permissions::from_mode(0o720)).unwrap();
        assert!(protected(&file,true,owner,dir.path()).is_err());
        let alias=dir.path().join("alias");symlink(&file,&alias).unwrap();
        assert!(protected(&alias,false,owner,dir.path()).is_err());
    }
    #[test]
    fn process_exit_closes_even_retained_private_socket_copies(){
        let (mut server,endpoint)=UnixStream::pair().unwrap();
        let duplicate=endpoint.try_clone().unwrap();
        server.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut process=Process::spawn(Command::new("/usr/bin/true"),endpoint).unwrap();
        assert!(process.child.wait().unwrap().success());
        process.close();
        let mut byte=[0];assert_eq!(server.read(&mut byte).unwrap(),0);
        drop(duplicate);
    }
}
