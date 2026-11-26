//! Direct notification bus transport for both shell presenters.
//! Snapshots remain authoritative; signals only request a fresh snapshot.
use gtk::{gio,glib};
use glib::variant::ToVariant;
use std::{cell::RefCell,rc::Rc,sync::{mpsc,Arc,atomic::{AtomicBool,Ordering}},time::Duration};

const NAME:&str="org.freedesktop.Notifications";
const PATH:&str="/org/atelier/Notifications";
const INTERFACE:&str="org.atelier.Notifications1";
const TIMEOUT_MS:i32=3000;

pub enum Request{
    Dismiss(String,u32),Invoke(String,u32,u64,String),Clear(String),Dnd(String,bool),
    Pause(String,u32),Refresh,
    // Only the transport's coalesced signal callback constructs this.
    Invalidate,
}
pub struct Reply<T>{pub snapshot:Result<T,String>,pub action_error:Option<String>}

fn call(method:&str,args:Option<&glib::Variant>)->Result<glib::Variant,String>{
    // The GLib shared session connection is reused; no CLI subprocess or
    // name activation. Calls run exclusively on the transport worker.
    let connection=gio::bus_get_sync(gio::BusType::Session,None::<&gio::Cancellable>).map_err(|e|e.to_string())?;
    connection.call_sync(Some(NAME),PATH,INTERFACE,method,args,None,gio::DBusCallFlags::NO_AUTO_START,
        TIMEOUT_MS,None::<&gio::Cancellable>).map_err(|e|e.to_string())
}
fn execute(request:Request)->Result<(),String>{
    let (method,args)=match request{
        Request::Dismiss(instance,id)=>("Dismiss",(instance,id).to_variant()),
        Request::Invoke(instance,id,revision,key)=>("Invoke",(instance,id,revision,key).to_variant()),
        Request::Clear(instance)=>("ClearHistoryFor",(instance,).to_variant()),
        Request::Dnd(instance,enabled)=>("SetDnd",(instance,enabled).to_variant()),
        Request::Pause(instance,seconds)=>("Pause",(instance,seconds).to_variant()),
        Request::Refresh|Request::Invalidate=>return Ok(()),
    };
    let reply=call(method,Some(&args))?;
    reply.get::<()>().ok_or_else(||"Invalid notification action reply".into())
}
fn snapshot<T>(method:&str,parse:fn(&str)->Result<T,String>)->Result<T,String>{
    let reply=call(method,None)?;
    let (json,)=reply.get::<(String,)>().ok_or("Invalid notification snapshot reply")?;
    parse(&json)
}

struct Subscriptions{connection:gio::DBusConnection,ids:Vec<gio::SignalSubscriptionId>}
impl Drop for Subscriptions{
    fn drop(&mut self){for id in self.ids.drain(..){self.connection.signal_unsubscribe(id);}}
}
/// Lifetime guard held by the GTK presenter, not the worker. Subscription
/// callbacks are dispatched on the GTK context and never perform bus calls.
pub struct Watch{_subscriptions:Rc<RefCell<Option<Subscriptions>>>}
fn watch(sender:mpsc::SyncSender<Request>,queued:Arc<AtomicBool>)->Watch{
    let subscriptions=Rc::new(RefCell::new(None));
    let weak=Rc::downgrade(&subscriptions);
    glib::MainContext::default().spawn_local(async move{
        let Ok(connection)=gio::bus_get_future(gio::BusType::Session).await else{return;};
        let Some(subscriptions)=weak.upgrade()else{return;};
        let wake=move||{
            if !queued.swap(true,Ordering::AcqRel) && sender.try_send(Request::Invalidate).is_err(){
                queued.store(false,Ordering::Release);
            }
        };
        let changed=wake.clone();
        let ids=vec![
            connection.signal_subscribe(Some(NAME),Some(INTERFACE),Some("Changed"),Some(PATH),None,
                gio::DBusSignalFlags::NONE,move|_,_,_,_,_,_|changed()),
            connection.signal_subscribe(Some("org.freedesktop.DBus"),Some("org.freedesktop.DBus"),Some("NameOwnerChanged"),
                Some("/org/freedesktop/DBus"),Some(NAME),gio::DBusSignalFlags::NONE,move|_,_,_,_,_,_|wake()),
        ];
        *subscriptions.borrow_mut()=Some(Subscriptions{connection,ids});
    });
    Watch{_subscriptions:subscriptions}
}

/// One serialized worker per open presenter. The fallback refresh covers
/// older daemons, dropped/coalesced signals and initial subscription races.
/// A bounded queue prevents signal bursts from accumulating snapshots.
pub fn start<T:Send+'static>(method:&'static str,parse:fn(&str)->Result<T,String>)
    ->(mpsc::SyncSender<Request>,mpsc::Receiver<Reply<T>>,Watch){
    let (sender,incoming)=mpsc::sync_channel::<Request>(2);
    let (outgoing,replies)=mpsc::sync_channel::<Reply<T>>(1);
    let queued=Arc::new(AtomicBool::new(false));
    let watcher=watch(sender.clone(),queued.clone());
    std::thread::spawn(move||{
        let mut action_error=None;
        loop{
            if outgoing.send(Reply{snapshot:snapshot(method,parse),action_error:action_error.take()}).is_err(){break;}
            match incoming.recv_timeout(Duration::from_secs(3)){
                Ok(Request::Invalidate)=>{queued.store(false,Ordering::Release);}
                Ok(Request::Refresh)|Err(mpsc::RecvTimeoutError::Timeout)=>{},
                Ok(request)=>{action_error=execute(request).err();}
                Err(mpsc::RecvTimeoutError::Disconnected)=>break,
            }
        }
    });
    (sender,replies,watcher)
}
