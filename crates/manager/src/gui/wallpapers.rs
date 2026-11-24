//! Classic wallpaper library. Saves only through the reviewed Mango backend.
use gtk::{prelude::*,gdk_pixbuf};
use gtk::gdk_pixbuf::prelude::*;
use std::{cell::RefCell,rc::Rc,path::PathBuf};
use crate::rice::{self,WallpaperReview,WallpaperMode};
use super::{background,label};

fn entry(form:&gtk::Box,hint:&str)->gtk::Entry{
    let entry=gtk::Entry::new();entry.set_placeholder_text(Some(hint));form.append(&entry);entry
}
pub(super) fn build()->gtk::Box{
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Wallpapers","title-1"));
    outer.append(&label("Atelier Classic · Per-output local images and five-minute slideshows. Close all Mango Atelier sessions before saving. Original images are referenced, not copied. Previews require python-pillow.","dim-label"));
    let form=gtk::Box::new(gtk::Orientation::Vertical,8);
    outer.append(&gtk::ScrolledWindow::builder().child(&form).vexpand(true).build());
    let path=entry(&form,"Absolute trusted Mango Atelier checkout");
    let trust=gtk::CheckButton::with_label("I trust this checkout and have closed all Atelier sessions.");form.append(&trust);
    let target=gtk::ComboBoxText::new();
    for (id,name) in [("mango","MangoWM"),("hyprland","Hyprland"),("niri","Niri")]{target.append(Some(id),name);}
    form.append(&target);
    let load=gtk::Button::with_label("Load wallpaper settings");form.append(&load);
    let editor=gtk::Box::new(gtk::Orientation::Vertical,8);editor.set_sensitive(false);form.append(&editor);
    let current=label("","dim-label");current.set_selectable(true);editor.append(&current);
    let folder=entry(&editor,"Absolute wallpaper library folder");
    let browse=gtk::Button::with_label("Browse local folder");editor.append(&browse);
    let gallery=gtk::ComboBoxText::new();editor.append(&gallery);
    let image=entry(&editor,"Absolute image path (or choose from library)");
    let preview=gtk::Button::with_label("Preview image");editor.append(&preview);
    let picture=gtk::Picture::new();picture.set_can_shrink(true);picture.set_size_request(240,160);editor.append(&picture);
    let screen=entry(&editor,"Exact output name, e.g. DP-1");
    let mode=gtk::ComboBoxText::new();
    for (id,name) in [("image","Static image"),("slideshow","Slideshow from library folder"),("clear","Clear image and slideshow")]{mode.append(Some(id),name);}
    mode.set_active_id(Some("image"));editor.append(&mode);
    let confirmation=entry(&editor,"Type set wallpaper OUTPUT");
    let save=gtk::Button::with_label("Save reviewed wallpaper");save.add_css_class("suggested-action");editor.append(&save);
    let status=label("Load settings first.","dim-label");status.set_selectable(true);outer.append(&status);
    let state:Rc<RefCell<Option<WallpaperReview>>>=Rc::new(RefCell::new(None));
    let invalidate={
        let (state,editor,confirmation,gallery,picture)=(state.clone(),editor.downgrade(),confirmation.downgrade(),gallery.downgrade(),picture.downgrade());
        Rc::new(move||{
            state.borrow_mut().take();
            if let Some(w)=editor.upgrade(){w.set_sensitive(false);}
            if let Some(w)=confirmation.upgrade(){w.set_text("");}
            if let Some(w)=gallery.upgrade(){w.remove_all();}
            if let Some(w)=picture.upgrade(){w.set_paintable(None::<&gtk::gdk::Paintable>);}
        })
    };
    {let (trust,invalidate)=(trust.downgrade(),invalidate.clone());path.connect_changed(move|_|{if let Some(t)=trust.upgrade(){t.set_active(false);}invalidate();});}
    {let invalidate=invalidate.clone();trust.connect_toggled(move|_|invalidate());}
    {let invalidate=invalidate.clone();target.connect_changed(move|_|invalidate());}
    for field in [&folder,&image,&screen]{
        let confirmation=confirmation.downgrade();
        field.connect_changed(move|_|{if let Some(c)=confirmation.upgrade(){c.set_text("");}});
    }
    {let c=confirmation.downgrade();mode.connect_changed(move|_|{if let Some(c)=c.upgrade(){c.set_text("");}});}
    {let p=picture.downgrade();image.connect_changed(move|_|{if let Some(p)=p.upgrade(){p.set_paintable(None::<&gtk::gdk::Paintable>);}});}
    {let gallery=gallery.downgrade();folder.connect_changed(move|_|{if let Some(g)=gallery.upgrade(){g.remove_all();}});}
    {let image=image.downgrade();gallery.connect_changed(move|g|{if let (Some(i),Some(id))=(image.upgrade(),g.active_id()){i.set_text(&id);}});}
    {
        let (form,path,trust,target,editor,current,status)=(form.downgrade(),path.downgrade(),trust.downgrade(),target.downgrade(),editor.downgrade(),current.downgrade(),status.downgrade());
        let state=state.clone();
        load.connect_clicked(move|_|{
            let (Some(f),Some(p),Some(t),Some(target),Some(s))=(form.upgrade(),path.upgrade(),trust.upgrade(),target.upgrade(),status.upgrade())else{return;};
            if !t.is_active(){s.set_text("Confirm trusted checkout and offline state.");return;}
            let Some(target)=target.active_id().map(|v|v.to_string())else{s.set_text("Choose a compositor target.");return;};
            let p=PathBuf::from(p.text().trim());
            state.borrow_mut().take();if let Some(e)=editor.upgrade(){e.set_sensitive(false);}
            f.set_sensitive(false);s.set_text("Reading reviewed wallpaper settings…");
            let (form,editor,current,status,state)=(form.clone(),editor.clone(),current.clone(),status.clone(),state.clone());
            background(move||rice::review_wallpaper(&p,&target),move|result|{
                if let (Some(s),Some(e),Some(current))=(status.upgrade(),editor.upgrade(),current.upgrade()){
                    match result{
                        Ok(review)=>{
                            let mut text=String::from("Saved output preferences:\n");
                            for (name,path) in &review.state.wallpapers{text.push_str(&format!("{name}: {path}\n"));}
                            for (name,path) in &review.state.slideshows{text.push_str(&format!("{name} slideshow: {path}\n"));}
                            current.set_text(&text);*state.borrow_mut()=Some(review);e.set_sensitive(true);s.set_text("Settings loaded. No changes made.");
                        }
                        Err(error)=>{current.set_text("");s.set_text(&format!("Load failed: {error:#}"));}
                    }
                }
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}
            });
        });
    }
    {
        let (form,folder,gallery,status)=(form.downgrade(),folder.downgrade(),gallery.downgrade(),status.downgrade());
        let state=state.clone();
        browse.connect_clicked(move|_|{
            let (Some(f),Some(folder),Some(s))=(form.upgrade(),folder.upgrade(),status.upgrade())else{return;};
            let Some(review)=state.borrow().clone()else{return;};let folder=folder.text().trim().to_owned();
            f.set_sensitive(false);s.set_text("Scanning local folder…");
            let (form,gallery,status)=(form.clone(),gallery.clone(),status.clone());
            background(move||review.gallery(&folder),move|result|{
                if let (Some(g),Some(s))=(gallery.upgrade(),status.upgrade()){
                    g.remove_all();
                    match result{Ok(items)=>{s.set_text(&format!("{} image candidates. Select one and preview.",items.len()));for item in items{g.append(Some(&item.path),&item.name);}},Err(e)=>s.set_text(&format!("Gallery failed: {e:#}"))}
                }
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}
            });
        });
    }
    {
        let (form,image,picture,status)=(form.downgrade(),image.downgrade(),picture.downgrade(),status.downgrade());
        let state=state.clone();
        preview.connect_clicked(move|_|{
            let (Some(f),Some(i),Some(s))=(form.upgrade(),image.upgrade(),status.upgrade())else{return;};
            let Some(review)=state.borrow().clone()else{return;};let image=i.text().trim().to_owned();
            f.set_sensitive(false);s.set_text("Decoding bounded thumbnail…");
            let (form,picture,status)=(form.clone(),picture.clone(),status.clone());
            background(move||review.thumbnail(&image),move|result|{
                if let (Some(p),Some(s))=(picture.upgrade(),status.upgrade()){
                    p.set_paintable(None::<&gtk::gdk::Paintable>);
                    let result=result.and_then(|bytes|{
                        let loader=gdk_pixbuf::PixbufLoader::with_type("png")?;
                        loader.write(&bytes)?;loader.close()?;
                        loader.pixbuf().ok_or_else(||anyhow::anyhow!("No decoded thumbnail"))
                    });
                    match result{Ok(pixbuf)=>{p.set_pixbuf(Some(&pixbuf));s.set_text("Local preview only. No settings changed.");},Err(e)=>s.set_text(&format!("Preview failed: {e:#}"))}
                }
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}
            });
        });
    }
    {
        let (form,editor,screen,mode,image,folder,confirmation,status)=(form.downgrade(),editor.downgrade(),screen.downgrade(),mode.downgrade(),image.downgrade(),folder.downgrade(),confirmation.downgrade(),status.downgrade());
        save.connect_clicked(move|_|{
            let (Some(f),Some(screen),Some(mode),Some(image),Some(folder),Some(c),Some(s))=(form.upgrade(),screen.upgrade(),mode.upgrade(),image.upgrade(),folder.upgrade(),confirmation.upgrade(),status.upgrade())else{return;};
            let screen=screen.text().trim().to_owned();
            if screen.is_empty() || c.text().as_str()!=format!("set wallpaper {screen}"){s.set_text("Type set wallpaper followed by the exact output name.");return;}
            let (mode,value)=match mode.active_id().as_deref(){
                Some("image")=>(WallpaperMode::Image,image.text().trim().to_owned()),
                Some("slideshow")=>(WallpaperMode::Slideshow,folder.text().trim().to_owned()),
                Some("clear")=>(WallpaperMode::Clear,String::new()),
                _=>return,
            };
            let Some(review)=state.borrow_mut().take()else{return;};
            c.set_text("");f.set_sensitive(false);s.set_text("Checking fingerprint and saving recovery snapshot…");
            let (form,editor,status)=(form.clone(),editor.clone(),status.clone());
            background(move||review.apply(mode,&screen,&value),move|result|{
                if let Some(s)=status.upgrade(){match result{Ok(text)=>s.set_text(&text),Err(e)=>s.set_text(&format!("Save failed: {e:#}. Reload settings before retrying."))}}
                if let Some(e)=editor.upgrade(){e.set_sensitive(false);}
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}
            });
        });
    }
    outer
}
