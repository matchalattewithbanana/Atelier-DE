//! On-demand controls for an unlocked desktop. Provider calls are isolated
//! from GTK; each card owns one worker and one request at a time.
use atelier_config::component;
use atelier_services::{
    audio::{audio_action,audio_snapshot,AudioAction,AudioSnapshot,Device,Direction},
    network::{network_action,network_snapshot,NetworkAction,NetworkSnapshot},
    bluetooth::{bluetooth_action,bluetooth_snapshot,BluetoothAction,BluetoothSnapshot},
};
use gtk::{glib,prelude::*};
use std::{cell::{Cell,RefCell},rc::Rc,sync::mpsc,time::Duration};

fn label(text:&str,class:&str)->gtk::Label{
    let l=gtk::Label::new(Some(text));l.set_xalign(0.0);l.set_wrap(true);l.add_css_class(class);l
}
fn button(text:&str)->gtk::Button{
    let b=gtk::Button::with_label(text);b.set_size_request(-1,48);b
}
fn card(title:&str)->gtk::Box{
    let c=gtk::Box::new(gtk::Orientation::Vertical,8);c.add_css_class("atelier-control-card");
    c.append(&label(title,"atelier-control-title"));c
}
fn row()->gtk::Box{gtk::Box::new(gtk::Orientation::Horizontal,8)}
fn settings_button(page:&'static str)->gtk::Button{
    let b=button("Open settings");
    b.connect_clicked(move |_|{
        match component("atelier-settings").args(["--page",page]).spawn(){
            Ok(mut child)=>{std::thread::spawn(move||{let _=child.wait();});}
            Err(e)=>eprintln!("Atelier settings launch failed: {e}"),
        }
    });
    b
}

type Request<A>=Option<A>; // None requests a fresh snapshot.
struct Reply<S>{snapshot:Result<S,String>,action_error:Option<String>}
/// A card is disabled until its authoritative post-action snapshot arrives.
/// No periodic overwrite of volume drafts; refresh is explicit.
fn worker<S:Send+'static,A:Send+'static>(
    card:&gtk::Box,status:&gtk::Label,read:impl Fn()->anyhow::Result<S>+Send+'static,
    act:impl Fn(A)->anyhow::Result<()>+Send+'static,
    display:impl Fn(Option<S>)+'static,
)->Rc<dyn Fn(Request<A>)>{
    let (tx,rx)=mpsc::sync_channel::<Request<A>>(1);
    let (events,updates)=mpsc::sync_channel::<Reply<S>>(1);
    std::thread::spawn(move||{
        while let Ok(request)=rx.recv(){
            let action_error=request.and_then(|action|act(action).err()).map(|e|e.to_string());
            let snapshot=read().map_err(|e|e.to_string());
            if events.send(Reply{snapshot,action_error}).is_err(){break;}
        }
    });
    let busy=Rc::new(Cell::new(false));
    let (weak_card,weak_status)=(card.downgrade(),status.downgrade());
    let request:Rc<dyn Fn(Request<A>)>={
        let busy=busy.clone();let (card,status)=(weak_card.clone(),weak_status.clone());
        Rc::new(move|action|{
            let (Some(card),Some(status))=(card.upgrade(),status.upgrade())else{return;};
            if busy.get(){return;}
            if tx.try_send(action).is_ok(){
                busy.set(true);card.set_sensitive(false);status.set_text("Updating…");
            }else{status.set_text("Service unavailable; refresh or reopen controls.");}
        })
    };
    // Do not retain the card in the polling closure. Display callbacks
    // retain only weak widget references or plain snapshot state.
    glib::timeout_add_local(Duration::from_millis(100),move||{
        let (Some(card),Some(status))=(weak_card.upgrade(),weak_status.upgrade())else{return glib::ControlFlow::Break;};
        match updates.try_recv(){
            Ok(reply)=>{
                busy.set(false);card.set_sensitive(true);
                match reply.snapshot{
                    Ok(snapshot)=>{display(Some(snapshot));status.set_text(reply.action_error.as_deref().unwrap_or("Up to date · Refresh to check external changes."));}
                    Err(e)=>{display(None);status.set_text(&format!("Unavailable: {}{e}",reply.action_error.map(|e|format!("{e}; ")).unwrap_or_default()));}
                }
            }
            Err(mpsc::TryRecvError::Empty)=>{}
            Err(mpsc::TryRecvError::Disconnected)=>{
                busy.set(false);display(None);card.set_sensitive(true);status.set_text("Service worker stopped.");return glib::ControlFlow::Break;
            }
        }
        glib::ControlFlow::Continue
    });
    request(None);request
}

