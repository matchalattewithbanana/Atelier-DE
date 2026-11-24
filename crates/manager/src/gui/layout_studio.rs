//! Editor for the existing Classic panel, dock and dashboard preferences.
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc,path::PathBuf};
use crate::rice::{self,LayoutReview,LAYOUT_FIELDS};
use super::{background,label};

pub(super) fn build()->gtk::Box{
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Panel, dock & widgets","title-1"));
    outer.append(&label("Atelier Classic · Edit existing shell preferences offline. Use Appearance Studio for panel height, dock height, radius and opacity. No settings are changed until you save.","dim-label"));
    let form=gtk::Box::new(gtk::Orientation::Vertical,8);
    outer.append(&gtk::ScrolledWindow::builder().child(&form).vexpand(true).build());
    let path=gtk::Entry::new();path.set_placeholder_text(Some("Absolute trusted Mango Atelier checkout"));form.append(&path);
    let trust=gtk::CheckButton::with_label("I trust this checkout and have closed all Atelier sessions.");form.append(&trust);
    let target=gtk::ComboBoxText::new();
    for (id,name) in [("mango","MangoWM"),("hyprland","Hyprland"),("niri","Niri")]{target.append(Some(id),name);}
    form.append(&target);
    let load=gtk::Button::with_label("Load layout settings");form.append(&load);
    let editor=gtk::Box::new(gtk::Orientation::Vertical,8);editor.set_sensitive(false);form.append(&editor);
    let mut group="";
    let mut toggles=Vec::new();
    for (_,title,section) in LAYOUT_FIELDS{
        if group!=*section{editor.append(&label(section,"title-3"));group=section;}
        let toggle=gtk::CheckButton::with_label(title);editor.append(&toggle);toggles.push(toggle);
    }
    editor.append(&label("Ordered dock pins","title-3"));
    editor.append(&label("One desktop application ID per line, top to bottom corresponds to left to right. Empty list removes all fixed pins; running applications still appear.","dim-label"));
    let pins=gtk::TextView::new();pins.set_monospace(true);pins.set_wrap_mode(gtk::WrapMode::WordChar);
    editor.append(&gtk::ScrolledWindow::builder().child(&pins).min_content_height(120).build());
    let reset=gtk::Button::with_label("Reset draft");editor.append(&reset);
    let confirmation=gtk::Entry::new();confirmation.set_placeholder_text(Some("Type apply layout"));editor.append(&confirmation);
    let save=gtk::Button::with_label("Save reviewed layout");save.add_css_class("suggested-action");editor.append(&save);
    let status=label("Load settings first.","dim-label");status.set_selectable(true);outer.append(&status);
    let state:Rc<RefCell<Option<LayoutReview>>>=Rc::new(RefCell::new(None));
    for toggle in &toggles{
        let confirmation=confirmation.downgrade();toggle.connect_toggled(move|_|{if let Some(c)=confirmation.upgrade(){c.set_text("");}});
    }
    {let confirmation=confirmation.downgrade();pins.buffer().connect_changed(move|_|{if let Some(c)=confirmation.upgrade(){c.set_text("");}});}
    let invalidate={
        let (state,editor,confirmation)=(state.clone(),editor.downgrade(),confirmation.downgrade());
        Rc::new(move||{state.borrow_mut().take();if let Some(e)=editor.upgrade(){e.set_sensitive(false);}if let Some(c)=confirmation.upgrade(){c.set_text("");}})
    };
    {let (trust,invalidate)=(trust.downgrade(),invalidate.clone());path.connect_changed(move|_|{if let Some(t)=trust.upgrade(){t.set_active(false);}invalidate();});}
    {let invalidate=invalidate.clone();trust.connect_toggled(move|_|invalidate());}
    {let invalidate=invalidate.clone();target.connect_changed(move|_|invalidate());}
    {
        let (state,pins,status)=(state.clone(),pins.downgrade(),status.downgrade());
        let toggles:Vec<_>=toggles.iter().map(|t|t.downgrade()).collect();
        reset.connect_clicked(move|_|{
            let baseline=state.borrow();let Some(review)=baseline.as_ref()else{return;};
            for ((key,_,_),toggle) in LAYOUT_FIELDS.iter().zip(&toggles){if let Some(t)=toggle.upgrade(){t.set_active(review.enabled(key));}}
            if let Some(p)=pins.upgrade(){p.buffer().set_text(&review.pins());}
            if let Some(s)=status.upgrade(){s.set_text("Draft reset; no settings changed.");}
        });
    }
    {
        let (form,path,trust,target,editor,pins,status,confirmation)=(form.downgrade(),path.downgrade(),trust.downgrade(),target.downgrade(),editor.downgrade(),pins.downgrade(),status.downgrade(),confirmation.downgrade());
        let toggles:Vec<_>=toggles.iter().map(|t|t.downgrade()).collect();let state=state.clone();
        load.connect_clicked(move|_|{
            let (Some(f),Some(p),Some(t),Some(target),Some(s))=(form.upgrade(),path.upgrade(),trust.upgrade(),target.upgrade(),status.upgrade())else{return;};
            if !t.is_active(){s.set_text("Confirm trusted checkout and offline state.");return;}
            let Some(target)=target.active_id().map(|v|v.to_string())else{s.set_text("Choose a compositor target.");return;};
            let p=PathBuf::from(p.text().trim());state.borrow_mut().take();
            if let Some(e)=editor.upgrade(){e.set_sensitive(false);}if let Some(c)=confirmation.upgrade(){c.set_text("");}
            f.set_sensitive(false);s.set_text("Reading reviewed layout settings…");
            let (form,editor,pins,status,state,toggles)=(form.clone(),editor.clone(),pins.clone(),status.clone(),state.clone(),toggles.clone());
            background(move||rice::review_layout(&p,&target),move|result|{
                if let (Some(s),Some(e),Some(p))=(status.upgrade(),editor.upgrade(),pins.upgrade()){
                    match result{
                        Ok(review)=>{
                            for ((key,_,_),toggle) in LAYOUT_FIELDS.iter().zip(&toggles){if let Some(t)=toggle.upgrade(){t.set_active(review.enabled(key));}}
                            p.buffer().set_text(&review.pins());*state.borrow_mut()=Some(review);e.set_sensitive(true);s.set_text("Layout draft loaded. No changes made.");
                        }
                        Err(error)=>s.set_text(&format!("Load failed: {error:#}")),
                    }
                }
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}
            });
        });
    }
    {
        let (form,editor,pins,confirmation,status)=(form.downgrade(),editor.downgrade(),pins.downgrade(),confirmation.downgrade(),status.downgrade());
        let toggles:Vec<_>=toggles.iter().map(|t|t.downgrade()).collect();
        save.connect_clicked(move|_|{
            let (Some(f),Some(p),Some(c),Some(s))=(form.upgrade(),pins.upgrade(),confirmation.upgrade(),status.upgrade())else{return;};
            if c.text().as_str()!="apply layout"{s.set_text("Type apply layout to confirm the draft.");return;}
            let Some(toggles)=toggles.iter().map(|t|t.upgrade().map(|t|t.is_active())).collect::<Option<Vec<_>>>()else{return;};
            let buffer=p.buffer();let pins=buffer.text(&buffer.start_iter(),&buffer.end_iter(),true).to_string();
            {
                let baseline=state.borrow();let Some(review)=baseline.as_ref()else{return;};
                match review.changes(&toggles,&pins){
                    Ok(edits) if edits.is_empty()=>{s.set_text("No layout changes.");return;}
                    Ok(_)=>{},
                    Err(e)=>{s.set_text(&format!("{e:#}"));return;}
                }
            }
            let Some(review)=state.borrow_mut().take()else{return;};
            c.set_text("");f.set_sensitive(false);s.set_text("Checking fingerprint and saving recovery snapshot…");
            let (form,editor,status)=(form.clone(),editor.clone(),status.clone());
            background(move||review.apply(toggles,pins),move|result|{
                if let Some(s)=status.upgrade(){match result{Ok(text)=>s.set_text(&text),Err(e)=>s.set_text(&format!("Save failed: {e:#}. Reload settings before retrying."))}}
                if let Some(e)=editor.upgrade(){e.set_sensitive(false);}
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}
            });
        });
    }
    outer
}
