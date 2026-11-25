//! Review and edit machine-local window exceptions as a draft.
use atelier_config::{appearance_path,AppearanceReview,WindowRule,WindowRules};
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc};
use super::{description,heading};

#[derive(Clone)]
struct Fields{
    id:gtk::Entry,focused:gtk::SpinButton,inactive:gtk::SpinButton,
    radius:gtk::SpinButton,blur:gtk::SpinButton,no_shadow:gtk::CheckButton,
}
impl Fields{
    fn populate(&self,id:&str,rule:WindowRule){
        self.id.set_text(id);
        for (field,value) in [(&self.focused,rule.focused_opacity),(&self.inactive,rule.inactive_opacity),(&self.radius,rule.corner_radius),(&self.blur,rule.backdrop_blur)]{
            field.set_value(value.map_or(-1.0,f64::from));
        }
        self.no_shadow.set_active(rule.disable_shadow);
    }
    fn rule(&self)->WindowRule{
        let value=|spin:&gtk::SpinButton|u32::try_from(spin.value_as_int()).ok();
        WindowRule{focused_opacity:value(&self.focused),inactive_opacity:value(&self.inactive),corner_radius:value(&self.radius),backdrop_blur:value(&self.blur),disable_shadow:self.no_shadow.is_active()}
    }
}
struct Editor{
    review:Option<AppearanceReview>,
    rules:WindowRules,
}
fn spin(parent:&gtk::Box,label:&str,max:f64)->gtk::SpinButton{
    parent.append(&description(label));
    let field=gtk::SpinButton::with_range(-1.0,max,1.0);
    field.set_value(-1.0);parent.append(&field);field
}
fn refresh(list:&gtk::ComboBoxText,rules:&WindowRules){
    list.remove_all();
    for id in rules.ids(){list.append(Some(id),id);}
}
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(p)=>p,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    parent.append(&description("Rules match exact case-sensitive xdg-shell application IDs, not window titles, desktop filenames or executable paths. Fullscreen and maximized windows retain their normal effect exclusions. Use -1 to inherit global settings; opacity overrides must otherwise be 70–100%."));
    let list=gtk::ComboBoxText::new();list.set_hexpand(true);parent.append(&list);
    let edit=gtk::Button::with_label("Edit selected draft rule");parent.append(&edit);
    let remove=gtk::Button::with_label("Remove selected draft rule");parent.append(&remove);
    parent.append(&heading("Rule editor"));
    let id=gtk::Entry::new();id.set_placeholder_text(Some("org.example.Application"));id.set_max_length(128);parent.append(&id);
    let focused=spin(parent,"Focused opacity (-1 inherits)",100.0);
    let inactive=spin(parent,"Inactive opacity (-1 inherits)",100.0);
    let radius=spin(parent,"Corner radius (-1 inherits, 0 square)",24.0);
    let blur=spin(parent,"Focused backdrop blur (-1 inherits, 0 off)",16.0);
    let no_shadow=gtk::CheckButton::with_label("Disable compositor shadows for this application");parent.append(&no_shadow);
    let fields=Fields{id,focused,inactive,radius,blur,no_shadow};
    let put=gtk::Button::with_label("Add or update rule in draft");parent.append(&put);
    let clear=gtk::Button::with_label("Clear editor for a new rule");parent.append(&clear);
    let reload=gtk::Button::with_label("Reload saved rules (discard draft)");parent.append(&reload);
    let previous=gtk::Button::with_label("Load previous rules into draft");parent.append(&previous);
    let save=gtk::Button::with_label("Save all draft rules");save.add_css_class("suggested-action");save.set_sensitive(false);parent.append(&save);
    let status=description("");status.set_selectable(true);parent.append(&status);
    let state=Rc::new(RefCell::new(Editor{review:None,rules:WindowRules::default()}));
    {let (fields,list,state,status,save)=(fields.clone(),list.clone(),state.clone(),status.clone(),save.downgrade());
    reload.connect_clicked(move|_|{
        match AppearanceReview::load(&path){
            Ok(review)=>{
                let mut state=state.borrow_mut();state.rules=review.value.window_rules.clone();state.review=Some(review);
                refresh(&list,&state.rules);fields.populate("",WindowRule::default());
                if let Some(save)=save.upgrade(){save.set_sensitive(true);}
                status.set_text("Saved rules loaded.");
            }
            Err(e)=>{state.borrow_mut().review=None;if let Some(save)=save.upgrade(){save.set_sensitive(false);}status.set_text(&format!("Load failed: {e:#}"));}
        }
    });}
    {let (fields,list,state,status)=(fields.clone(),list.clone(),state.clone(),status.clone());
    edit.connect_clicked(move|_|{
        let Some(id)=list.active_id()else{status.set_text("Select a draft rule first.");return;};
        if let Some(rule)=state.borrow().rules.get(id.as_str()){fields.populate(id.as_str(),*rule);status.set_text("Editing selection. Add or update to place changes into the draft list.");}
    });}
    {let (fields,status)=(fields.clone(),status.clone());
    clear.connect_clicked(move|_|{fields.populate("",WindowRule::default());status.set_text("Editor cleared; draft list unchanged.");});}
    {let (fields,list,state,status)=(fields.clone(),list.clone(),state.clone(),status.clone());
    put.connect_clicked(move|_|{
        let id=fields.id.text().to_string();let mut state=state.borrow_mut();
        match state.rules.insert(id.clone(),fields.rule()){
            Ok(())=>{refresh(&list,&state.rules);list.set_active_id(Some(&id));status.set_text("Rule updated in draft. Save all draft rules to apply.");}
            Err(e)=>status.set_text(&format!("Rule rejected: {e:#}")),
        }
    });}
    {let (list,state,status)=(list.clone(),state.clone(),status.clone());
    remove.connect_clicked(move|_|{
        let Some(id)=list.active_id()else{status.set_text("Select a draft rule first.");return;};
        let mut state=state.borrow_mut();state.rules.remove(id.as_str());refresh(&list,&state.rules);status.set_text("Rule removed from draft; save to apply.");
    });}
    {let (list,state,status,fields)=(list.clone(),state.clone(),status.clone(),fields.clone());
    previous.connect_clicked(move|_|{
        let mut state=state.borrow_mut();let Some(review)=state.review.as_ref()else{status.set_text("Reload first.");return;};
        match review.previous(){
            Ok(a)=>{state.rules=a.window_rules;refresh(&list,&state.rules);fields.populate("",WindowRule::default());status.set_text("Previous rules loaded into draft only.");}
            Err(e)=>status.set_text(&format!("{e:#}")),
        }
    });}
    save.connect_clicked(move|_|{
        let mut state=state.borrow_mut();
        // Refuse to silently discard pending edits to the active editor.
        let id=fields.id.text();let rule=fields.rule();
        if !id.is_empty()&&state.rules.get(id.as_str())!=Some(&rule){status.set_text("Add/update the editor rule or clear the editor before saving.");return;}
        if id.is_empty()&&rule!=WindowRule::default(){status.set_text("Enter an application ID and add the editor rule, or clear it.");return;}
        let rules=state.rules.clone();
        let Some(review)=state.review.as_mut()else{return;};
        let mut next=review.value.clone();next.window_rules=rules;
        match review.save(next){
            Ok(true)=>status.set_text("Saved application effects. Running native compositor applies changes live."),
            Ok(false)=>status.set_text("No changes."),
            Err(e)=>status.set_text(&format!("Save failed: {e:#}. Reload before retrying.")),
        }
    });
    reload.emit_clicked();
}
