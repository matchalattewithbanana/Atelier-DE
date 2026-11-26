//! Optional logind advisory gate for popups, not a lock security boundary.
//! Only the shell process's own session is queried. No user-wide guessing.
use gtk::{gio,glib};
use glib::variant::ToVariant;
use std::{collections::HashMap,sync::mpsc,time::{Duration,Instant}};

const SERVICE:&str="org.freedesktop.login1";
const MANAGER:&str="/org/freedesktop/login1";
const SESSION:&str="org.freedesktop.login1.Session";
const TIMEOUT:i32=1000;
const FRESH:Duration=Duration::from_secs(2);

fn allowed(properties:&HashMap<String,glib::Variant>)->bool{
    properties.get("Active").and_then(|v|v.get::<bool>())==Some(true) &&
        properties.get("LockedHint").and_then(|v|v.get::<bool>())==Some(false)
}
fn query()->Option<bool>{
    let bus=gio::bus_get_sync(gio::BusType::System,None::<&gio::Cancellable>).ok()?;
    let flags=gio::DBusCallFlags::NO_AUTO_START;
    let reply=bus.call_sync(Some(SERVICE),MANAGER,"org.freedesktop.login1.Manager","GetSessionByPID",
        Some(&(std::process::id(),).to_variant()),None,flags,TIMEOUT,None::<&gio::Cancellable>).ok()?;
    if reply.type_().as_str()!="(o)"{return None;}
    let path=reply.child_value(0);
    let path=path.str()?;
    let reply=bus.call_sync(Some(SERVICE),path,"org.freedesktop.DBus.Properties","GetAll",
        Some(&(SESSION,).to_variant()),None,flags,TIMEOUT,None::<&gio::Cancellable>).ok()?;
    let (properties,)=reply.get::<(HashMap<String,glib::Variant>,)>()?;
    Some(allowed(&properties))
}
#[derive(Clone,Copy)]
struct Sample{started:Instant,allowed:bool}
impl Sample{
    fn permits(self,now:Instant)->bool{self.allowed&&now.saturating_duration_since(self.started)<FRESH}
}
struct Probe{receiver:mpsc::Receiver<Sample>,latest:Option<Sample>}
impl Probe{
    fn start()->Self{
        let (sender,receiver)=mpsc::sync_channel(1);
        std::thread::spawn(move||loop{
            let started=Instant::now();
            let allowed=query().unwrap_or(false);
            if sender.send(Sample{started,allowed}).is_err(){break;}
            std::thread::sleep(Duration::from_millis(500));
        });
        Self{receiver,latest:None}
    }
    fn permits(&mut self,now:Instant)->bool{
        loop{
            match self.receiver.try_recv(){
                Ok(sample)=>self.latest=Some(sample),
                Err(mpsc::TryRecvError::Empty)=>break,
                Err(mpsc::TryRecvError::Disconnected)=>{self.latest=None;break;}
            }
        }
        self.latest.is_some_and(|sample|sample.permits(now))
    }
}

/// Disabled by default for compatibility with compositors/session managers
/// that do not report a lock hint. Enabled means unknown/stale is suppressed.
#[derive(Default)]
pub struct Guard{probe:Option<Probe>}
impl Guard{
    pub fn permits(&mut self,enabled:bool,now:Instant)->bool{
        if !enabled{self.probe=None;return true;}
        self.probe.get_or_insert_with(Probe::start).permits(now)
    }
}
/// A blocked interval invalidates all earlier popup deliveries. On resume
/// the presenter must establish a new watermark from a fresh snapshot.
pub struct Gate{permitted:bool,resumed:Option<Instant>}
impl Default for Gate{
    fn default()->Self{Self{permitted:true,resumed:None}}
}
impl Gate{
    pub fn update(&mut self,permitted:bool,now:Instant)->bool{
        if self.permitted==permitted{return false;}
        self.permitted=permitted;
        self.resumed=permitted.then_some(now);
        true
    }
    pub fn accepts(&self,started:Instant)->bool{
        self.permitted&&self.resumed.is_none_or(|resumed|started>=resumed)
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn only_explicit_active_unlocked_allows(){
        let mut properties=HashMap::new();assert!(!allowed(&properties));
        properties.insert("Active".into(),true.to_variant());assert!(!allowed(&properties));
        properties.insert("LockedHint".into(),false.to_variant());assert!(allowed(&properties));
        properties.insert("LockedHint".into(),true.to_variant());assert!(!allowed(&properties));
        properties.insert("LockedHint".into(),false.to_variant());
        properties.insert("Active".into(),false.to_variant());assert!(!allowed(&properties));
        properties.insert("Active".into(),"true".to_variant());assert!(!allowed(&properties));
    }
    #[test]
    fn resume_rejects_snapshots_started_before_new_baseline(){
        let now=Instant::now();let later=now+Duration::from_secs(1);let mut gate=Gate::default();
        assert!(gate.accepts(now));
        assert!(gate.update(false,now));assert!(!gate.accepts(now));
        assert!(gate.update(true,later));assert!(!gate.accepts(now));assert!(gate.accepts(later));
        assert!(!gate.update(true,later));
    }
    #[test]
    fn stale_samples_do_not_allow_popups(){
        let now=Instant::now();let sample=Sample{started:now,allowed:true};
        assert!(sample.permits(now));assert!(!sample.permits(now+FRESH));
        assert!(!Sample{started:now,allowed:false}.permits(now));
    }
}