#[derive(Clone,Debug,PartialEq,Eq)]
struct AudioView{id:String,description:String,volume:Option<u32>,muted:Option<bool>}
fn default_device(snapshot:&AudioSnapshot,direction:Direction)->Option<AudioView>{
    let (id,devices)=match direction{Direction::Output=>(&snapshot.default_output,&snapshot.outputs),Direction::Input=>(&snapshot.default_input,&snapshot.inputs)};
    let id=id.as_ref()?;
    let d=devices.iter().find(|d|d.id==*id)?;
    Some(AudioView{id:d.id.clone(),description:d.description.clone(),volume:average(d),muted:d.muted})
}
fn average(device:&Device)->Option<u32>{
    if device.channels.is_empty(){return None;}
    let sum=device.channels.iter().try_fold(0u64,|sum,c|Some(sum+u64::from(c.percent?)))?;
    Some((sum/device.channels.len() as u64) as u32)
}
struct AudioCommand{direction:Direction,id:String,action:AudioAction}
struct AudioWidgets{
    section:gtk::Box,device:gtk::Label,value:gtk::Label,scale:gtk::Scale,apply:gtk::Button,mute:gtk::Button,
}
impl AudioWidgets{
    fn new(title:&str)->Self{
        let section=gtk::Box::new(gtk::Orientation::Vertical,6);
        section.append(&label(title,"atelier-control-subtitle"));
        let device=label("Checking device…","dim-label");section.append(&device);
        let value=label("Volume unavailable","body");section.append(&value);
        let scale=gtk::Scale::with_range(gtk::Orientation::Horizontal,0.0,100.0,1.0);
        scale.add_css_class("atelier-expressive-slider");scale.set_size_request(-1,48);scale.set_draw_value(false);
        scale.set_tooltip_text(Some(&format!("{title} volume draft, apply to confirm")));scale.set_sensitive(false);section.append(&scale);
        let controls=row();let apply=button("Apply volume");let mute=button("Mute");
        apply.set_hexpand(true);mute.set_hexpand(true);apply.set_sensitive(false);mute.set_sensitive(false);
        controls.append(&mute);controls.append(&apply);section.append(&controls);
        Self{section,device,value,scale,apply,mute}
    }
    fn weak_display(&self)->impl Fn(Option<AudioView>)+'static{
        let (device,value,scale,apply,mute)=(self.device.downgrade(),self.value.downgrade(),self.scale.downgrade(),self.apply.downgrade(),self.mute.downgrade());
        move|view|{
            let (Some(device),Some(value),Some(scale),Some(apply),Some(mute))=(device.upgrade(),value.upgrade(),scale.upgrade(),apply.upgrade(),mute.upgrade())else{return;};
            if let Some(v)=view{
                device.set_text(&v.description);
                if let Some(percent)=v.volume{
                    scale.set_value(f64::from(percent.min(100)));scale.set_sensitive(true);
                    value.set_text(&format!("Current: {percent}%"));
                }else{scale.set_sensitive(false);value.set_text("Volume unavailable");}
                apply.set_sensitive(v.volume.is_some_and(|p|p>100));
                mute.set_sensitive(v.muted.is_some());mute.set_label(if v.muted==Some(true){"Unmute"}else{"Mute"});
                if v.muted==Some(true){mute.add_css_class("atelier-control-active");}else{mute.remove_css_class("atelier-control-active");}
            }else{
                device.set_text("No default device reported");value.set_text("Volume unavailable");scale.set_sensitive(false);apply.set_sensitive(false);mute.set_sensitive(false);
                mute.set_label("Mute");mute.remove_css_class("atelier-control-active");
            }
        }
    }
    fn connect(&self,direction:Direction,state:Rc<RefCell<Option<AudioView>>>,request:Rc<dyn Fn(Request<AudioCommand>)>){
        let (value,apply)=(self.value.downgrade(),self.apply.downgrade());
        let state_for_scale=state.clone();
        self.scale.connect_value_changed(move|scale|{
            let (Some(value),Some(apply))=(value.upgrade(),apply.upgrade())else{return;};
            let percent=scale.value().round() as u32;
            if let Some(v)=state_for_scale.borrow().as_ref(){
                if let Some(current)=v.volume{
                    value.set_text(&format!("Current: {current}% · Draft: {percent}%"));
                    apply.set_sensitive(percent!=current);
                }
            }
        });
        let scale=self.scale.downgrade();let s=state.clone();let r=request.clone();
        self.apply.connect_clicked(move |_|{
            if let (Some(scale),Some(v))=(scale.upgrade(),s.borrow().as_ref()){
                r(Some(AudioCommand{direction,id:v.id.clone(),action:AudioAction::Volume(scale.value().round().clamp(0.0,100.0) as u8)}));
            }
        });
        self.mute.connect_clicked(move |_|{
            if let Some(v)=state.borrow().as_ref(){
                if let Some(muted)=v.muted{request(Some(AudioCommand{direction,id:v.id.clone(),action:AudioAction::Mute(!muted)}));}
            }
        });
    }
}
fn audio_card()->gtk::Box{
    let card=card("Audio");
    let output=AudioWidgets::new("Output");let input=AudioWidgets::new("Input");
    card.append(&output.section);card.append(&input.section);
    let footer=row();let refresh=button("Refresh");footer.append(&refresh);footer.append(&settings_button("audio"));card.append(&footer);
    let status=label("Checking audio…","dim-label");card.append(&status);
    let out_state=Rc::new(RefCell::new(None));let in_state=Rc::new(RefCell::new(None));
    let out_display=output.weak_display();let in_display=input.weak_display();
    let (out,in_)= (out_state.clone(),in_state.clone());
    let request=worker(&card,&status,audio_snapshot,|cmd:AudioCommand|audio_action(cmd.direction,&cmd.id,cmd.action),move|snapshot|{
        let ov=snapshot.as_ref().and_then(|s|default_device(s,Direction::Output));
        let iv=snapshot.as_ref().and_then(|s|default_device(s,Direction::Input));
        *out.borrow_mut()=ov.clone();*in_.borrow_mut()=iv.clone();
        out_display(ov);in_display(iv);
    });
    output.connect(Direction::Output,out_state,request.clone());input.connect(Direction::Input,in_state,request.clone());
    refresh.connect_clicked(move |_|request(None));card
}
fn radio_card<S:Send+'static,A:Send+'static>(
    title:&str,page:&'static str,read:impl Fn()->anyhow::Result<S>+Send+'static,
    act:impl Fn(A)->anyhow::Result<()>+Send+'static,
    state_of:impl Fn(S)->(Option<bool>,String)+'static,command:impl Fn(bool)->A+'static,
)->gtk::Box{
    let card=card(title);
    let detail=label("Checking service…","body");card.append(&detail);
    let actions=row();let toggle=button("Unavailable");toggle.set_hexpand(true);toggle.set_sensitive(false);
    let refresh=button("Refresh");actions.append(&toggle);actions.append(&refresh);card.append(&actions);card.append(&settings_button(page));
    let status=label("Checking…","dim-label");card.append(&status);
    let state=Rc::new(Cell::new(None::<bool>));
    let (weak_toggle,weak_detail,s)=(toggle.downgrade(),detail.downgrade(),state.clone());
    let request=worker(&card,&status,read,act,move|snapshot|{
        let (value,summary)=snapshot.map(&state_of).unwrap_or((None,"Provider unavailable".into()));
        s.set(value);
        if let (Some(toggle),Some(detail))=(weak_toggle.upgrade(),weak_detail.upgrade()){
            detail.set_text(&summary);toggle.set_sensitive(value.is_some());
            toggle.set_label(match value{Some(true)=>"Turn off",Some(false)=>"Turn on",None=>"Unavailable"});
            if value==Some(true){toggle.add_css_class("atelier-control-active");}else{toggle.remove_css_class("atelier-control-active");}
        }
    });
    let send=request.clone();toggle.connect_clicked(move |_|{if let Some(value)=state.get(){send(Some(command(!value)));}});
    refresh.connect_clicked(move |_|request(None));card
}
pub fn show(app:&gtk::Application){
    if let Some(window)=app.windows().into_iter().find(|w|w.widget_name()=="atelier-quick-controls"){window.present();return;}
    let window=gtk::ApplicationWindow::builder().application(app).title("Atelier · Quick controls").default_width(480).default_height(680).build();
    window.set_widget_name("atelier-quick-controls");
    let content=gtk::Box::new(gtk::Orientation::Vertical,12);content.add_css_class("atelier-control-window");
    content.set_margin_top(16);content.set_margin_bottom(16);content.set_margin_start(16);content.set_margin_end(16);
    content.append(&label("Quick controls","atelier-control-heading"));
    content.append(&label("Changes apply to this desktop session. No elevated privileges requested.","dim-label"));
    content.append(&radio_card("Wi-Fi","network",network_snapshot,network_action,
        |s:NetworkSnapshot|(s.wifi_enabled,match s.wifi_enabled{Some(true)=>"Wi-Fi enabled",Some(false)=>"Wi-Fi disabled",None=>"Wi-Fi state unknown"}.into()),NetworkAction::Wifi));
    content.append(&radio_card("Bluetooth","bluetooth",bluetooth_snapshot,bluetooth_action,
        |s:BluetoothSnapshot|(s.powered,match s.powered{Some(true)=>"Bluetooth enabled",Some(false)=>"Bluetooth disabled",None=>"Bluetooth state unknown"}.into()),BluetoothAction::Power));
    content.append(&audio_card());
    let media=button("Media controls");let app=app.downgrade();media.connect_clicked(move |_|{if let Some(app)=app.upgrade(){crate::media_controls::show(&app);}});content.append(&media);
    let scroll=gtk::ScrolledWindow::builder().child(&content).hscrollbar_policy(gtk::PolicyType::Never).build();
    window.set_child(Some(&scroll));window.present();
}
#[cfg(test)]
mod tests{
    use super::*;
    use atelier_services::audio::Channel;
    #[test]
    fn average_requires_known_channels_and_preserves_amplified_values(){
        let mut d=Device{id:"speaker".into(),description:"Speaker".into(),muted:Some(false),monitor:false,channels:vec![Channel{name:"left".into(),percent:Some(110)},Channel{name:"right".into(),percent:Some(130)}]};
        assert_eq!(average(&d),Some(120));d.channels[1].percent=None;assert_eq!(average(&d),None);
    }
    #[test]
    fn missing_default_never_guesses_device(){
        let s=AudioSnapshot{schema:1,outputs:vec![],inputs:vec![],default_output:Some("missing".into()),default_input:None};
        assert!(default_device(&s,Direction::Output).is_none());assert!(default_device(&s,Direction::Input).is_none());
    }
}
