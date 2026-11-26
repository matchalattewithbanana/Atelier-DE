//! Local notification rules and presets, with explicit draft/save/restore.
use atelier_config::notification_preferences::{self,AppRule,Preferences,Preset,Review};
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc};
use super::description;

struct Draft { review:Review,value:Preferences }
#[derive(Clone)]
struct Fields{
    apps:gtk::ComboBoxText,app:gtk::Entry,popups:gtk::CheckButton,history:gtk::CheckButton,
    presets:gtk::ComboBoxText,name:gtk::Entry,icon:gtk::Entry,urgency:gtk::ComboBoxText,timeout:gtk::SpinButton,
}
impl Fields{
    fn refill(&self,value:&Preferences){
        let selected_app=self.apps.active_id();let selected_preset=self.presets.active_id();
        self.apps.remove_all();self.apps.append(Some(""),"New application rule");
        for name in value.applications().keys(){self.apps.append(Some(name),name);}
        if !selected_app.as_ref().is_some_and(|id|self.apps.set_active_id(Some(id.as_str()))){self.apps.set_active_id(Some(""));}
        self.presets.remove_all();self.presets.append(Some(""),"New custom preset");
        for name in value.custom_presets().keys(){self.presets.append(Some(name),name);}
        if !selected_preset.as_ref().is_some_and(|id|self.presets.set_active_id(Some(id.as_str()))){self.presets.set_active_id(Some(""));}
    }
}
fn entry(parent:&gtk::Box,title:&str,placeholder:&str)->gtk::Entry{
    parent.append(&description(title));let field=gtk::Entry::new();field.set_placeholder_text(Some(placeholder));parent.append(&field);field
}
fn button(parent:&gtk::Box,title:&str)->gtk::Button{let b=gtk::Button::with_label(title);parent.append(&b);b}
fn update_draft(state:&Rc<RefCell<Option<Draft>>>,fields:&Fields,status:&gtk::Label,change:impl FnOnce(&mut Preferences))->bool{
    let next={
        let state=state.borrow();let Some(draft)=state.as_ref()else{status.set_text("Load preferences first.");return false;};
        let mut next=draft.value.clone();change(&mut next);next
    };
    if let Err(error)=next.validate(){status.set_text(&format!("Invalid draft: {error:#}"));return false;}
    if let Some(draft)=state.borrow_mut().as_mut(){draft.value=next.clone();}
    fields.refill(&next);status.set_text("Draft updated. Save to apply.");true
}
pub fn build(parent:&gtk::Box){
    let path=match notification_preferences::path(){Ok(path)=>path,Err(error)=>{parent.append(&description(&format!("{error:#}")));return;}};
    parent.append(&description("Rules match the exact application label shown in the inbox. They are not an identity or security policy. History off prevents future closed-history retention, not active inbox visibility."));
    parent.append(&super::heading("Application rules"));
    let apps=gtk::ComboBoxText::new();parent.append(&apps);
    let app=entry(parent,"Application label","Example App");
    let popups=gtk::CheckButton::with_label("Show notification popups");popups.set_active(true);parent.append(&popups);
    let history=gtk::CheckButton::with_label("Retain closed history");history.set_active(true);parent.append(&history);
    let add_app=button(parent,"Add or update application rule in draft");
    let remove_app=button(parent,"Remove selected application rule from draft");
    parent.append(&super::heading("Custom presets"));
    parent.append(&description("Built-in presets cannot be overridden. Custom presets only define icon, urgency and expiry; they cannot execute commands."));
    let presets=gtk::ComboBoxText::new();parent.append(&presets);
    let name=entry(parent,"Preset name","backup");
    let icon=entry(parent,"Theme icon name","document-save-symbolic");
    let urgency=gtk::ComboBoxText::new();for label in ["Low","Normal","Critical"]{urgency.append_text(label);}parent.append(&urgency);
    parent.append(&description("Expiry milliseconds (0 = persistent, otherwise 1000–60000)"));
    let timeout=gtk::SpinButton::with_range(0.0,60000.0,1000.0);parent.append(&timeout);
    let add_preset=button(parent,"Add or update preset in draft");
    let remove_preset=button(parent,"Remove selected preset from draft");
    parent.append(&super::heading("Save and recovery"));
    let reload=button(parent,"Reload saved notification rules");
    let previous=button(parent,"Load previous rules into draft");
    let reset=button(parent,"Reset all rules and custom presets in draft");
    let save=button(parent,"Save notification rules and presets");save.add_css_class("suggested-action");
    let status=description("");status.set_selectable(true);parent.append(&status);
    let fields=Fields{apps,app,popups,history,presets,name,icon,urgency,timeout};
    let state=Rc::new(RefCell::new(None::<Draft>));
    let editing=[&add_app,&remove_app,&add_preset,&remove_preset,&previous,&reset,&save];
    let enable={let buttons:Vec<_>=editing.iter().map(|b|b.downgrade()).collect();Rc::new(move|enabled|for b in &buttons{if let Some(b)=b.upgrade(){b.set_sensitive(enabled);}})};
    enable(false);
    {
        let state=state.clone();
        let (app,popups,history)=(fields.app.downgrade(),fields.popups.downgrade(),fields.history.downgrade());
        fields.apps.connect_changed(move|p|{
            let Some(id)=p.active_id()else{return;};
            let (Some(app),Some(popups),Some(history))=(app.upgrade(),popups.upgrade(),history.upgrade())else{return;};
            let rule=state.borrow().as_ref().map_or(AppRule::default(),|d|d.value.app(&id));
            app.set_text(&id);popups.set_active(rule.popups);history.set_active(rule.history);
        });
    }
    {
        let state=state.clone();
        let (name,icon,urgency,timeout)=(fields.name.downgrade(),fields.icon.downgrade(),fields.urgency.downgrade(),fields.timeout.downgrade());
        fields.presets.connect_changed(move|p|{
            let Some(id)=p.active_id()else{return;};
            let (Some(name),Some(icon),Some(urgency),Some(timeout))=(name.upgrade(),icon.upgrade(),urgency.upgrade(),timeout.upgrade())else{return;};
            let preset=state.borrow().as_ref().and_then(|d|d.value.custom_presets().get(id.as_str()).cloned()).unwrap_or_default();
            name.set_text(&id);icon.set_text(&preset.icon);urgency.set_active(Some(u32::from(preset.urgency)));timeout.set_value(f64::from(preset.timeout_ms));
        });
    }
    {
        let (state,fields,status)=(state.clone(),fields.clone(),status.clone());
        add_app.connect_clicked(move |_|{
            let name=fields.app.text().to_string();let rule=AppRule{popups:fields.popups.is_active(),history:fields.history.is_active()};
            if update_draft(&state,&fields,&status,|p|p.set_app(name.clone(),rule)){fields.apps.set_active_id(Some(&name));}
        });
    }
    {
        let (state,fields,status)=(state.clone(),fields.clone(),status.clone());
        remove_app.connect_clicked(move |_|{
            let Some(name)=fields.apps.active_id().filter(|id|!id.is_empty())else{status.set_text("Select an existing application rule.");return;};
            update_draft(&state,&fields,&status,|p|p.remove_app(&name));
        });
    }
    {
        let (state,fields,status)=(state.clone(),fields.clone(),status.clone());
        add_preset.connect_clicked(move |_|{
            let name=fields.name.text().trim().to_owned();
            let preset=Preset{icon:fields.icon.text().trim().to_owned(),urgency:fields.urgency.active().unwrap_or(1) as u8,timeout_ms:fields.timeout.value_as_int()};
            if update_draft(&state,&fields,&status,|p|p.set_preset(name.clone(),preset)){fields.presets.set_active_id(Some(&name));}
        });
    }
    {
        let (state,fields,status)=(state.clone(),fields.clone(),status.clone());
        remove_preset.connect_clicked(move |_|{
            let Some(name)=fields.presets.active_id().filter(|id|!id.is_empty())else{status.set_text("Select an existing custom preset.");return;};
            update_draft(&state,&fields,&status,|p|p.remove_preset(&name));
        });
    }
    {
        let (state,fields,status,enable)=(state.clone(),fields.clone(),status.clone(),enable.clone());
        reload.connect_clicked(move |_|match Review::load(&path){
            Ok(review)=>{
                let value=review.value.clone();*state.borrow_mut()=Some(Draft{review,value:value.clone()});
                fields.refill(&value);enable(true);status.set_text("Saved notification rules loaded.");
            }
            Err(error)=>{state.borrow_mut().take();enable(false);status.set_text(&format!("Load failed; file unchanged: {error:#}"));}
        });
    }
    {
        let (state,fields,status)=(state.clone(),fields.clone(),status.clone());
        previous.connect_clicked(move |_|{
            let result={let state=state.borrow();let Some(draft)=state.as_ref()else{return;};draft.review.previous()};
            match result{
                Ok(value)=>{update_draft(&state,&fields,&status,|p|*p=value);status.set_text("Previous preferences loaded into draft. Save to restore.");}
                Err(error)=>status.set_text(&format!("{error:#}")),
            }
        });
    }
    {
        let (state,fields,status)=(state.clone(),fields.clone(),status.clone());
        reset.connect_clicked(move |_|{update_draft(&state,&fields,&status,|p|*p=Preferences::default());});
    }
    save.connect_clicked(move |_|{
        let mut state=state.borrow_mut();let Some(draft)=state.as_mut()else{return;};
        match draft.review.save(draft.value.clone()){
            Ok(true)=>status.set_text("Saved. The running notification daemon reloads these rules automatically."),
            Ok(false)=>status.set_text("No changes."),
            Err(error)=>status.set_text(&format!("Save failed: {error:#}")),
        }
    });
    reload.emit_clicked();
}
