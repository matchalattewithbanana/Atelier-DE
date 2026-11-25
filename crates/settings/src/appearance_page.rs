//! Independent native appearance draft and previous-version recovery.
use atelier_config::{appearance_path,Appearance,AppearanceReview};
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc};
use super::description;

#[derive(Clone)]
struct Fields{
    accent:gtk::Entry,height:gtk::SpinButton,spacing:gtk::SpinButton,radius:gtk::SpinButton,icons:gtk::SpinButton,
    toggles:Vec<gtk::CheckButton>,
    dock:gtk::CheckButton,auto_hide:gtk::CheckButton,hide_fullscreen:gtk::CheckButton,dock_icons:gtk::SpinButton,pins:gtk::TextView,
}
impl Fields{
    fn populate(&self,p:&Appearance){
        self.accent.set_text(&p.accent);
        self.dock.set_active(p.dock_enabled);self.auto_hide.set_active(p.dock_auto_hide);self.hide_fullscreen.set_active(p.dock_hide_fullscreen);self.dock_icons.set_value(p.dock_icon_size as f64);self.pins.buffer().set_text(&p.dock_pins.join("\n"));
        for (field,value) in [(&self.height,p.panel_height),(&self.spacing,p.panel_spacing),(&self.radius,p.button_radius),(&self.icons,p.launcher_icon_size)]{field.set_value(value as f64);}
        for (field,value) in self.toggles.iter().zip([p.show_date,p.show_battery,p.show_media,p.show_notifications,p.show_audio,p.show_terminal]){field.set_active(value);}
    }
    fn collect(&self,baseline:&Appearance)->Appearance{
        let buffer=self.pins.buffer();let pins=buffer.text(&buffer.start_iter(),&buffer.end_iter(),true);
        Appearance{dock_enabled:self.dock.is_active(),dock_auto_hide:self.auto_hide.is_active(),dock_hide_fullscreen:self.hide_fullscreen.is_active(),dock_icon_size:self.dock_icons.value_as_int() as u32,
            dock_pins:pins.lines().map(str::trim).filter(|s|!s.is_empty()).map(str::to_owned).collect(),accent:self.accent.text().to_string(),panel_height:self.height.value_as_int() as u32,panel_spacing:self.spacing.value_as_int() as u32,
            button_radius:self.radius.value_as_int() as u32,launcher_icon_size:self.icons.value_as_int() as u32,
            show_date:self.toggles[0].is_active(),show_battery:self.toggles[1].is_active(),show_media:self.toggles[2].is_active(),
            show_notifications:self.toggles[3].is_active(),show_audio:self.toggles[4].is_active(),show_terminal:self.toggles[5].is_active(),..baseline.clone()}
    }
}
fn spin(parent:&gtk::Box,title:&str,min:f64,max:f64)->gtk::SpinButton{
    let row=gtk::Box::new(gtk::Orientation::Horizontal,12);
    let label=description(title);label.set_hexpand(true);row.append(&label);
    let spin=gtk::SpinButton::with_range(min,max,1.0);row.append(&spin);parent.append(&row);spin
}
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(p)=>p,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    let accent=gtk::Entry::new();accent.set_placeholder_text(Some("#RRGGBB or blank for GTK default"));parent.append(&description("Accent for highlighted native shell buttons"));parent.append(&accent);
    let height=spin(parent,"Panel minimum height",32.0,80.0);
    let spacing=spin(parent,"Panel item spacing",0.0,24.0);
    let radius=spin(parent,"Button radius",0.0,24.0);
    let icons=spin(parent,"Launcher icon size",16.0,64.0);
    let toggles:Vec<_>=["Show date","Show battery","Show media button","Show notifications button","Show audio button","Show terminal button"].iter().map(|title|{let field=gtk::CheckButton::with_label(title);parent.append(&field);field}).collect();
    parent.append(&super::heading("Application dock"));
    parent.append(&description("Optional bottom dock in panel mode. Native task support requires a compatible compositor. External desktops use launch-only mode."));
    let dock=gtk::CheckButton::with_label("Enable bottom application dock");parent.append(&dock);
    let auto_hide=gtk::CheckButton::with_label("Auto-hide dock; reveal at bottom center");parent.append(&auto_hide);
    let hide_fullscreen=gtk::CheckButton::with_label("Hide dock during native fullscreen");parent.append(&hide_fullscreen);
    let dock_icons=spin(parent,"Dock icon size",16.0,64.0);
    parent.append(&description("Choose an installed application to append, or edit the exact case-sensitive desktop IDs below. Order is left to right."));
    let picker=gtk::ComboBoxText::new();picker.set_hexpand(true);
    let mut apps:Vec<_>=gtk::gio::AppInfo::all().into_iter().filter(|app|app.should_show()).collect();
    apps.sort_by_key(|app|app.display_name().to_lowercase());
    let mut seen=std::collections::HashSet::new();
    for app in apps{
        if let Some(id)=app.id(){
            if id.ends_with(".desktop") && seen.insert(id.to_string()){
                picker.append(Some(&id),&format!("{} ({id})",app.display_name()));
            }
        }
    }
    let add=gtk::Button::with_label("Add pin");
    let row=gtk::Box::new(gtk::Orientation::Horizontal,8);row.append(&picker);row.append(&add);parent.append(&row);
    let pins=gtk::TextView::new();pins.set_monospace(true);pins.set_wrap_mode(gtk::WrapMode::WordChar);
    parent.append(&gtk::ScrolledWindow::builder().child(&pins).min_content_height(100).build());
    {let picker=picker.downgrade();let pins=pins.downgrade();add.connect_clicked(move |_|{
        let (Some(picker),Some(pins))=(picker.upgrade(),pins.upgrade())else{return;};
        let Some(id)=picker.active_id()else{return;};let buffer=pins.buffer();
        let text=buffer.text(&buffer.start_iter(),&buffer.end_iter(),true);
        if !text.lines().any(|line|line.trim()==id.as_str()){
            let text=text.trim_end();buffer.set_text(&if text.is_empty(){id.to_string()}else{format!("{text}\n{id}")});
        }
    });}
    let fields=Fields{accent,height,spacing,radius,icons,toggles,dock,auto_hide,hide_fullscreen,dock_icons,pins};
    let status=description("Load saved appearance to begin.");status.set_selectable(true);
    let reload=gtk::Button::with_label("Load saved appearance");
    let defaults=gtk::Button::with_label("Preview defaults");
    let previous=gtk::Button::with_label("Load previous appearance into draft");
    let save=gtk::Button::with_label("Save native appearance");save.add_css_class("suggested-action");
    for button in [&reload,&defaults,&previous,&save]{parent.append(button);}parent.append(&status);
    let state=Rc::new(RefCell::new(None::<AppearanceReview>));
    let enable={let (defaults,previous,save)=(defaults.downgrade(),previous.downgrade(),save.downgrade());Rc::new(move|enabled|{for button in [&defaults,&previous,&save]{if let Some(b)=button.upgrade(){b.set_sensitive(enabled);}}})};
    enable(false);
    {
        let (fields,state,status,enable)=(fields.clone(),state.clone(),status.clone(),enable.clone());
        reload.connect_clicked(move |_|match AppearanceReview::load(&path){
            Ok(review)=>{fields.populate(&review.value);*state.borrow_mut()=Some(review);enable(true);status.set_text("Draft loaded. Colors, radius and panel controls update live; dock structure and existing launcher icon sizes require reopening or restart.");}
            Err(e)=>{state.borrow_mut().take();enable(false);status.set_text(&format!("Load failed; existing files unchanged: {e:#}"));}
        });
    }
    {let (fields,status)=(fields.clone(),status.clone());defaults.connect_clicked(move |_|{fields.populate(&Appearance::default());status.set_text("Defaults in draft only.");});}
    {let (fields,state,status)=(fields.clone(),state.clone(),status.clone());previous.connect_clicked(move |_|{
        let baseline=state.borrow();let Some(r)=baseline.as_ref()else{return;};
        match r.previous(){Ok(value)=>{fields.populate(&value);status.set_text("Previous appearance loaded into draft. Save to restore.");},Err(e)=>status.set_text(&format!("{e:#}"))}
    });}
    save.connect_clicked(move |_|{
        let mut baseline=state.borrow_mut();let Some(r)=baseline.as_mut()else{return;};
        let next=fields.collect(&r.value);
        match r.save(next){
            Ok(true)=>status.set_text("Appearance saved. Colors, radius and panel controls update live. Restart shell for dock structure; reopen launcher for icon size."),
            Ok(false)=>status.set_text("No appearance changes."),
            Err(e)=>status.set_text(&format!("Save failed: {e:#}")),
        }
    });
    reload.emit_clicked();
}
