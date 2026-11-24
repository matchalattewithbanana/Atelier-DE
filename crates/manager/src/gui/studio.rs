//! Offline Classic appearance draft editor. No setting is written by preview.
use gtk::prelude::*;
use std::{cell::RefCell,rc::Rc,path::PathBuf};
use crate::rice::{self, AppearanceReview, APPEARANCE_FIELDS};
use super::{background,label};

fn color(value:&str)->Option<(f64,f64,f64)> {
    let s=value.strip_prefix('#')?;
    if s.len()!=6 || !s.bytes().all(|b|b.is_ascii_hexdigit()) {return None;}
    let n=u32::from_str_radix(s,16).ok()?;
    Some((((n>>16)&255) as f64/255.0,((n>>8)&255) as f64/255.0,(n&255) as f64/255.0))
}
fn field_text(value:&serde_json::Value)->String {
    value.as_str().map(str::to_owned).or_else(||value.as_u64().map(|n|n.to_string())).unwrap_or_default()
}
pub(super) fn build()->gtk::Box {
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Appearance Studio","title-1"));
    outer.append(&label("Atelier Classic · Offline appearance editing. Native Atelier Settings remains independent. Close all Mango Atelier sessions and use a trusted checkout with the reviewed appearance API.","dim-label"));
    let form=gtk::Box::new(gtk::Orientation::Vertical,10);
    let path=gtk::Entry::new();path.set_placeholder_text(Some("Absolute path to trusted Mango Atelier checkout"));form.append(&path);
    let trust=gtk::CheckButton::with_label("I trust this checkout and have closed all Mango Atelier sessions.");form.append(&trust);
    let target=gtk::ComboBoxText::new();
    for (id,name) in [("mango","MangoWM"),("hyprland","Hyprland"),("niri","Niri")]{target.append(Some(id),name);}
    form.append(&target);
    let load=gtk::Button::with_label("Load appearance");form.append(&load);
    let editor=gtk::Box::new(gtk::Orientation::Vertical,8);editor.set_sensitive(false);
    let state:Rc<RefCell<Option<AppearanceReview>>>=Rc::new(RefCell::new(None));
    let presets=gtk::ComboBoxText::new();
    let preset_row=gtk::Box::new(gtk::Orientation::Horizontal,8);
    presets.set_hexpand(true);preset_row.append(&presets);
    let use_preset=gtk::Button::with_label("Use preset in draft");preset_row.append(&use_preset);
    let reset=gtk::Button::with_label("Reset draft");preset_row.append(&reset);
    editor.append(&label("Bundled presets","title-3"));editor.append(&preset_row);
    let fields:Vec<gtk::Entry>=APPEARANCE_FIELDS.iter().map(|(_,_,title,range)|{
        let entry=gtk::Entry::new();
        let description=range.map(|(min,max)|format!("{title} ({min}–{max})")).unwrap_or_else(||title.to_string());
        editor.append(&label(&description,"body"));editor.append(&entry);entry
    }).collect();
    editor.append(&label("Blank fields preserve existing values. Draft illustration only; not a live shell render.","dim-label"));
    let preview=gtk::DrawingArea::new();preview.set_content_height(145);preview.set_hexpand(true);
    {
        let state=state.clone();let draft:Vec<_>=fields.iter().map(|e|e.downgrade()).collect();
        preview.set_draw_func(move|_,cr,w,h|{
            let state=state.borrow();
            let palette=state.as_ref().map(|r|&r.palette);
            let get=|key:&str,fallback:(f64,f64,f64)|APPEARANCE_FIELDS.iter().position(|(k,_,_,_)|*k==key)
                .and_then(|i|draft[i].upgrade()).and_then(|entry|color(entry.text().as_str()))
                .or_else(||palette.and_then(|p|p[key].as_str()).and_then(color)).unwrap_or(fallback);
            let (r,g,b)=get("background",(0.08,0.09,0.12));cr.set_source_rgb(r,g,b);let _=cr.paint();
            let (r,g,b)=get("surface",(0.15,0.18,0.22));cr.set_source_rgb(r,g,b);
            cr.rectangle(12.0,12.0,(w as f64-24.0).max(0.0),30.0);let _=cr.fill();
            cr.rectangle(12.0,54.0,(w as f64*0.55-18.0).max(0.0),(h as f64-66.0).max(0.0));let _=cr.fill();
            let (r,g,b)=get("accent",(0.65,0.72,1.0));
            cr.set_source_rgb(r,g,b);cr.rectangle(w as f64*0.65,65.0,w as f64*0.23,26.0);let _=cr.fill();
            cr.rectangle(w as f64*0.35,h as f64-22.0,w as f64*0.30,8.0);let _=cr.fill();
            for (i,key) in ["background","surface","elevated","text","muted","accent","border","success","danger"].iter().enumerate(){
                let (r,g,b)=get(key,(0.4,0.4,0.4));cr.set_source_rgb(r,g,b);
                let width=(w as f64-24.0).max(0.0)/9.0;cr.rectangle(12.0+i as f64*width,h as f64-12.0,(width-2.0).max(0.0),8.0);let _=cr.fill();
            }
        });
    }
    editor.append(&preview);
    let approval=gtk::Entry::new();approval.set_placeholder_text(Some("Type apply appearance"));editor.append(&approval);
    let apply=gtk::Button::with_label("Save reviewed appearance");apply.add_css_class("suggested-action");editor.append(&apply);
    form.append(&editor);
    outer.append(&gtk::ScrolledWindow::builder().child(&form).vexpand(true).build());
    let status=label("Load a palette before editing.","dim-label");status.set_selectable(true);outer.append(&status);
    for field in &fields{
        let (approval,preview)=(approval.downgrade(),preview.downgrade());
        field.connect_changed(move|_|{if let Some(a)=approval.upgrade(){a.set_text("");}if let Some(p)=preview.upgrade(){p.queue_draw();}});
    }
    {
        let (state,presets,status)=(state.clone(),presets.downgrade(),status.downgrade());
        let fields:Vec<_>=fields.iter().map(|e|e.downgrade()).collect();
        use_preset.connect_clicked(move|_|{
            let Some(id)=presets.upgrade().and_then(|p|p.active_id())else{return;};
            let baseline=state.borrow();
            let Some(review)=baseline.as_ref()else{return;};
            let Some(preset)=review.presets.iter().find(|p|p.id==id.as_str())else{return;};
            for ((key,_,_,_),field) in APPEARANCE_FIELDS.iter().zip(&fields){
                if let Some(field)=field.upgrade(){
                    if !preset.palette[*key].is_null(){field.set_text(&field_text(&preset.palette[*key]));}
                }
            }
            if let Some(status)=status.upgrade(){status.set_text("Preset loaded into draft only. Review and explicitly save to apply.");}
        });
    }
    {
        let (state,status)=(state.clone(),status.downgrade());
        let fields:Vec<_>=fields.iter().map(|e|e.downgrade()).collect();
        reset.connect_clicked(move|_|{
            let baseline=state.borrow();let Some(review)=baseline.as_ref()else{return;};
            for ((key,_,_,_),field) in APPEARANCE_FIELDS.iter().zip(&fields){
                if let Some(field)=field.upgrade(){field.set_text(&field_text(&review.palette[*key]));}
            }
            if let Some(status)=status.upgrade(){status.set_text("Draft reset to the loaded baseline. No settings changed.");}
        });
    }
    let invalidate={
        let (state,editor,approval,preview)=(state.clone(),editor.downgrade(),approval.downgrade(),preview.downgrade());
        Rc::new(move||{state.borrow_mut().take();if let Some(e)=editor.upgrade(){e.set_sensitive(false);}if let Some(a)=approval.upgrade(){a.set_text("");}if let Some(p)=preview.upgrade(){p.queue_draw();}})
    };
    {let (invalidate,trust)=(invalidate.clone(),trust.downgrade());path.connect_changed(move|_|{if let Some(t)=trust.upgrade(){t.set_active(false);}invalidate();});}
    {let invalidate=invalidate.clone();target.connect_changed(move|_|invalidate());}
    {let invalidate=invalidate.clone();trust.connect_toggled(move|_|invalidate());}
    {
        let (form,path,trust,target,status,editor,preview)=(form.downgrade(),path.downgrade(),trust.downgrade(),target.downgrade(),status.downgrade(),editor.downgrade(),preview.downgrade());
        let presets=presets.downgrade();
        let fields:Vec<_>=fields.iter().map(|e|e.downgrade()).collect();let state=state.clone();
        load.connect_clicked(move|_|{
            let (Some(form_widget),Some(path),Some(trust),Some(target),Some(message))=(form.upgrade(),path.upgrade(),trust.upgrade(),target.upgrade(),status.upgrade())else{return;};
            if !trust.is_active(){message.set_text("Confirm checkout trust and offline state first.");return;}
            let Some(target)=target.active_id().map(|v|v.to_string())else{message.set_text("Select target compositor.");return;};
            let path=PathBuf::from(path.text().trim());state.borrow_mut().take();
            if let Some(e)=editor.upgrade(){e.set_sensitive(false);}
            form_widget.set_sensitive(false);message.set_text("Loading validated palette…");
            let (form,status,editor,preview,fields,state,presets)=(form.clone(),status.clone(),editor.clone(),preview.clone(),fields.clone(),state.clone(),presets.clone());
            background(move||rice::review_appearance(&path,&target),move|result|{
                let (Some(form),Some(status),Some(editor))=(form.upgrade(),status.upgrade(),editor.upgrade())else{return;};
                match result{
                    Ok(review)=>{
                        if let Some(presets)=presets.upgrade(){presets.remove_all();for p in &review.presets{presets.append(Some(&p.id),&p.name);}}
                        for ((key,_,_,_),field) in APPEARANCE_FIELDS.iter().zip(&fields){
                            if let Some(field)=field.upgrade(){field.set_text(&field_text(&review.palette[*key]));}
                        }
                        *state.borrow_mut()=Some(review);editor.set_sensitive(true);status.set_text("Draft ready. No settings changed.");
                        if let Some(p)=preview.upgrade(){p.queue_draw();}
                    }
                    Err(e)=>status.set_text(&format!("Load failed: {e:#}")),
                }
                form.set_sensitive(true);
            });
        });
    }
    {
        let (form,status,editor,approval)=(form.downgrade(),status.downgrade(),editor.downgrade(),approval.downgrade());
        let fields:Vec<_>=fields.iter().map(|e|e.downgrade()).collect();
        apply.connect_clicked(move|_|{
            let (Some(form_widget),Some(message),Some(approval_widget))=(form.upgrade(),status.upgrade(),approval.upgrade())else{return;};
            if approval_widget.text().as_str()!="apply appearance"{message.set_text("Type apply appearance to confirm this draft.");return;}
            let Some(values)=fields.iter().map(|e|e.upgrade().map(|e|e.text().to_string())).collect::<Option<Vec<_>>>()else{return;};
            {
                let baseline=state.borrow();
                let Some(review)=baseline.as_ref()else{return;};
                match review.changes(&values){Ok(changes) if changes.is_empty()=>{message.set_text("No appearance changes.");return;},Ok(_)=>{},Err(e)=>{message.set_text(&format!("{e:#}"));return;}}
            }
            let Some(review)=state.borrow_mut().take()else{return;};
            approval_widget.set_text("");form_widget.set_sensitive(false);message.set_text("Revalidating review and creating recovery snapshot…");
            let (form,status,editor)=(form.clone(),status.clone(),editor.clone());
            background(move||review.apply(values),move|result|{
                if let Some(status)=status.upgrade(){match result{Ok(text)=>status.set_text(&text),Err(e)=>status.set_text(&format!("Save failed: {e:#}. Load appearance again; inspect Rice backups if needed."))}}
                if let Some(editor)=editor.upgrade(){editor.set_sensitive(false);}
                if let Some(form)=form.upgrade(){form.set_sensitive(true);}
            });
        });
    }
    outer
}
