//! Versioned native window inventory and fixed-size activation packets.
//! Local desktop-user interface, not an authorization boundary between apps.
use anyhow::{bail,Result};
use serde::{Deserialize,Serialize};
use std::{io::Read,path::{Path,PathBuf},fs};

pub const MAX_WINDOWS:usize=128;
pub const MAX_BYTES:u64=512*1024;
pub const ACTIVATE_LEN:usize=25;
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskWindow{
    pub id:u64,
    pub app_id:String,
    pub title:String,
    pub workspace:u8,
    pub focused:bool,
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSnapshot{
    pub version:u32,
    pub session:[u8;16],
    pub truncated:bool,
    pub windows:Vec<TaskWindow>,
}
impl TaskSnapshot{
    pub fn validate(&self)->Result<()>{
        if self.version!=1||self.windows.len()>MAX_WINDOWS{bail!("unsupported task inventory");}
        let mut ids=std::collections::HashSet::new();
        let mut focused=0;
        for w in &self.windows{
            if w.id==0||!ids.insert(w.id)||!(1..=4).contains(&w.workspace)
                ||w.app_id.chars().count()>128||w.title.chars().count()>256
                ||w.app_id.chars().any(char::is_control)||w.title.chars().any(char::is_control){
                bail!("invalid task entry");
            }
            if w.focused{focused+=1;}
        }
        if focused>1{bail!("multiple focused windows");}
        Ok(())
    }
    pub fn encode(&self)->Result<Vec<u8>>{self.validate()?;let bytes=serde_json::to_vec(self)?;if bytes.len() as u64>MAX_BYTES{bail!("task inventory too large");}Ok(bytes)}
    pub fn read(path:&Path)->Result<Self>{
        let meta=fs::symlink_metadata(path)?;
        if !meta.is_file()||meta.len()>MAX_BYTES{bail!("invalid inventory file");}
        let mut bytes=Vec::new();fs::File::open(path)?.take(MAX_BYTES+1).read_to_end(&mut bytes)?;
        if bytes.len() as u64>MAX_BYTES{bail!("task inventory too large");}
        let state:Self=serde_json::from_slice(&bytes)?;state.validate()?;Ok(state)
    }
}
pub fn state_path(display:&str)->Result<PathBuf>{let (command,_)=crate::workspace_paths(display)?;Ok(command.with_extension("tasks"))}
pub fn activation(session:[u8;16],id:u64)->[u8;ACTIVATE_LEN]{
    let mut bytes=[0;ACTIVATE_LEN];bytes[0]=0xa1;bytes[1..17].copy_from_slice(&session);bytes[17..].copy_from_slice(&id.to_le_bytes());bytes
}
pub fn decode_activation(bytes:&[u8],session:[u8;16])->Option<u64>{
    if bytes.len()!=ACTIVATE_LEN||bytes[0]!=0xa1||bytes[1..17]!=session{return None;}
    let id=u64::from_le_bytes(bytes[17..].try_into().ok()?);
    (id!=0).then_some(id)
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn requests_are_exact_and_session_bound(){
        let packet=activation([3;16],42);
        assert_eq!(decode_activation(&packet,[3;16]),Some(42));
        assert_eq!(decode_activation(&packet,[4;16]),None);
        assert_eq!(decode_activation(&packet[..24],[3;16]),None);
        assert_eq!(decode_activation(&activation([3;16],0),[3;16]),None);
    }
    #[test]
    fn validates_inventory_and_duplicate_ids(){
        let item=TaskWindow{id:1,app_id:"app".into(),title:"Title".into(),workspace:1,focused:true};
        let mut s=TaskSnapshot{version:1,session:[1;16],truncated:false,windows:vec![item.clone()]};
        assert!(s.encode().is_ok());s.windows.push(item);assert!(s.encode().is_err());
    }
}
