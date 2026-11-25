//! Native task adapter. Never queries unrelated compositors or process names.
use atelier_config::tasks::{self,TaskSnapshot,TaskWindow};
use anyhow::{bail,Context,Result};
use std::{fs,os::unix::{fs::{FileTypeExt,MetadataExt},net::UnixDatagram},path::{Path,PathBuf}};

pub struct TaskBackend{command:PathBuf,state:PathBuf}
fn private_path(path:&Path,socket:bool,uid:u32)->Result<()>{
    let meta=fs::symlink_metadata(path)?;
    if meta.uid()!=uid||meta.mode()&0o077!=0||if socket{!meta.file_type().is_socket()}else{!meta.is_file()}{
        bail!("unsafe native task channel");
    }
    Ok(())
}
impl TaskBackend{
    pub fn detect()->Option<Self>{
        let display=std::env::var("WAYLAND_DISPLAY").ok()?;
        let (command,_)=atelier_config::workspace_paths(&display).ok()?;
        let state=tasks::state_path(&display).ok()?;
        let backend=Self{command,state};
        // The panel may start before inventory publication; only construct an
        // adapter for an actual native control socket, never a name guess.
        let parent=backend.command.parent()?;
        let meta=fs::symlink_metadata(parent).ok()?;
        if !meta.is_dir()||meta.mode()&0o077!=0{return None;}
        private_path(&backend.command,true,meta.uid()).ok()?;
        Some(backend)
    }
    pub fn snapshot(&self)->Result<TaskSnapshot>{
        let parent=self.command.parent().context("missing runtime directory")?;
        let meta=fs::symlink_metadata(parent)?;
        if !meta.is_dir()||meta.mode()&0o077!=0{bail!("unsafe runtime directory");}
        private_path(&self.command,true,meta.uid())?;
        private_path(&self.state,false,meta.uid())?;
        TaskSnapshot::read(&self.state)
    }
    pub fn activate(&self,reviewed:&TaskSnapshot,id:u64)->Result<()>{
        let current=self.snapshot()?;
        if current.session!=reviewed.session||!current.windows.iter().any(|w|w.id==id){
            bail!("Window is no longer available");
        }
        let socket=UnixDatagram::unbound()?;
        socket.set_nonblocking(true)?;
        socket.send_to(&tasks::activation(current.session,id),&self.command)?;
        // Delivery is not proof of focus; next snapshot is authoritative.
        Ok(())
    }
}
pub fn matches_pin(pin:&str,app_id:&str)->bool{
    !app_id.is_empty()&&(pin==app_id||pin.strip_suffix(".desktop")==Some(app_id))
}
pub fn next_window<'a>(snapshot:&'a TaskSnapshot,pin:&str)->Option<&'a TaskWindow>{
    let matches:Vec<_>=snapshot.windows.iter().filter(|w|matches_pin(pin,&w.app_id)).collect();
    if matches.is_empty(){return None;}
    let next=matches.iter().position(|w|w.focused).map_or(0,|i|(i+1)%matches.len());
    Some(matches[next])
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn exact_case_sensitive_matching_and_cycle(){
        let window=|id,focused,app_id:&str|TaskWindow{id,focused,app_id:app_id.into(),title:"Title".into(),workspace:1};
        let state=TaskSnapshot{version:1,session:[1;16],truncated:false,windows:vec![window(1,true,"org.example.App"),window(2,false,"org.example.App.desktop"),window(3,false,"org.example.AppOther")]};
        assert_eq!(next_window(&state,"org.example.App.desktop").unwrap().id,2);
        assert!(next_window(&state,"org.example.app.desktop").is_none());
        assert!(!matches_pin("org.example.App.desktop",""));
    }
}
