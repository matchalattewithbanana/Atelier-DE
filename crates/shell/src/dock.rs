//! Optional pinned launcher and native window tasks. External sessions stay launch-only.
use atelier_config::{Appearance,component};
use gtk::{glib,prelude::*};
use std::{cell::RefCell,rc::Rc,time::Duration};
use atelier_config::tasks::{TaskSnapshot,TaskWindow};
use std::collections::{HashMap,HashSet};
use crate::task_backend::{self,TaskBackend};
use gtk4_layer_shell::{Edge,KeyboardMode,Layer,LayerShell};
use crate::applications;

/// Owns both dock surfaces and polling sources. Dropping it removes the
/// reservation and stops callbacks before a replacement dock is constructed.
pub struct Dock {
    window:gtk::ApplicationWindow,
    tasks:Option<glib::SourceId>,
    visibility:crate::dock_visibility::Visibility,
}
impl Drop for Dock{
    fn drop(&mut self){
        if let Some(source)=self.tasks.take(){source.remove();}
        self.visibility.stop();
        self.window.destroy();
    }
}
pub fn build(app:&gtk::Application,appearance:&Appearance)->Dock{
    let window=gtk::ApplicationWindow::builder().application(app).title("Atelier dock").decorated(false).resizable(false).build();
    window.add_css_class("atelier-dock-layer");
    window.init_layer_shell();window.set_namespace(Some("atelier-dock"));window.set_layer(Layer::Top);
    window.set_anchor(Edge::Bottom,true);window.set_keyboard_mode(KeyboardMode::None);
    // Auto-hide overlays the workspace, persistent mode reserves space.
    if appearance.dock_auto_hide{window.set_exclusive_zone(0);}else{window.auto_exclusive_zone_enable();}
    let row=gtk::Box::new(gtk::Orientation::Horizontal,6);row.add_css_class("atelier-dock");
    let launcher=gtk::Button::with_label("Apps");launcher.set_tooltip_text(Some("Open application launcher"));
    let app_ref=app.downgrade();
    launcher.connect_clicked(move |_|{if let Some(app)=app_ref.upgrade(){crate::build_shell(&app);}});
    row.append(&launcher);
    let backend=TaskBackend::detect().map(Rc::new);
    let snapshot:Rc<RefCell<Option<TaskSnapshot>>>=Rc::new(RefCell::new(None));
    let mut pins=Vec::new();
    let apps=applications::installed();
    for id in &appearance.dock_pins{
        let Some(info)=apps.iter().find(|info|info.id().as_deref()==Some(id.as_str())).cloned()else{
            let missing=gtk::Button::with_label("?");
            missing.set_tooltip_text(Some(&format!("Unavailable desktop entry: {id}")));
            missing.set_sensitive(false);row.append(&missing);continue;
        };
        let name=info.display_name();
        let button=gtk::Button::new();button.set_tooltip_text(Some(&name));
        let icon=info.icon().map(|icon|gtk::Image::from_gicon(&icon)).unwrap_or_else(||gtk::Image::from_icon_name("application-x-executable"));
        icon.set_pixel_size(appearance.dock_icon_size as i32);
        let indicator=Indicator::new();
        let tile=gtk::Box::new(gtk::Orientation::Vertical,3);
        tile.append(&icon);tile.append(&indicator.root);button.set_child(Some(&tile));
        button.add_css_class("atelier-dock-tile");
        button.update_property(&[gtk::accessible::Property::Label(&name)]);
        let native_tasks=backend.is_some();
        let (backend,snapshot,pin)=(backend.clone(),snapshot.clone(),id.clone());
        button.connect_clicked(move |button|{
            if let Some(backend)=&backend{
                if let Some(state)=snapshot.borrow().as_ref(){
                    if let Some(target)=task_backend::next_window(state,&pin){
                        if let Err(error)=backend.activate(state,target.id){button.set_tooltip_text(Some(&format!("Focus request failed: {error}")));}
                        return;
                    }
                }
            }
            match applications::launch(&info){
                Ok(())=>button.set_tooltip_text(Some(&info.display_name())),
                Err(error)=>{eprintln!("Dock launch failed: {error}");button.set_tooltip_text(Some(&format!("Launch failed: {error}")));}
            }
        });
        if native_tasks{button.set_tooltip_text(Some(&format!("{name} · task state not yet available")));}
        pins.push(Pin{id:id.clone(),name:name.to_string(),button:button.downgrade(),indicator:indicator.downgrade()});
        row.append(&button);
    }
    let tasks=gtk::Box::new(gtk::Orientation::Horizontal,4);row.append(&tasks);
    let settings=gtk::Button::with_label("Settings");settings.set_tooltip_text(Some("Configure dock appearance"));
    settings.connect_clicked(move |_|{
        match component("atelier-settings").args(["--page","shell-appearance"]).spawn(){
            Ok(mut child)=>{std::thread::spawn(move||{let _=child.wait();});}
            Err(error)=>eprintln!("Cannot open dock settings: {error}"),
        }
    });row.append(&settings);
    let scroll=gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Automatic).vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(true).propagate_natural_height(true).max_content_width(640).child(&row).build();
    let weak_scroll=scroll.downgrade();
    window.connect_map(move|window|{
        if let (Some(scroll),Some(surface))=(weak_scroll.upgrade(),window.surface()){
            if let Some(monitor)=surface.display().monitor_at_surface(&surface){
                scroll.set_max_content_width((monitor.geometry().width()-24).clamp(80,640));
            }
        }
    });
    window.set_child(Some(&scroll));window.present();
    let tasks=backend.map(|backend|watch_tasks(&window,&tasks,backend,snapshot.clone(),pins));
    let visibility=crate::dock_visibility::start(app,&window,snapshot,appearance.dock_auto_hide,appearance.dock_hide_fullscreen);
    Dock{window,tasks,visibility}
}


