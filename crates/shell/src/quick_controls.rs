//! On-demand controls for an unlocked desktop. Provider calls are isolated
//! from GTK; each card owns one worker and one request at a time.
use atelier_config::component;
use atelier_services::{
    audio::{audio_action,audio_default_action,audio_snapshot,AudioAction,AudioSnapshot,Device,Direction},
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
    b.connect_clicked(move |button|{
        if let Some(popover)=button.ancestor(gtk::Popover::static_type()).and_then(|w|w.downcast::<gtk::Popover>().ok()){popover.popdown();}
        match component("atelier-settings").args(["--page",page]).spawn(){
            Ok(mut child)=>{std::thread::spawn(move||{let _=child.wait();});}
            Err(e)=>eprintln!("Atelier settings launch failed: {e}"),
        }
    });
    b
}

type Request<A>=Option<A>; // None requests a fresh snapshot.
type Refresh=Rc<dyn Fn()>;
struct Reply<S>{snapshot:Result<S,String>,action_error:Option<String>,reactive:bool}
/// Serialize provider operations. A pending invalidation is a single bit,
/// not a queue of snapshots; hidden cards never start follow-up reads.
fn worker<S:Send+'static,A:Send+'static>(
    card:&gtk::Box,status:&gtk::Label,read:impl Fn()->anyhow::Result<S>+Send+'static,
    act:impl Fn(A)->anyhow::Result<()>+Send+'static,
    display:impl Fn(Option<S>,bool)+'static,
    write_groups:&[gtk::Box],
)->(Rc<dyn Fn(Request<A>)>,Refresh){
    let (tx,rx)=mpsc::sync_channel::<(Request<A>,bool)>(1);
    let (events,updates)=mpsc::sync_channel::<Reply<S>>(1);
    std::thread::spawn(move||{
        while let Ok((request,reactive))=rx.recv(){
            let action_error=request.and_then(|action|act(action).err()).map(|e|e.to_string());
            let snapshot=read().map_err(|e|e.to_string());
            if events.send(Reply{snapshot,action_error,reactive}).is_err(){break;}
        }
    });
    // Reactive audio reads may leave editors interactive, but all writes
    // remain gated by insensitive parents until the serialized read finishes.
    let weak_groups:Vec<_>=write_groups.iter().map(|group|group.downgrade()).collect();
    let weak=card.downgrade();
    let gate:Rc<dyn Fn(bool,bool)>=Rc::new(move|busy,reactive|{
        if let Some(card)=weak.upgrade(){card.set_sensitive(!busy||(reactive&&!weak_groups.is_empty()));}
        for group in &weak_groups{if let Some(group)=group.upgrade(){group.set_sensitive(!busy);}}
    });
    let busy=Rc::new(Cell::new(false));
    let pending=Rc::new(Cell::new(None::<bool>));
    let updates=Rc::new(updates);
    let display=Rc::new(display);
    let (weak_card,weak_status)=(card.downgrade(),status.downgrade());
    let submit:Rc<dyn Fn(Request<A>,bool)>=Rc::new(move|action,reactive|{
        let (Some(card),Some(status))=(weak_card.upgrade(),weak_status.upgrade())else{return;};
        if !card.is_mapped(){return;}
        if busy.get(){
            if action.is_none(){pending.set(Some(pending.get().unwrap_or(true)&&reactive));}
            else{status.set_text("Refreshing service; retry the action when ready.");}
            return;
        }
        if tx.try_send((action,reactive)).is_err(){
            status.set_text("Service unavailable; reopen controls.");return;
        }
        busy.set(true);gate(true,reactive);
        if !reactive{status.set_text("Updating…");}
        let (weak_card,weak_status)=(weak_card.clone(),weak_status.clone());
        let (busy,pending,updates,display,tx,gate)=(busy.clone(),pending.clone(),updates.clone(),display.clone(),tx.clone(),gate.clone());
        glib::timeout_add_local(Duration::from_millis(100),move||{
            let (Some(card),Some(status))=(weak_card.upgrade(),weak_status.upgrade())else{return glib::ControlFlow::Break;};
            match updates.try_recv(){
                Ok(reply)=>{
                    match reply.snapshot{
                        Ok(snapshot)=>{display(Some(snapshot),reply.reactive);status.set_text(reply.action_error.as_deref().unwrap_or("Up to date"));}
                        Err(e)=>{display(None,reply.reactive);status.set_text(&format!("Unavailable: {}{e}",reply.action_error.map(|e|format!("{e}; ")).unwrap_or_default()));}
                    }
                    if let Some(reactive)=pending.take(){
                        if card.is_mapped()&&tx.try_send((None,reactive)).is_ok(){gate(true,reactive);return glib::ControlFlow::Continue;}
                    }
                    busy.set(false);gate(false,false);
                }
                Err(mpsc::TryRecvError::Empty)=>return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected)=>{
                    pending.take();busy.set(false);display(None,false);gate(false,false);status.set_text("Service worker stopped.");
                }
            }
            glib::ControlFlow::Break
        });
    });
    let request:Rc<dyn Fn(Request<A>)>={let submit=submit.clone();Rc::new(move|action|submit(action,false))};
    let refresh=request.clone();
    card.connect_map(move |_|refresh(None));
    let reactive:Refresh=Rc::new(move||submit(None,true));
    (request,reactive)
}

