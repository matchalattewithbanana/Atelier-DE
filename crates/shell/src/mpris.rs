//! Direct session-bus MPRIS transport for the interactive shell player.
//! Existing playerctl CLI/snapshot consumers stay compatible. All calls here
//! are made on the media worker, never the GTK thread.
use anyhow::{bail,Context,Result};
use gtk::{gio,glib};
use glib::variant::{ToVariant,ObjectPath};
use std::collections::HashMap;

const PREFIX:&str="org.mpris.MediaPlayer2.";
const PATH:&str="/org/mpris/MediaPlayer2";
const PLAYER:&str="org.mpris.MediaPlayer2.Player";
const DBUS:&str="org.freedesktop.DBus";
const TIMEOUT:i32=1500;
type Properties=HashMap<String,glib::Variant>;

#[derive(Debug,Clone,PartialEq,Eq)]
pub struct Identity{pub name:String,owner:String}
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct Track{pub player:Identity,id:String,pub length_us:i64}
#[derive(Debug,Clone,Default)]
pub struct Capabilities{pub play:bool,pub pause:bool,pub previous:bool,pub next:bool,pub seek:bool}
#[derive(Debug,Clone)]
pub struct Details{
    pub identity:Identity,pub title:String,pub artist:String,pub album:String,pub status:String,
    pub track:Option<Track>,pub position_us:Option<i64>,pub capabilities:Capabilities,
    pub art_uri:Option<String>,
}
#[derive(Debug,Clone)]
pub enum Action{Play,Pause,Previous,Next,Seek(Track,i64)}