/// Fixed indicator slot: the mark changes shape without shifting the icon.
struct Indicator{root:gtk::Box,mark:gtk::Box,count:gtk::Label}
struct WeakIndicator{mark:glib::WeakRef<gtk::Box>,count:glib::WeakRef<gtk::Label>}
impl Indicator{
    fn new()->Self{
        let root=gtk::Box::new(gtk::Orientation::Horizontal,3);
        root.set_halign(gtk::Align::Center);root.set_size_request(32,12);
        let slot=gtk::Box::new(gtk::Orientation::Horizontal,0);slot.set_size_request(22,8);
        slot.set_halign(gtk::Align::Center);slot.set_valign(gtk::Align::Center);
        let mark=gtk::Box::new(gtk::Orientation::Horizontal,0);mark.set_halign(gtk::Align::Center);
        mark.set_hexpand(true);mark.add_css_class("atelier-dock-mark");mark.set_opacity(0.0);
        slot.append(&mark);root.append(&slot);
        let count=gtk::Label::new(None);count.add_css_class("atelier-dock-count");count.set_width_chars(4);count.set_visible(false);
        root.append(&count);Self{root,mark,count}
    }
    fn downgrade(&self)->WeakIndicator{WeakIndicator{mark:self.mark.downgrade(),count:self.count.downgrade()}}
    fn update(&self,count:usize,active:bool,partial:bool){self.downgrade().update(count,active,partial);}
}
impl WeakIndicator{
    fn update(&self,count:usize,active:bool,partial:bool){
        if let (Some(mark),Some(label))=(self.mark.upgrade(),self.count.upgrade()){
            mark.set_opacity(if count>0{1.0}else{0.0});
            if active{mark.add_css_class("atelier-dock-mark-focused");}else{mark.remove_css_class("atelier-dock-mark-focused");}
            label.set_visible(count>1||partial);
            label.set_text(&if partial{if count==0{"?".into()}else{format!("{count}+")}}else{count.to_string()});
        }
    }
}
struct Pin{id:String,name:String,button:glib::WeakRef<gtk::Button>,indicator:WeakIndicator}
#[derive(Debug,PartialEq,Eq)]
struct PinState{count:usize,active:bool,partial:bool}
fn pin_state(snapshot:&TaskSnapshot,id:&str)->PinState{
    let mut count=0;let mut active=false;
    for window in &snapshot.windows{
        if task_backend::matches_pin(id,&window.app_id){count+=1;active|=window.focused;}
    }
    PinState{count,active,partial:snapshot.truncated}
}
impl Pin{
    fn update(&self,state:Option<&TaskSnapshot>){
        let Some(button)=self.button.upgrade()else{return;};
        let (count,active,partial,text)=if let Some(state)=state{
            let PinState{count,active,partial}=pin_state(state,&self.id);
            let text=if partial{format!("{} · {count} known windows (partial inventory)",self.name)}
                else if count>0{format!("{} · {count} window(s) · click to focus/cycle",self.name)}
                else{self.name.clone()};
            (count,active,partial,text)
        }else{(0,false,true,format!("{} · task inventory unavailable",self.name))};
        self.indicator.update(count,active,partial);
        set_focused(&button,active);
        button.set_tooltip_text(Some(&text));
        button.update_property(&[gtk::accessible::Property::Label(&text)]);
    }
}
fn set_focused(button:&gtk::Button,active:bool){
    if active{button.add_css_class("suggested-action");}else{button.remove_css_class("suggested-action");}
}
#[derive(Clone,Copy,Debug,Hash,PartialEq,Eq)]
struct Key{session:[u8;16],id:u64}
impl Key{
    fn of(state:&TaskSnapshot,window:&TaskWindow)->Self{Self{session:state.session,id:window.id}}
}
struct TaskTile{button:gtk::Button,label:gtk::Label,indicator:Indicator,last:Option<TaskWindow>}
impl TaskTile{
    fn new(key:Key,backend:Rc<TaskBackend>,snapshot:Rc<RefCell<Option<TaskSnapshot>>>)->Self{
        let label=gtk::Label::new(None);label.set_max_width_chars(14);label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let indicator=Indicator::new();
        let content=gtk::Box::new(gtk::Orientation::Vertical,3);content.append(&label);content.append(&indicator.root);
        let button=gtk::Button::new();button.add_css_class("atelier-dock-tile");button.set_child(Some(&content));
        button.connect_clicked(move|button|{
            let state=snapshot.borrow();
            if let Some(state)=state.as_ref().filter(|s|s.session==key.session&&s.windows.iter().any(|w|w.id==key.id)){
                if let Err(error)=backend.activate(state,key.id){button.set_tooltip_text(Some(&format!("Focus request failed: {error}")));}
            }
        });
        Self{button,label,indicator,last:None}
    }
    fn update(&mut self,task:&TaskWindow){
        if self.last.as_ref()==Some(task){return;}
        let title=if task.title.is_empty(){if task.app_id.is_empty(){"Window"}else{&task.app_id}}else{&task.title};
        let description=format!("{title} · workspace {}{}",task.workspace,if task.focused{" · focused"}else{""});
        self.label.set_text(title);self.button.set_tooltip_text(Some(&description));
        self.button.update_property(&[gtk::accessible::Property::Label(&description)]);
        self.indicator.update(1,task.focused,false);set_focused(&self.button,task.focused);
        self.last=Some(task.clone());
    }
}
fn watch_tasks(window:&gtk::ApplicationWindow,container:&gtk::Box,backend:Rc<TaskBackend>,
    snapshot:Rc<RefCell<Option<TaskSnapshot>>>,pins:Vec<Pin>)->glib::SourceId{
    let window=window.downgrade();let container=container.downgrade();
    let mut tiles:HashMap<Key,TaskTile>=HashMap::new();
    let mut initialized=false;
    let more=gtk::Label::new(Some("More windows…"));more.set_visible(false);
    if let Some(container)=container.upgrade(){container.append(&more);}
    let tick=move||{
        let (Some(_window),Some(container))=(window.upgrade(),container.upgrade())else{return glib::ControlFlow::Continue;};
        let next=backend.snapshot().ok();
        if initialized&&*snapshot.borrow()==next{return glib::ControlFlow::Continue;}
        initialized=true;*snapshot.borrow_mut()=next;
        let state=snapshot.borrow();
        for pin in &pins{pin.update(state.as_ref());}
        let wanted:HashSet<_>=state.as_ref().into_iter().flat_map(|s|s.windows.iter().map(|w|Key::of(s,w))).collect();
        tiles.retain(|key,tile|{
            if wanted.contains(key){true}else{container.remove(&tile.button);false}
        });
        let mut previous:Option<gtk::Widget>=None;
        if let Some(state)=state.as_ref(){
            for task in &state.windows{
                let key=Key::of(state,task);
                let tile=tiles.entry(key).or_insert_with(||{
                    let tile=TaskTile::new(key,backend.clone(),snapshot.clone());
                    container.append(&tile.button);tile
                });
                tile.update(task);
                // Reorder existing children only when the actual order changes.
                if tile.button.prev_sibling()!=previous{container.reorder_child_after(&tile.button,previous.as_ref());}
                previous=Some(tile.button.clone().upcast());
            }
        }
        more.set_visible(state.as_ref().is_some_and(|s|s.truncated));
        if more.prev_sibling()!=previous{container.reorder_child_after(&more,previous.as_ref());}
        glib::ControlFlow::Continue
    };
    // Bounded local inventory reads; no process scans or new worker.
    let mut tick=tick;tick();glib::timeout_add_local(Duration::from_millis(500),tick)
}

