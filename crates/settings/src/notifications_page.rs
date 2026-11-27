//! Notification presentation preferences. Does not start a daemon or shell.
use atelier_config::{appearance_path, AppearanceReview, NotificationAppearance, NotificationPosition, NotificationStyle};
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc};
use super::description;

#[derive(Clone)]
struct Fields {
    style: gtk::ComboBoxText,
    position: gtk::ComboBoxText,
    output: gtk::ComboBoxText,
    privacy: gtk::CheckButton,
    width: gtk::SpinButton,
    cards: gtk::SpinButton,
    timeout: gtk::SpinButton,
    animation: gtk::SpinButton,
    radius: gtk::SpinButton,
    opacity: gtk::SpinButton,
}
impl Fields {
    fn populate(&self, n: &NotificationAppearance) {
        self.style.set_active_id(Some(match n.style {
            NotificationStyle::LaunchDefault => "launch-default", NotificationStyle::Island => "island",
            NotificationStyle::Floating => "floating", NotificationStyle::Compact => "compact",
        }));
        self.position.set_active_id(Some(match n.position {
            NotificationPosition::Automatic => "automatic", NotificationPosition::TopLeft => "top-left",
            NotificationPosition::TopCenter => "top-center", NotificationPosition::TopRight => "top-right",
        }));
        refill_outputs(&self.output,&n.output);
        self.privacy.set_active(n.hide_when_session_unavailable);
        for (field,value) in [(&self.width,n.width),(&self.cards,n.max_cards),(&self.timeout,n.timeout_seconds),
            (&self.animation,n.animation_ms),(&self.radius,n.radius),(&self.opacity,n.opacity)] {
            field.set_value(f64::from(value));
        }
    }
    fn collect(&self) -> NotificationAppearance {
        NotificationAppearance {
            style: match self.style.active_id().as_deref() {
                Some("island") => NotificationStyle::Island, Some("floating") => NotificationStyle::Floating,
                Some("compact") => NotificationStyle::Compact, _ => NotificationStyle::LaunchDefault,
            },
            position: match self.position.active_id().as_deref() {
                Some("top-left") => NotificationPosition::TopLeft, Some("top-center") => NotificationPosition::TopCenter,
                Some("top-right") => NotificationPosition::TopRight, _ => NotificationPosition::Automatic,
            },
            output:self.output.active_id().and_then(|id|id.strip_prefix("output:").map(str::to_owned)).unwrap_or_default(),
            hide_when_session_unavailable:self.privacy.is_active(),
            width:self.width.value_as_int() as u32, max_cards:self.cards.value_as_int() as u32,
            timeout_seconds:self.timeout.value_as_int() as u32, animation_ms:self.animation.value_as_int() as u32,
            radius:self.radius.value_as_int() as u32, opacity:self.opacity.value_as_int() as u32,
        }
    }
}
fn choice(parent:&gtk::Box,title:&str,values:&[(&str,&str)])->gtk::ComboBoxText{
    parent.append(&description(title));
    let field=gtk::ComboBoxText::new();
    for (id,label) in values {field.append(Some(id),label);}
    parent.append(&field);field
}
// Retain a disconnected preference so saving another field cannot silently
// change the chosen output. Reload repopulates the current monitor list.
fn refill_outputs(picker:&gtk::ComboBoxText,selected:&str){
    picker.remove_all();picker.append(Some("panel"),"Follow panel (automatic)");
    let mut found=selected.is_empty();
    if let Some(display)=gtk::gdk::Display::default(){
        let monitors=display.monitors();
        for index in 0..monitors.n_items(){
            let Some(monitor)=monitors.item(index).and_downcast::<gtk::gdk::Monitor>()else{continue;};
            let Some(connector)=monitor.connector()else{continue;};
            let geometry=monitor.geometry();
            picker.append(Some(&format!("output:{connector}")),&format!("{} · {} × {}",connector,geometry.width(),geometry.height()));
            found|=connector.as_str()==selected;
        }
    }
    if !found{picker.append(Some(&format!("output:{selected}")),&format!("{selected} (not connected)"));}
    let id=if selected.is_empty(){"panel".into()}else{format!("output:{selected}")};
    picker.set_active_id(Some(&id));
}
fn spin(parent:&gtk::Box,title:&str,min:f64,max:f64,step:f64)->gtk::SpinButton{
    let row=gtk::Box::new(gtk::Orientation::Horizontal,12);
    let title=description(title);title.set_hexpand(true);row.append(&title);
    let field=gtk::SpinButton::with_range(min,max,step);row.append(&field);parent.append(&row);field
}
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(p)=>p,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    parent.append(&description("These preferences affect an already enabled Atelier popup presenter. Start the shell with --panel --notification-island or --panel --notification-popups to opt in. They do not start or replace a notification daemon."));
    parent.append(&description("Draft preview · Sample notifications only. Simulate the launch mode and advisory session privacy without sending a notification or changing session state."));
    let preview=gtk::DrawingArea::new();preview.set_content_height(270);preview.set_hexpand(true);parent.append(&preview);
    let island=gtk::CheckButton::with_label("Simulate notification island launch mode");parent.append(&island);
    let unavailable=gtk::CheckButton::with_label("Simulate unavailable or locked session");parent.append(&unavailable);
    let fields=Fields{
        style:choice(parent,"Layout",&[("launch-default","Follow shell launch mode"),("island","Notification island"),("floating","Single floating card"),("compact","Compact stack")]),
        position:choice(parent,"Placement",&[("automatic","Automatic"),("top-left","Top left"),("top-center","Top center"),("top-right","Top right")]),
        output:choice(parent,"Notification display",&[]),
        privacy:{let check=gtk::CheckButton::with_label("Hide popups unless this logind session reports active and unlocked");parent.append(&check);check},
        width:spin(parent,"Preferred width (logical pixels)",260.0,480.0,10.0),
        cards:spin(parent,"Maximum stacked cards",1.0,3.0,1.0),
        timeout:spin(parent,"Ordinary card timeout (seconds)",2.0,15.0,1.0),
        animation:spin(parent,"Slide-in duration (0 disables)",0.0,400.0,20.0),
        radius:spin(parent,"Card corner radius",0.0,32.0,1.0),
        opacity:spin(parent,"Background opacity (%)",85.0,100.0,1.0),
    };
    parent.append(&description("Session privacy is advisory: enable only with a locker that updates logind LockedHint. Unknown or inactive sessions hide popups; this is not a security boundary. A disconnected display falls back to the panel display and is selected again when available. Reload to refresh the display list. Animation must be off (0) or 80–400 ms. Reduced motion overrides this setting. Progress and action cards follow their own lifecycle; the timeout affects ordinary cards on their next update."));
    let reload=gtk::Button::with_label("Reload saved notification appearance");
    let defaults=gtk::Button::with_label("Reset notification appearance in draft");
    let previous=gtk::Button::with_label("Load previous notification appearance");
    let save=gtk::Button::with_label("Save notification appearance");save.add_css_class("suggested-action");
    let status=description("");status.set_selectable(true);
    for button in [&reload,&defaults,&previous,&save]{parent.append(button);}parent.append(&status);
    let state=Rc::new(RefCell::new(None::<AppearanceReview>));
    save.set_sensitive(false);previous.set_sensitive(false);
    {
        let (state,fields,island,unavailable)=(state.clone(),fields.clone(),island.clone(),unavailable.clone());
        preview.set_draw_func(move|_,cr,w,h|{
            let state=state.borrow();let Some(review)=state.as_ref()else{return;};
            let mut draft=review.value.clone();draft.notifications=fields.collect();
            super::shell_preview::notifications(cr,w,h,&draft,island.is_active(),unavailable.is_active());
        });
    }
    for field in [&fields.style,&fields.position]{let p=preview.downgrade();field.connect_changed(move|_|{if let Some(p)=p.upgrade(){p.queue_draw();}});}
    for field in [&fields.width,&fields.cards,&fields.radius,&fields.opacity]{let p=preview.downgrade();field.connect_value_changed(move|_|{if let Some(p)=p.upgrade(){p.queue_draw();}});}
    for field in [&fields.privacy,&island,&unavailable]{let p=preview.downgrade();field.connect_toggled(move|_|{if let Some(p)=p.upgrade(){p.queue_draw();}});}
    parent.append(&description("Preview is static; output selection, timeout and animation are not simulated. Palette and panel height come from the loaded native profile."));
    {
        let (state,fields,status,save,previous,preview)=(state.clone(),fields.clone(),status.clone(),save.downgrade(),previous.downgrade(),preview.downgrade());
        reload.connect_clicked(move |_| {
            let result=AppearanceReview::load(&path);
            let enabled=result.is_ok();
            match result {
                Ok(review)=>{fields.populate(&review.value.notifications);*state.borrow_mut()=Some(review);status.set_text("Saved notification preferences loaded.");}
                Err(error)=>{state.borrow_mut().take();status.set_text(&format!("Load failed: {error:#}"));}
            }
            if let Some(button)=save.upgrade(){button.set_sensitive(enabled);}
            if let Some(button)=previous.upgrade(){button.set_sensitive(enabled);}
            if let Some(p)=preview.upgrade(){p.queue_draw();}
        });
    }
    {let (fields,status)=(fields.clone(),status.clone());defaults.connect_clicked(move |_| {
        fields.populate(&NotificationAppearance::default());status.set_text("Defaults in draft only.");
    });}
    {let (fields,state,status)=(fields.clone(),state.clone(),status.clone());previous.connect_clicked(move |_| {
        let state=state.borrow();let Some(review)=state.as_ref() else{return;};
        match review.previous(){Ok(value)=>{fields.populate(&value.notifications);status.set_text("Previous notification preferences loaded into draft.");}
            Err(error)=>status.set_text(&format!("{error:#}"))}
    });}
    save.connect_clicked(move |_| {
        let mut state=state.borrow_mut();let Some(review)=state.as_mut() else{return;};
        let mut next=review.value.clone();next.notifications=fields.collect();
        match review.save(next){
            Ok(true)=>status.set_text("Saved. The running enabled presenter reloads these settings."),
            Ok(false)=>status.set_text("No changes."),
            Err(error)=>status.set_text(&format!("Save failed: {error:#}. Reload if preferences changed elsewhere.")),
        }
    });
    reload.emit_clicked();
}