fn clean(text:&str)->String{text.chars().filter(|c|!c.is_control()&&!matches!(*c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')).take(240).collect()}
fn get<T:glib::variant::FromVariant>(props:&Properties,key:&str)->Option<T>{props.get(key)?.get::<T>()}
fn flag(props:&Properties,key:&str)->bool{get::<bool>(props,key)==Some(true)}
fn details(identity:Identity,props:&Properties)->Details{
    let metadata=get::<Properties>(props,"Metadata").unwrap_or_default();
    let string=|key|clean(&get::<String>(&metadata,key).unwrap_or_default());
    let title=string("xesam:title");let album=string("xesam:album");
    let artist=clean(&get::<Vec<String>>(&metadata,"xesam:artist").unwrap_or_default().join(", "));
    let status=get::<String>(props,"PlaybackStatus").filter(|s|matches!(s.as_str(),"Playing"|"Paused"|"Stopped")).unwrap_or_else(||"Unknown".into());
    let track=get::<ObjectPath>(&metadata,"mpris:trackid").and_then(|id|{
        let id=id.as_str().to_owned();
        let length_us=get::<i64>(&metadata,"mpris:length")?;
        if id=="/org/mpris/MediaPlayer2/TrackList/NoTrack"||length_us<=0{return None;}
        Some(Track{player:identity.clone(),id,length_us})
    });
    let position_us=get::<i64>(props,"Position").filter(|p|*p>=0).map(|p|track.as_ref().map_or(p,|t|p.min(t.length_us)));
    let control=flag(props,"CanControl");
    let capabilities=Capabilities{play:control&&flag(props,"CanPlay"),pause:control&&flag(props,"CanPause"),
        previous:control&&flag(props,"CanGoPrevious"),next:control&&flag(props,"CanGoNext"),
        seek:control&&flag(props,"CanSeek")&&track.is_some()&&position_us.is_some()};
    let art_uri=get::<String>(&metadata,"mpris:artUrl").filter(|s|s.len()<=4096&&!s.chars().any(char::is_control));
    Details{identity,title,artist,album,status,track,position_us,capabilities,art_uri}
}
/// Bind authorization and the wire request to one fresh snapshot.
fn action_call(state:&Details,action:Action)->Result<(&'static str,Option<glib::Variant>)>{
    let (method,args)=match action{
            Action::Play if state.capabilities.play=>("Play",None),
            Action::Pause if state.capabilities.pause=>("Pause",None),
            Action::Previous if state.capabilities.previous=>("Previous",None),
            Action::Next if state.capabilities.next=>("Next",None),
            Action::Seek(track,position) if state.capabilities.seek=>{
                if state.track.as_ref()!=Some(&track)||position<0||position>track.length_us{bail!("track or duration changed; seek discarded");}
                let path=ObjectPath::try_from(track.id).context("invalid track path")?;
                ("SetPosition",Some((path,position).to_variant()))
            }
            _=>bail!("player no longer supports this action"),
    };

    Ok((method,args))
}
pub struct Bus{connection:gio::DBusConnection}
impl Bus{
    pub fn connect()->Result<Self>{
        Ok(Self{connection:gio::bus_get_sync(gio::BusType::Session,None::<&gio::Cancellable>)?})
    }
    fn call(&self,name:&str,path:&str,interface:&str,method:&str,args:Option<&glib::Variant>)->Result<glib::Variant>{
        Ok(self.connection.call_sync(Some(name),path,interface,method,args,None,gio::DBusCallFlags::NO_AUTO_START,TIMEOUT,None::<&gio::Cancellable>)?)
    }
    pub fn players(&self)->Result<Vec<String>>{
        let reply=self.call(DBUS,"/org/freedesktop/DBus",DBUS,"ListNames",None)?;
        let (names,)=reply.get::<(Vec<String>,)>().context("invalid bus name list")?;
        let mut names:Vec<_>=names.into_iter().filter(|name|name.starts_with(PREFIX)&&name.len()>PREFIX.len()&&name.len()<=255).collect();
        names.sort();names.dedup();Ok(names)
    }
    fn identity(&self,name:&str)->Result<Identity>{
        if !name.starts_with(PREFIX)||name.len()>255||name.len()<=PREFIX.len(){bail!("invalid player name");}
        let reply=self.call(DBUS,"/org/freedesktop/DBus",DBUS,"GetNameOwner",Some(&(name,).to_variant()))?;
        let (owner,)=reply.get::<(String,)>().context("invalid player owner")?;
        if !owner.starts_with(':'){bail!("player owner is not unique");}
        Ok(Identity{name:name.into(),owner})
    }
    fn read(&self,identity:Identity)->Result<Details>{
        let reply=self.call(&identity.owner,PATH,"org.freedesktop.DBus.Properties","GetAll",Some(&(PLAYER,).to_variant()))?;
        let (props,)=reply.get::<(Properties,)>().context("invalid MPRIS properties")?;
        Ok(details(identity,&props))
    }
    pub fn selected(&self,name:&str)->Result<Details>{self.read(self.identity(name)?)}
    pub fn action(&self,expected:&Identity,action:Action)->Result<()>{
        let current=self.identity(&expected.name)?;
        if &current!=expected{bail!("player instance changed; refresh and retry");}
        let state=self.read(current)?;
        let (method,args)=action_call(&state,action)?;
        // Destination is the exact unique owner, never a replaceable alias.
        // SetPosition also supplies the track ID, so compliant players ignore
        // a seek if the track changes between GetAll and SetPosition.
        self.call(&expected.owner,PATH,PLAYER,method,args.as_ref())?;
        Ok(())
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    fn identity()->Identity{Identity{name:format!("{PREFIX}test"),owner:":1.2".into()}}
    #[test]
    fn unknown_capabilities_are_disabled(){
        let state=details(identity(),&Properties::new());
        assert!(!state.capabilities.play&&!state.capabilities.seek);
        assert!(state.track.is_none()&&state.position_us.is_none());
    }
    #[test]
    fn typed_track_metadata_and_capabilities(){
        let mut metadata=Properties::new();
        metadata.insert("mpris:trackid".into(),ObjectPath::try_from("/track/one").unwrap().to_variant());
        metadata.insert("mpris:length".into(),120_000_000i64.to_variant());
        metadata.insert("xesam:title".into(),"Example".to_variant());
        metadata.insert("xesam:artist".into(),vec!["Artist A","Artist B"].to_variant());
        let mut props=Properties::new();
        props.insert("Metadata".into(),metadata.to_variant());
        props.insert("Position".into(),130_000_000i64.to_variant());
        for flag in ["CanControl","CanSeek","CanPlay"]{props.insert(flag.into(),true.to_variant());}
        let state=details(identity(),&props);
        assert_eq!(state.artist,"Artist A, Artist B");
        assert_eq!(state.position_us,Some(120_000_000));
        assert!(state.capabilities.seek&&state.capabilities.play&&!state.capabilities.pause);
        props.insert("CanControl".into(),false.to_variant());
        assert!(!details(identity(),&props).capabilities.seek);
        let track=state.track.clone().unwrap();
        let (method,args)=action_call(&state,Action::Seek(track.clone(),40_000_000)).unwrap();
        assert_eq!(method,"SetPosition");assert_eq!(args.unwrap().type_().as_str(),"(ox)");
        let mut stale=track.clone();stale.id="/track/old".into();
        assert!(action_call(&state,Action::Seek(stale,40_000_000)).is_err());
        assert!(action_call(&state,Action::Seek(track.clone(),-1)).is_err());
        assert!(action_call(&state,Action::Seek(track.clone(),track.length_us+1)).is_err());
        assert!(action_call(&details(identity(),&props),Action::Seek(track,0)).is_err());
        assert!(action_call(&state,Action::Pause).is_err());
    }
}
