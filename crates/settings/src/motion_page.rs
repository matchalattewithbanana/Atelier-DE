//! Local motion settings; portable themes cannot override accessibility.
use atelier_config::{appearance_path,AppearanceReview,Motion};
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc};
use super::description;
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(p)=>p,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    parent.append(&description("Opening fades apply only to ordinary newly mapped native windows without active focused backdrop blur. Input and layout remain unchanged. Reduced motion immediately settles any active fades when preferences are applied."));
    let enabled=gtk::CheckButton::with_label("Animate window opening");parent.append(&enabled);
    parent.append(&description("Opening duration (milliseconds)"));
    let duration=gtk::SpinButton::with_range(80.0,400.0,10.0);duration.set_value(180.0);parent.append(&duration);
    let reduced=gtk::CheckButton::with_label("Reduce motion (overrides opening animation)");parent.append(&reduced);
    let reload=gtk::Button::with_label("Reload saved motion settings");parent.append(&reload);
    let previous=gtk::Button::with_label("Load previous motion settings into draft");parent.append(&previous);
    let reset=gtk::Button::with_label("Reset motion settings in draft");parent.append(&reset);
    let save=gtk::Button::with_label("Save motion preferences");save.add_css_class("suggested-action");save.set_sensitive(false);parent.append(&save);
    let status=description("");status.set_selectable(true);parent.append(&status);
    let state=Rc::new(RefCell::new(None::<AppearanceReview>));
    let populate={let (enabled,duration,reduced)=(enabled.clone(),duration.clone(),reduced.clone());move|value:Motion|{
        enabled.set_active(value.opening_fade_ms>0);
        duration.set_value(if value.opening_fade_ms==0{180.0}else{f64::from(value.opening_fade_ms)});
        reduced.set_active(value.reduced_motion);
    }};
    {let duration=duration.downgrade();enabled.connect_toggled(move|button|{if let Some(d)=duration.upgrade(){d.set_sensitive(button.is_active());}});}
    {let (state,populate,status,save)=(state.clone(),populate.clone(),status.clone(),save.downgrade());
    reload.connect_clicked(move|_|match AppearanceReview::load(&path){
        Ok(review)=>{populate(review.value.motion);*state.borrow_mut()=Some(review);if let Some(s)=save.upgrade(){s.set_sensitive(true);}status.set_text("Saved motion preferences loaded.");}
        Err(e)=>{state.borrow_mut().take();if let Some(s)=save.upgrade(){s.set_sensitive(false);}status.set_text(&format!("Load failed: {e:#}"));}
    });}
    {let (state,populate,status)=(state.clone(),populate.clone(),status.clone());
    previous.connect_clicked(move|_|{
        let state=state.borrow();let Some(review)=state.as_ref()else{return;};
        match review.previous(){Ok(value)=>{populate(value.motion);status.set_text("Previous motion preferences loaded into draft.");}Err(e)=>status.set_text(&format!("{e:#}"))}
    });}
    {let (populate,status)=(populate.clone(),status.clone());reset.connect_clicked(move|_|{populate(Motion::default());status.set_text("Defaults in draft only.");});}
    save.connect_clicked(move|_|{
        let mut state=state.borrow_mut();let Some(review)=state.as_mut()else{return;};
        let mut next=review.value.clone();
        next.motion=Motion{opening_fade_ms:if enabled.is_active(){duration.value_as_int() as u32}else{0},reduced_motion:reduced.is_active()};
        match review.save(next){
            Ok(true)=>status.set_text("Saved. The running native compositor applies motion preferences after active input grabs finish."),
            Ok(false)=>status.set_text("No changes."),
            Err(e)=>status.set_text(&format!("Save failed: {e:#}. Reload before retrying.")),
        }
    });
    reload.emit_clicked();
}
