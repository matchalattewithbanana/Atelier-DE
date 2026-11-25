//! Static SDDM screenshot preview and explicit user-local preparation.
use gtk::{gdk,glib,prelude::*};
use std::{cell::RefCell,rc::Rc,path::PathBuf};
use crate::login_theme::{self,Review};
use super::{background,label};

pub(super) fn build()->gtk::Box{
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Login themes · SDDM","title-1"));
    outer.append(&label("Inspect an installed or local theme. Static screenshots only. QML is executable code; staging does not establish trust or readiness. No system changes.","dim-label"));
    let form=gtk::Box::new(gtk::Orientation::Vertical,8);outer.append(&form);
    let path=gtk::Entry::new();path.set_placeholder_text(Some("Absolute local theme directory"));form.append(&path);
    let installed=gtk::ComboBoxText::new();
    if let Ok(entries)=std::fs::read_dir("/usr/share/sddm/themes"){
        for e in entries.take(128).flatten(){
            if !e.file_type().is_ok_and(|kind|kind.is_dir()){continue;}
            if let Some(value)=e.path().to_str(){
                if !value.chars().any(char::is_control){installed.append(Some(value),value);}
            }
        }
    }
    {let path=path.downgrade();installed.connect_changed(move|p|{if let (Some(path),Some(id))=(path.upgrade(),p.active_id()){path.set_text(&id);}});}
    form.append(&installed);
    let review_button=gtk::Button::with_label("Inspect theme and screenshot");form.append(&review_button);
    let image=gtk::Picture::new();image.set_can_shrink(true);image.set_size_request(320,180);outer.append(&image);
    let detail=label("","body");detail.set_selectable(true);
    outer.append(&gtk::ScrolledWindow::builder().child(&detail).vexpand(true).min_content_height(110).build());
    let confirm=gtk::Entry::new();confirm.set_sensitive(false);outer.append(&confirm);
    let prepare=gtk::Button::with_label("Prepare reviewed copy");prepare.set_sensitive(false);outer.append(&prepare);
    let status=label("No changes made.","dim-label");status.set_selectable(true);outer.append(&status);
    let state=Rc::new(RefCell::new(None::<Review>));
    let invalidate={
        let (state,image,detail,confirm,prepare)=(state.clone(),image.downgrade(),detail.downgrade(),confirm.downgrade(),prepare.downgrade());
        Rc::new(move||{
            state.borrow_mut().take();
            if let Some(i)=image.upgrade(){i.set_paintable(None::<&gdk::Paintable>);}
            if let Some(d)=detail.upgrade(){d.set_text("");}
            if let Some(c)=confirm.upgrade(){c.set_text("");c.set_sensitive(false);c.set_placeholder_text(Some("Inspect first"));}
            if let Some(p)=prepare.upgrade(){p.set_sensitive(false);}
        })
    };
    {let invalidate=invalidate.clone();path.connect_changed(move|_|invalidate());}
    {
        let (form,path,image,detail,confirm,prepare,status)=(form.downgrade(),path.downgrade(),image.downgrade(),detail.downgrade(),confirm.downgrade(),prepare.downgrade(),status.downgrade());
        let state=state.clone();
        review_button.connect_clicked(move|_|{
            let (Some(f),Some(p),Some(s))=(form.upgrade(),path.upgrade(),status.upgrade())else{return;};
            let path=PathBuf::from(p.text().trim());invalidate();f.set_sensitive(false);s.set_text("Inspecting bounded local theme files…");
            let (form,image,detail,confirm,prepare,status,state)=(form.clone(),image.clone(),detail.clone(),confirm.clone(),prepare.clone(),status.clone(),state.clone());
            background(move||{let review=login_theme::inspect(&path)?;let text=review.detail()?;Ok((review,text))},move|result|{
                if let (Some(i),Some(d),Some(c),Some(p),Some(s))=(image.upgrade(),detail.upgrade(),confirm.upgrade(),prepare.upgrade(),status.upgrade()){
                    match result{
                        Ok((mut review,text))=>{
                            if let Some(frame)=review.preview.take(){
                                let bytes=glib::Bytes::from_owned(frame.rgba);
                                let texture=gdk::MemoryTexture::new(frame.width as i32,frame.height as i32,gdk::MemoryFormat::R8g8b8a8,&bytes,frame.width as usize*4);
                                i.set_paintable(Some(&texture));
                            }
                            d.set_text(&text);c.set_placeholder_text(Some(&format!("Type {}",review.phrase())));c.set_sensitive(true);p.set_sensitive(true);
                            s.set_text(&review.info.preview_status);*state.borrow_mut()=Some(review);
                        }
                        Err(e)=>s.set_text(&format!("Inspection failed: {e:#}")),
                    }
                }
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}
            });
        });
    }
    {
        let (form,confirm,status)=(form.downgrade(),confirm.downgrade(),status.downgrade());
        prepare.connect_clicked(move|button|{
            let (Some(f),Some(c),Some(s))=(form.upgrade(),confirm.upgrade(),status.upgrade())else{return;};
            let typed=c.text().to_string();
            if !state.borrow().as_ref().is_some_and(|r|r.phrase()==typed){s.set_text("Type the exact reviewed confirmation.");return;}
            let Some(review)=state.borrow_mut().take()else{return;};
            f.set_sensitive(false);button.set_sensitive(false);c.set_sensitive(false);c.set_text("");
            s.set_text("Rechecking source fingerprint and preparing inert copy…");
            let (form,status)=(form.clone(),status.clone());
            background(move||review.apply(&typed),move|result|{
                if let Some(s)=status.upgrade(){match result{Ok(path)=>s.set_text(&format!("Prepared {}. No login settings changed.",path.display())),Err(e)=>s.set_text(&format!("Preparation failed: {e:#}. Inspect again.",))}}
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}
            });
        });
    }
    outer
}
