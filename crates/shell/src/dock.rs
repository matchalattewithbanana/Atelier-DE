//! Optional pinned launcher and native window tasks. External sessions stay launch-only.
use atelier_config::{Appearance,component};
use gtk::{glib,prelude::*};
use std::{cell::RefCell,rc::Rc,time::Duration};
use atelier_config::tasks::TaskSnapshot;
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
        button.set_child(Some(&icon));
        button.update_property(&[gtk::accessible::Property::Label(&name)]);
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
        pins.push((id.clone(),name.to_string(),button.downgrade()));
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
    window.set_child(Some(&scroll));window.present();
    let tasks=backend.map(|backend|watch_tasks(&window,&tasks,backend,snapshot.clone(),pins));
    let visibility=crate::dock_visibility::start(app,&window,snapshot,appearance.dock_auto_hide,appearance.dock_hide_fullscreen);
    Dock{window,tasks,visibility}
}


fn watch_tasks(window:&gtk::ApplicationWindow,container:&gtk::Box,backend:Rc<TaskBackend>,
    snapshot:Rc<RefCell<Option<TaskSnapshot>>>,pins:Vec<(String,String,glib::WeakRef<gtk::Button>)>)->glib::SourceId{
    let window=window.downgrade();let container=container.downgrade();
    let tick=move||{
        let (Some(_window),Some(container))=(window.upgrade(),container.upgrade())else{return glib::ControlFlow::Continue;};
        let next=backend.snapshot().ok();
        if *snapshot.borrow()==next{return glib::ControlFlow::Continue;}
        *snapshot.borrow_mut()=next;
        let state=snapshot.borrow();
        for (id,name,button) in &pins{
            let Some(button)=button.upgrade()else{continue;};
            let count=state.as_ref().map_or(0,|s|s.windows.iter().filter(|w|task_backend::matches_pin(id,&w.app_id)).count());
            let active=state.as_ref().is_some_and(|s|s.windows.iter().any(|w|w.focused&&task_backend::matches_pin(id,&w.app_id)));
            if active{button.add_css_class("suggested-action");}else{button.remove_css_class("suggested-action");}
            if count>0{button.add_css_class("running-task");button.set_tooltip_text(Some(&format!("{name} · {count} window(s) · click to focus/cycle")));}
            else{button.remove_css_class("running-task");button.set_tooltip_text(Some(name));}
        }
        while let Some(child)=container.first_child(){container.remove(&child);}
        if let Some(state)=state.as_ref(){
            for task in &state.windows{
                // Include every window, even if its app ID is absent or doesn't
                // match a desktop entry. Individual controls never guess an ID.
                let title=if task.title.is_empty(){if task.app_id.is_empty(){"Window"}else{&task.app_id}}else{&task.title};
                let label=gtk::Label::new(Some(title));label.set_max_width_chars(14);label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                let button=gtk::Button::new();button.set_child(Some(&label));
                button.set_tooltip_text(Some(&format!("{title} · workspace {}",task.workspace)));
                if task.focused{button.add_css_class("suggested-action");}
                let backend=backend.clone();let snapshot=snapshot.clone();let session=state.session;let id=task.id;
                button.connect_clicked(move|button|{
                    let state=snapshot.borrow();
                    if let Some(state)=state.as_ref().filter(|s|s.session==session){
                        if let Err(error)=backend.activate(state,id){button.set_tooltip_text(Some(&format!("Focus request failed: {error}")));}
                    }
                });
                container.append(&button);
            }
            if state.truncated{container.append(&gtk::Label::new(Some("More windows…")));}
        }
        glib::ControlFlow::Continue
    };
    // Run on the GTK thread using only bounded local file reads and a
    // nonblocking datagram on clicks. No child processes or overlapping workers.
    tick();glib::timeout_add_local(Duration::from_millis(500),tick)
}