#[cfg(test)]
mod tests{
    use super::*;
    fn sample()->TaskSnapshot{
        TaskSnapshot{version:1,session:[1;16],truncated:true,fullscreen_active:None,windows:vec![
            TaskWindow{id:1,app_id:"org.example.App".into(),title:"First".into(),workspace:1,focused:true},
            TaskWindow{id:2,app_id:"org.example.App.desktop".into(),title:"Second".into(),workspace:2,focused:false},
            TaskWindow{id:3,app_id:"org.example.Other".into(),title:"Other".into(),workspace:1,focused:false},
        ]}
    }
    #[test]
    fn pin_counts_exact_matches_and_marks_partial_inventory(){
        let state=sample();
        assert_eq!(pin_state(&state,"org.example.App.desktop"),PinState{count:2,active:true,partial:true});
        assert_eq!(pin_state(&state,"org.example.app.desktop"),PinState{count:0,active:false,partial:true});
    }
    #[test]
    fn tile_identity_survives_title_focus_and_workspace_changes_but_not_restart(){
        let mut state=sample();let key=Key::of(&state,&state.windows[0]);
        state.windows[0].title="Changed".into();state.windows[0].workspace=3;state.windows[0].focused=false;
        assert_eq!(key,Key::of(&state,&state.windows[0]));
        state.session=[2;16];assert_ne!(key,Key::of(&state,&state.windows[0]));
    }
}