#[derive(Clone,Debug,PartialEq,Eq)]
struct AudioView{id:String,description:String,volume:Option<u32>,muted:Option<bool>}
fn inventory(snapshot:&AudioSnapshot,direction:Direction)->(&Option<String>,&[Device]){
    match direction{Direction::Output=>(&snapshot.default_output,&snapshot.outputs),Direction::Input=>(&snapshot.default_input,&snapshot.inputs)}
}
/// Ignore channel and mute changes when comparing the device selector.
fn inventory_changed(old:Option<&AudioSnapshot>,new:Option<&AudioSnapshot>,direction:Direction)->bool{
    match (old,new){
        (Some(old),Some(new))=>{
            let (a,ad)=inventory(old,direction);let (b,bd)=inventory(new,direction);
            a!=b||ad.len()!=bd.len()||ad.iter().zip(bd).any(|(a,b)|a.id!=b.id||a.description!=b.description||a.monitor!=b.monitor)
        }
        (None,None)=>false,
        _=>true,
    }
}
fn default_device(snapshot:&AudioSnapshot,direction:Direction)->Option<AudioView>{
    let (id,devices)=inventory(snapshot,direction);
    let id=id.as_ref()?;
    let d=devices.iter().find(|d|d.id==*id)?;
    Some(AudioView{id:d.id.clone(),description:d.description.clone(),volume:average(d),muted:d.muted})
}
fn average(device:&Device)->Option<u32>{
    if device.channels.is_empty(){return None;}
    let sum=device.channels.iter().try_fold(0u64,|sum,c|Some(sum+u64::from(c.percent?)))?;
    Some((sum/device.channels.len() as u64) as u32)
}
#[derive(Default)]
struct AudioDraft{
    view:Option<AudioView>,
    draft:Option<u8>,
    review_required:bool,
    updating:bool,
}
impl AudioDraft{
    fn reconcile(&mut self,next:Option<AudioView>,reactive:bool){
        let same=self.view.as_ref().zip(next.as_ref()).is_some_and(|(a,b)|a.id==b.id);
        if !reactive{self.draft=None;self.review_required=false;}
        else if !same{
            // Losing or changing a reviewed device during an edit requires
            // explicit refresh before accepting input for the new target.
            self.review_required|=self.view.is_some();
            self.draft=None;
        }
        if next.as_ref().and_then(|v|v.volume).is_none(){self.draft=None;}
        self.view=next;
    }
    fn edit(&mut self,percent:u8){
        if !self.updating&&!self.review_required&&self.view.as_ref().and_then(|v|v.volume).is_some(){self.draft=Some(percent.min(100));}
    }
    fn confirmed(&self)->Option<u32>{self.view.as_ref()?.volume}
    fn percent(&self)->Option<u8>{self.confirmed().map(|p|self.draft.unwrap_or(p.min(100) as u8))}
    fn can_apply(&self)->bool{
        !self.review_required&&self.percent().zip(self.confirmed()).is_some_and(|(draft,current)|u32::from(draft)!=current)
    }
    fn volume_action(&self,direction:Direction)->Option<AudioCommand>{
        if !self.can_apply(){return None;}
        Some(AudioCommand{direction,id:self.view.as_ref()?.id.clone(),action:AudioAction::Volume(self.percent()?)})
    }
    fn text(&self)->String{
        if self.review_required{return "Default changed · Refresh to review before editing".into();}
        match (self.confirmed(),self.percent()){
            (Some(current),Some(draft)) if self.draft.is_some()&&current!=u32::from(draft)=>format!("Current: {current}% · Draft: {draft}%"),
            (Some(current),_)=>format!("Current: {current}%"),
            _=>"Volume unavailable".into(),
        }
    }
}
struct AudioCommand{direction:Direction,id:String,action:AudioAction}
struct AudioWidgets{
    section:gtk::Box,device:gtk::Label,choice:gtk::ComboBoxText,make_default:gtk::Button,value:gtk::Label,scale:gtk::Scale,apply:gtk::Button,mute:gtk::Button,
    writes:[gtk::Box;2],
}
impl AudioWidgets{
    fn new(title:&str)->Self{
        let section=gtk::Box::new(gtk::Orientation::Vertical,6);
        section.append(&label(title,"atelier-control-subtitle"));
        let device=label("Checking default device…","dim-label");section.append(&device);
        let choice=gtk::ComboBoxText::new();choice.set_size_request(-1,48);choice.set_sensitive(false);section.append(&choice);
        let make_default=button("Set selected device as default");make_default.set_sensitive(false);
        let default_group=gtk::Box::new(gtk::Orientation::Vertical,0);default_group.append(&make_default);section.append(&default_group);
        let value=label("Volume unavailable","body");section.append(&value);
        let scale=gtk::Scale::with_range(gtk::Orientation::Horizontal,0.0,100.0,1.0);
        scale.add_css_class("atelier-expressive-slider");scale.set_size_request(-1,48);scale.set_draw_value(false);
        scale.set_tooltip_text(Some(&format!("{title} volume draft, apply to confirm")));scale.set_sensitive(false);section.append(&scale);
        let controls=row();let apply=button("Apply volume");apply.add_css_class("suggested-action");let mute=button("Mute");mute.add_css_class("atelier-control-tonal");mute.add_css_class("atelier-control-switch");
        apply.set_hexpand(true);mute.set_hexpand(true);apply.set_sensitive(false);mute.set_sensitive(false);
        controls.append(&mute);controls.append(&apply);section.append(&controls);
        Self{section,device,choice,make_default,value,scale,apply,mute,writes:[default_group,controls]}
    }
    fn weak_inventory(&self,direction:Direction)->impl Fn(Option<&AudioSnapshot>)+'static{
        let (choice,make_default)=(self.choice.downgrade(),self.make_default.downgrade());
        move|snapshot|{
            let (Some(choice),Some(make_default))=(choice.upgrade(),make_default.upgrade())else{return;};
            choice.remove_all();
            if let Some(snapshot)=snapshot{
                let (default,devices)=inventory(snapshot,direction);
                for device in devices{
                    let suffix=if device.monitor{" (monitor)"}else{""};
                    let active=if default.as_ref()==Some(&device.id){" · Default"}else{""};
                    choice.append(Some(&device.id),&format!("{}{suffix}{active}",device.description));
                }
                if let Some(default)=default{choice.set_active_id(Some(default));}
                choice.set_sensitive(!devices.is_empty());
            }else{choice.set_sensitive(false);}
            make_default.set_sensitive(false);
        }
    }
    fn weak_display(&self,state:Rc<RefCell<AudioDraft>>)->impl Fn()+'static{
        let (device,value,scale,apply,mute)=(self.device.downgrade(),self.value.downgrade(),self.scale.downgrade(),self.apply.downgrade(),self.mute.downgrade());
        move||{
            let (Some(device),Some(value),Some(scale),Some(apply),Some(mute))=(device.upgrade(),value.upgrade(),scale.upgrade(),apply.upgrade(),mute.upgrade())else{return;};
            let (view,percent,editable,can_apply,message)={
                let mut state=state.borrow_mut();state.updating=true;
                (state.view.clone(),state.percent(),!state.review_required&&state.confirmed().is_some(),state.can_apply(),state.text())
            };
            if let Some(v)=view{
                device.set_text(&format!("Current default: {}",v.description));
                mute.set_sensitive(v.muted.is_some());mute.set_label(if v.muted==Some(true){"Unmute"}else{"Mute"});
                if v.muted==Some(true){mute.add_css_class("atelier-control-active");}else{mute.remove_css_class("atelier-control-active");}
            }else{
                device.set_text("No default device reported");mute.set_sensitive(false);
                mute.set_label("Mute");mute.remove_css_class("atelier-control-active");
            }
            // Do not disturb an in-progress drag if its draft is unchanged.
            scale.set_sensitive(editable);
            if let Some(percent)=percent{if scale.value().round() as u8!=percent{scale.set_value(f64::from(percent));}}
            value.set_text(&message);apply.set_sensitive(can_apply);
            state.borrow_mut().updating=false;
        }
    }
    fn connect(&self,direction:Direction,state:Rc<RefCell<AudioDraft>>,request:Rc<dyn Fn(Request<AudioCommand>)>){
        let (value,apply)=(self.value.downgrade(),self.apply.downgrade());
        let default_state=state.clone();let button=self.make_default.downgrade();
        self.choice.connect_changed(move|choice|{
            if let Some(button)=button.upgrade(){
                let selected=choice.active_id();
                button.set_sensitive(selected.as_ref().is_some_and(|id|default_state.borrow().view.as_ref().is_none_or(|v|v.id!=id.as_str())));
            }
        });
        let choice=self.choice.downgrade();let send=request.clone();
        self.make_default.connect_clicked(move |_|{
            if let Some(id)=choice.upgrade().and_then(|c|c.active_id()){
                send(Some(AudioCommand{direction,id:id.to_string(),action:AudioAction::Default}));
            }
        });
        let state_for_scale=state.clone();
        self.scale.connect_value_changed(move|scale|{
            let (Some(value),Some(apply))=(value.upgrade(),apply.upgrade())else{return;};
            let mut state=state_for_scale.borrow_mut();
            if state.updating{return;}
            state.edit(scale.value().round().clamp(0.0,100.0) as u8);
            value.set_text(&state.text());apply.set_sensitive(state.can_apply());
        });
        let s=state.clone();let r=request.clone();
        self.apply.connect_clicked(move |_|{
            if let Some(command)=s.borrow().volume_action(direction){r(Some(command));}
        });
        self.mute.connect_clicked(move |_|{
            if let Some(v)=state.borrow().view.as_ref(){
                if let Some(muted)=v.muted{request(Some(AudioCommand{direction,id:v.id.clone(),action:AudioAction::Mute(!muted)}));}
            }
        });
    }
}
fn audio_card()->gtk::Box{
    let card=card("Audio");
    let output=AudioWidgets::new("Output");let input=AudioWidgets::new("Input");
    card.append(&output.section);
    let input_expander=gtk::Expander::new(Some("Microphone and input devices"));
    input_expander.set_child(Some(&input.section));card.append(&input_expander);
    let footer=row();let refresh=button("Refresh");footer.append(&refresh);footer.append(&settings_button("audio"));card.append(&footer);
    let status=label("Checking audio…","dim-label");card.append(&status);
    let out_state=Rc::new(RefCell::new(AudioDraft::default()));let in_state=Rc::new(RefCell::new(AudioDraft::default()));
    let out_display=output.weak_display(out_state.clone());let in_display=input.weak_display(in_state.clone());
    let out_inventory=output.weak_inventory(Direction::Output);let in_inventory=input.weak_inventory(Direction::Input);
    let (out,in_)= (out_state.clone(),in_state.clone());
    let previous=RefCell::new(None::<AudioSnapshot>);
    let writes=[output.writes[0].clone(),output.writes[1].clone(),input.writes[0].clone(),input.writes[1].clone()];
    let (request,reactive)=worker(&card,&status,audio_snapshot,|cmd:AudioCommand|{
        if cmd.action==AudioAction::Default{audio_action(cmd.direction,&cmd.id,cmd.action)}else{audio_default_action(cmd.direction,&cmd.id,cmd.action)}
    },move|snapshot,reactive|{
        let mut previous=previous.borrow_mut();
        if reactive&&*previous==snapshot{return;}
        let ov=snapshot.as_ref().and_then(|s|default_device(s,Direction::Output));
        let iv=snapshot.as_ref().and_then(|s|default_device(s,Direction::Input));
        let old_out=out.borrow().view.clone();let old_in=in_.borrow().view.clone();
        let out_changed=old_out!=ov;let in_changed=old_in!=iv;
        out.borrow_mut().reconcile(ov,reactive);in_.borrow_mut().reconcile(iv,reactive);
        if !reactive||inventory_changed(previous.as_ref(),snapshot.as_ref(),Direction::Output){out_inventory(snapshot.as_ref());}
        if !reactive||inventory_changed(previous.as_ref(),snapshot.as_ref(),Direction::Input){in_inventory(snapshot.as_ref());}
        if !reactive||out_changed{out_display();}
        if !reactive||in_changed{in_display();}
        *previous=snapshot;
    },&writes);
    crate::audio_watch::attach(&card,reactive);
    output.connect(Direction::Output,out_state,request.clone());input.connect(Direction::Input,in_state,request.clone());
    refresh.connect_clicked(move |_|request(None));card
}
fn radio_card<S:Send+'static,A:Send+'static>(
    title:&str,page:&'static str,provider:crate::connectivity_watch::Provider,read:impl Fn()->anyhow::Result<S>+Send+'static,
    act:impl Fn(A)->anyhow::Result<()>+Send+'static,
    state_of:impl Fn(S)->(Option<bool>,String)+'static,command:impl Fn(bool)->A+'static,
)->gtk::Box{
    let card=card(title);
    let detail=label("Checking service…","body");card.append(&detail);
    let actions=row();let toggle=button("Unavailable");toggle.add_css_class("atelier-control-tonal");toggle.add_css_class("atelier-control-switch");toggle.set_hexpand(true);toggle.set_sensitive(false);
    let refresh=button("Refresh");actions.append(&toggle);actions.append(&refresh);card.append(&actions);card.append(&settings_button(page));
    let status=label("Checking…","dim-label");card.append(&status);
    let state=Rc::new(Cell::new(None::<bool>));
    let (weak_toggle,weak_detail,s)=(toggle.downgrade(),detail.downgrade(),state.clone());
    let previous=RefCell::new(None::<(Option<bool>,String)>);
    let (request,reactive)=worker(&card,&status,read,act,move|snapshot,reactive|{
        let (value,summary)=snapshot.map(&state_of).unwrap_or((None,"Provider unavailable".into()));
        if reactive&&previous.borrow().as_ref().is_some_and(|(old,detail)|*old==value&&detail==&summary){return;}
        *previous.borrow_mut()=Some((value,summary.clone()));
        s.set(value);
        if let (Some(toggle),Some(detail))=(weak_toggle.upgrade(),weak_detail.upgrade()){
            detail.set_text(&summary);toggle.set_sensitive(value.is_some());
            toggle.set_label(match value{Some(true)=>"Turn off",Some(false)=>"Turn on",None=>"Unavailable"});
            if value==Some(true){toggle.add_css_class("atelier-control-active");}else{toggle.remove_css_class("atelier-control-active");}
        }
    },&[]);
    crate::connectivity_watch::attach(&card,provider,reactive);
    let send=request.clone();toggle.connect_clicked(move |_|{if let Some(value)=state.get(){send(Some(command(!value)));}});
    refresh.connect_clicked(move |_|request(None));card
}
fn network_summary(s:NetworkSnapshot)->(Option<bool>,String){
    let mut summary=match s.wifi_enabled{Some(true)=>"Wi-Fi enabled",Some(false)=>"Wi-Fi disabled",None=>"Wi-Fi state unknown"}.to_owned();
    let mut active=s.devices.iter().filter(|d|d.kind=="wifi"&&d.state=="connected").filter_map(|d|d.connection.as_deref()).collect::<Vec<_>>();
    active.sort_unstable();active.dedup();
    if s.wifi_enabled==Some(true)&&!active.is_empty(){
        let count=active.len();
        let names=active.iter().take(3).map(|name|name.chars().take(80).collect::<String>()).collect::<Vec<_>>().join(", ");
        summary.push_str(&format!(" · Connected: {names}"));
        if count>3{summary.push_str(&format!(" +{}",count-3));}
    }
    (s.wifi_enabled,summary)
}
fn bluetooth_summary(s:BluetoothSnapshot)->(Option<bool>,String){
    let mut summary=match s.powered{Some(true)=>"Bluetooth enabled",Some(false)=>"Bluetooth disabled",None=>"Bluetooth state unknown"}.to_owned();
    if s.controllers.len()==1{
        let connected=s.paired_devices.iter().filter(|d|d.connected==Some(true)).count();
        let unknown=s.paired_devices.iter().filter(|d|d.connected.is_none()).count();
        summary.push_str(&format!(" · {} paired · {connected} connected",s.paired_devices.len()));
        if unknown>0{summary.push_str(&format!(" · {unknown} connection states unknown"));}
    }else{summary.push_str(&format!(" · {} controllers detected",s.controllers.len()));}
    (s.powered,summary)
}
/// Same view for the panel popover and standalone launcher. Stack pages
/// map independently; audio is not queried until its page is visible.
pub fn content(app:&gtk::Application)->gtk::Box{
    let root=gtk::Box::new(gtk::Orientation::Vertical,10);root.add_css_class("atelier-control-window");
    root.set_margin_top(12);root.set_margin_bottom(12);root.set_margin_start(12);root.set_margin_end(12);
    root.append(&label("Quick controls","atelier-control-heading"));
    let pages=gtk::Stack::new();pages.set_transition_type(gtk::StackTransitionType::Crossfade);
    pages.set_transition_duration(crate::native_appearance().motion.opening_duration().as_millis() as u32);
    pages.set_hhomogeneous(false);pages.set_vhomogeneous(false);
    let switcher=gtk::StackSwitcher::new();switcher.set_stack(Some(&pages));switcher.set_halign(gtk::Align::Center);
    switcher.add_css_class("atelier-control-tabs");root.append(&switcher);
    let connectivity=gtk::Box::new(gtk::Orientation::Vertical,8);
    use crate::connectivity_watch::Provider;
    connectivity.append(&radio_card("Wi-Fi","network",Provider::Network,network_snapshot,network_action,
        network_summary,NetworkAction::Wifi));
    connectivity.append(&radio_card("Bluetooth","bluetooth",Provider::Bluetooth,bluetooth_snapshot,bluetooth_action,
        bluetooth_summary,BluetoothAction::Power));
    pages.add_titled(&connectivity,Some("connectivity"),"Connectivity");
    pages.add_titled(&audio_card(),Some("audio"),"Audio");
    root.append(&pages);
    let media=button("Open media controls");let app=app.downgrade();media.connect_clicked(move |button|{
        if let Some(popover)=button.ancestor(gtk::Popover::static_type()).and_then(|w|w.downcast::<gtk::Popover>().ok()){popover.popdown();}
        if let Some(app)=app.upgrade(){crate::media_controls::show(&app);}
    });root.append(&media);
    root
}
pub fn show(app:&gtk::Application){
    if let Some(window)=app.windows().into_iter().find(|w|w.widget_name()=="atelier-quick-controls"){window.present();return;}
    let window=gtk::ApplicationWindow::builder().application(app).title("Atelier · Quick controls").default_width(480).default_height(580).build();
    window.set_widget_name("atelier-quick-controls");
    let scroll=gtk::ScrolledWindow::builder().child(&content(app)).hscrollbar_policy(gtk::PolicyType::Never).build();
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
    fn selector_inventory_ignores_volume_and_mute_but_not_default_or_device_changes(){
        fn sample()->AudioSnapshot{
            AudioSnapshot{schema:1,outputs:vec![Device{id:"speaker".into(),description:"Speaker".into(),muted:Some(false),monitor:false,channels:vec![Channel{name:"left".into(),percent:Some(50)}]}],inputs:vec![],default_output:Some("speaker".into()),default_input:None}
        }
        let old=sample();let mut new=sample();
        new.outputs[0].channels[0].percent=Some(75);new.outputs[0].muted=Some(true);
        assert!(!inventory_changed(Some(&old),Some(&new),Direction::Output));
        assert_ne!(default_device(&old,Direction::Output),default_device(&new,Direction::Output));
        new.default_output=None;
        assert!(inventory_changed(Some(&old),Some(&new),Direction::Output));
        new=sample();new.outputs[0].description="Renamed".into();
        assert!(inventory_changed(Some(&old),Some(&new),Direction::Output));
        assert!(inventory_changed(Some(&old),None,Direction::Output));
    }
    #[test]
    fn connectivity_summary_does_not_infer_unknown_or_inactive_connections(){
        use atelier_services::{network,bluetooth};
        let wifi=|state:&str,name:&str|network::Device{name:name.into(),kind:"wifi".into(),state:state.into(),connection:Some(name.into())};
        let result=network_summary(NetworkSnapshot{schema:1,wifi_enabled:Some(true),devices:vec![wifi("connected","Home"),wifi("disconnected","Old"),wifi("connected","Home")],profiles:vec![]});
        assert_eq!(result,(Some(true),"Wi-Fi enabled · Connected: Home".into()));
        let result=network_summary(NetworkSnapshot{schema:1,wifi_enabled:None,devices:vec![wifi("connected","Home")],profiles:vec![]});
        assert_eq!(result,(None,"Wi-Fi state unknown".into()));
        let adapter=||bluetooth::Controller{address:"00:11:22:33:44:55".into(),name:"Adapter".into()};
        let device=|connected|bluetooth::Device{address:"AA:BB:CC:DD:EE:FF".into(),name:"Device".into(),connected};
        let result=bluetooth_summary(BluetoothSnapshot{schema:1,controllers:vec![adapter()],powered:Some(true),paired_devices:vec![device(Some(true)),device(None)]});
        assert_eq!(result,(Some(true),"Bluetooth enabled · 2 paired · 1 connected · 1 connection states unknown".into()));
        let result=bluetooth_summary(BluetoothSnapshot{schema:1,controllers:vec![adapter(),adapter()],powered:None,paired_devices:vec![]});
        assert_eq!(result,(None,"Bluetooth state unknown · 2 controllers detected".into()));
    }
    #[test]
    fn missing_default_never_guesses_device(){
        let s=AudioSnapshot{schema:1,outputs:vec![],inputs:vec![],default_output:Some("missing".into()),default_input:None};
        assert!(default_device(&s,Direction::Output).is_none());assert!(default_device(&s,Direction::Input).is_none());
    }
}
