//! System-bus invalidations for the optional NetworkManager and BlueZ
//! providers. Signals never supply authoritative values or trigger writes.
use gtk::{gio,glib,prelude::*};
use std::{cell::{Cell,RefCell},collections::HashMap,rc::Rc,time::Duration};

#[derive(Clone,Copy)]
pub enum Provider{Network,Bluetooth}
impl Provider{
    fn name(self)->&'static str{match self{Self::Network=>"org.freedesktop.NetworkManager",Self::Bluetooth=>"org.bluez"}}
    fn properties(self,interface:&str,key:&str)->bool{
        match (self,interface){
            (Self::Network,"org.freedesktop.NetworkManager")=>matches!(key,"WirelessEnabled"|"WirelessHardwareEnabled"|"NetworkingEnabled"|"ActiveConnections"|"Devices"|"State"),
            (Self::Network,"org.freedesktop.NetworkManager.Device")=>matches!(key,"State"|"ActiveConnection"|"Interface"|"Managed"),
            (Self::Network,"org.freedesktop.NetworkManager.Connection.Active")=>matches!(key,"State"|"Id"|"Devices"),
            (Self::Bluetooth,"org.bluez.Adapter1")=>matches!(key,"Powered"|"Alias"|"Name"|"Address"),
            (Self::Bluetooth,"org.bluez.Device1")=>matches!(key,"Paired"|"Connected"|"Alias"|"Name"|"Address"),
            _=>false,
        }
    }
    fn changed(self,parameters:&glib::Variant)->bool{
        let Some((interface,changed,invalidated))=parameters.get::<(String,HashMap<String,glib::Variant>,Vec<String>)>()else{return false;};
        changed.keys().chain(invalidated.iter()).any(|key|self.properties(&interface,key))
    }
    fn structure(self,interface:&str,member:&str,parameters:&glib::Variant)->bool{
        match self{
            Self::Network=>matches!((interface,member),
                ("org.freedesktop.NetworkManager","DeviceAdded"|"DeviceRemoved")|
                ("org.freedesktop.NetworkManager.Settings","NewConnection"|"ConnectionRemoved")|
                ("org.freedesktop.NetworkManager.Settings.Connection","Updated"|"Removed")),
            Self::Bluetooth=>{
                if interface!="org.freedesktop.DBus.ObjectManager"{return false;}
                let relevant=|name:&str|matches!(name,"org.bluez.Adapter1"|"org.bluez.Device1");
                match member{
                    "InterfacesAdded"=>parameters.get::<(glib::variant::ObjectPath,HashMap<String,HashMap<String,glib::Variant>>)>().is_some_and(|(_,interfaces)|interfaces.keys().any(|k|relevant(k))),
                    "InterfacesRemoved"=>parameters.get::<(glib::variant::ObjectPath,Vec<String>)>().is_some_and(|(_,interfaces)|interfaces.iter().any(|k|relevant(k))),
                    _=>false,
                }
            }
        }
    }
}
struct Subscription{connection:gio::DBusConnection,ids:Vec<gio::SignalSubscriptionId>}
impl Drop for Subscription{
    fn drop(&mut self){for id in self.ids.drain(..){self.connection.signal_unsubscribe(id);}}
}
fn subscribe(connection:gio::DBusConnection,provider:Provider,invalidate:Rc<dyn Fn()>)->Subscription{
    let changed=invalidate.clone();let structure=invalidate.clone();
    let name=provider.name();
    let mut ids=vec![
        connection.signal_subscribe(Some(name),Some("org.freedesktop.DBus.Properties"),Some("PropertiesChanged"),None,None,
            gio::DBusSignalFlags::NONE,move|_,_,_,_,_,parameters|{if provider.changed(parameters){changed();}}),
        connection.signal_subscribe(Some("org.freedesktop.DBus"),Some("org.freedesktop.DBus"),Some("NameOwnerChanged"),Some("/org/freedesktop/DBus"),Some(name),
            gio::DBusSignalFlags::NONE,move|_,_,_,_,_,_|invalidate()),
    ];
    let interfaces:&[&str]=match provider{
        Provider::Network=>&["org.freedesktop.NetworkManager","org.freedesktop.NetworkManager.Settings","org.freedesktop.NetworkManager.Settings.Connection"],
        Provider::Bluetooth=>&["org.freedesktop.DBus.ObjectManager"],
    };
    for interface in interfaces{
        let structure=structure.clone();
        ids.push(connection.signal_subscribe(Some(name),Some(interface),None,None,None,
            gio::DBusSignalFlags::NONE,move|_,_,_,interface,member,parameters|{if provider.structure(interface,member,parameters){structure();}}));
    }
    Subscription{connection,ids}
}
#[derive(Default)]
struct State{
    epoch:u64,
    subscription:Option<Subscription>,
    connecting:bool,
    fallback:Option<glib::SourceId>,
}
fn connect(state:&Rc<RefCell<State>>,epoch:u64,provider:Provider,invalidate:Rc<dyn Fn()>){
    {
        let mut state=state.borrow_mut();
        if state.epoch!=epoch||state.connecting{return;}
        if state.subscription.as_ref().is_some_and(|s|!s.connection.is_closed()){return;}
        state.subscription.take();state.connecting=true;
    }
    let weak=Rc::downgrade(state);
    glib::MainContext::default().spawn_local(async move{
        let connection=gio::bus_get_future(gio::BusType::System).await;
        let Some(state)=weak.upgrade()else{return;};
        {
            let mut state=state.borrow_mut();
            if state.epoch!=epoch{return;}
            state.connecting=false;
            if let Ok(connection)=connection{state.subscription=Some(subscribe(connection,provider,invalidate.clone()));}
        }
        // Close the gap between the initial worker read and subscription.
        invalidate();
    });
}
fn stop(state:&mut State){
    state.epoch=state.epoch.wrapping_add(1);
    state.subscription.take();state.connecting=false;
    if let Some(source)=state.fallback.take(){source.remove();}
}
pub fn attach(card:&gtk::Box,provider:Provider,refresh:Rc<dyn Fn()>){
    let life=Rc::new(RefCell::new(State::default()));
    let state=life.clone();
    card.connect_map(move|card|{
        let epoch={let mut state=state.borrow_mut();stop(&mut state);state.epoch};
        let weak=card.downgrade();let life=Rc::downgrade(&state);
        let queued=Rc::new(Cell::new(false));let refresh=refresh.clone();
        let invalidate:Rc<dyn Fn()>=Rc::new(move||{
            if queued.replace(true){return;}
            let (weak,life,queued,refresh)=(weak.clone(),life.clone(),queued.clone(),refresh.clone());
            glib::timeout_add_local_once(Duration::from_millis(500),move||{
                queued.set(false);
                if weak.upgrade().is_some_and(|c|c.is_mapped())&&life.upgrade().is_some_and(|s|s.borrow().epoch==epoch){refresh();}
            });
        });
        connect(&state,epoch,provider,invalidate.clone());
        let weak=card.downgrade();let life=Rc::downgrade(&state);
        let source=glib::timeout_add_local(Duration::from_secs(30),move||{
            let (Some(card),Some(life))=(weak.upgrade(),life.upgrade())else{return glib::ControlFlow::Break;};
            if life.borrow().epoch!=epoch{return glib::ControlFlow::Break;}
            if !card.is_mapped(){
                life.borrow_mut().fallback.take();
                return glib::ControlFlow::Break;
            }
            connect(&life,epoch,provider,invalidate.clone());
            invalidate();glib::ControlFlow::Continue
        });
        state.borrow_mut().fallback=Some(source);
    });
    card.connect_unmap(move |_|stop(&mut life.borrow_mut()));
}
#[cfg(test)]
mod tests{
    use super::*;
    use glib::variant::ToVariant;
    #[test]
    fn property_invalidation_ignores_discovery_noise_and_matches_invalidations(){
        let properties=|interface:&str,key:&str| (interface.to_owned(),HashMap::from([(key.to_owned(),true.to_variant())]),Vec::<String>::new()).to_variant();
        assert!(Provider::Network.changed(&properties("org.freedesktop.NetworkManager","WirelessEnabled")));
        assert!(Provider::Network.changed(&properties("org.freedesktop.NetworkManager.Device","ActiveConnection")));
        assert!(!Provider::Network.changed(&properties("org.freedesktop.NetworkManager.AccessPoint","Strength")));
        assert!(Provider::Bluetooth.changed(&properties("org.bluez.Device1","Connected")));
        assert!(!Provider::Bluetooth.changed(&properties("org.bluez.Device1","RSSI")));
        let invalidated=("org.bluez.Adapter1".to_owned(),HashMap::<String,glib::Variant>::new(),vec!["Powered".to_owned()]).to_variant();
        assert!(Provider::Bluetooth.changed(&invalidated));
        assert!(!Provider::Bluetooth.changed(&"bad".to_variant()));
    }
    #[test]
    fn structural_events_are_scoped_to_inventory(){
        let path=glib::variant::ObjectPath::try_from("/org/bluez/hci0").unwrap();
        let removed=(path,vec!["org.bluez.Adapter1".to_owned()]).to_variant();
        assert!(Provider::Bluetooth.structure("org.freedesktop.DBus.ObjectManager","InterfacesRemoved",&removed));
        assert!(!Provider::Bluetooth.structure("other","InterfacesRemoved",&removed));
        assert!(Provider::Network.structure("org.freedesktop.NetworkManager","DeviceAdded",&removed));
        assert!(!Provider::Network.structure("org.freedesktop.NetworkManager.Device.Wireless","AccessPointAdded",&removed));
    }
}
