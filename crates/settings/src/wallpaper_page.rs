//! Native wallpaper drafts and validation on a worker, with reviewed persistence.
use atelier_config::{appearance_path,AppearanceReview,WallpaperFit,wallpaper};
use gtk::{glib,prelude::*};
use std::{cell::RefCell,rc::Rc,path::PathBuf,time::Duration};
use super::description;
fn texture(r:wallpaper::Raster)->gtk::gdk::MemoryTexture{
    gtk::gdk::MemoryTexture::new(r.width as i32,r.height as i32,gtk::gdk::MemoryFormat::R8g8b8a8,&glib::Bytes::from_owned(r.rgba),r.width as usize*4)
}
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(p)=>p,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    let form=gtk::Box::new(gtk::Orientation::Vertical,10);parent.append(&form);
    let image=gtk::Entry::new();image.set_placeholder_text(Some("Absolute PNG, JPEG or WebP path; blank disables wallpaper"));form.append(&image);
    let choose=gtk::Button::with_label("Choose local image");form.append(&choose);
    let fit=gtk::ComboBoxText::new();for (id,label) in [("cover","Fill (crop)"),("contain","Fit (letterbox)"),("stretch","Stretch")]{fit.append(Some(id),label);}fit.set_active_id(Some("cover"));form.append(&fit);
    let picture=gtk::Picture::new();picture.set_can_shrink(true);picture.set_content_fit(gtk::ContentFit::Contain);picture.set_size_request(240,160);form.append(&picture);
    let preview=gtk::Button::with_label("Preview image");form.append(&preview);
    let clear=gtk::Button::with_label("Use solid background");form.append(&clear);
    let reload=gtk::Button::with_label("Reload saved appearance");form.append(&reload);
    let previous=gtk::Button::with_label("Load previous wallpaper into draft");form.append(&previous);
    let save=gtk::Button::with_label("Save wallpaper");save.add_css_class("suggested-action");form.append(&save);
    let status=description("Images remain in their original location. Native panel mode only.");status.set_selectable(true);parent.append(&status);
    let baseline=Rc::new(RefCell::new(None::<AppearanceReview>));
    {let image=image.downgrade();choose.connect_clicked(move|button|{
        let parent=button.root().and_downcast::<gtk::Window>();
        let dialog=gtk::FileChooserNative::new(Some("Choose wallpaper"),parent.as_ref(),gtk::FileChooserAction::Open,Some("Choose"),Some("Cancel"));
        let filter=gtk::FileFilter::new();filter.set_name(Some("Raster images"));for pattern in ["*.png","*.jpg","*.jpeg","*.webp","*.PNG","*.JPG","*.JPEG","*.WEBP"]{filter.add_pattern(pattern);}dialog.add_filter(&filter);
        let image=image.clone();dialog.connect_response(move|dialog,response|{
            if response==gtk::ResponseType::Accept{
                if let (Some(entry),Some(path))=(image.upgrade(),dialog.file().and_then(|f|f.path()).and_then(|p|p.into_os_string().into_string().ok())){entry.set_text(&path);}
            }
            dialog.destroy();
        });dialog.show();
    });}
    {let image=image.downgrade();clear.connect_clicked(move|_|{if let Some(image)=image.upgrade(){image.set_text("");}});}
    {let picture=picture.downgrade();image.connect_changed(move|_|{if let Some(p)=picture.upgrade(){p.set_paintable(None::<&gtk::gdk::Paintable>);}});}
    {
        let (baseline,image,fit,status,save)=(baseline.clone(),image.clone(),fit.clone(),status.clone(),save.downgrade());
        reload.connect_clicked(move|_|match AppearanceReview::load(&path){
            Ok(r)=>{image.set_text(&r.value.wallpaper_path);fit.set_active_id(Some(match r.value.wallpaper_fit{WallpaperFit::Cover=>"cover",WallpaperFit::Contain=>"contain",WallpaperFit::Stretch=>"stretch"}));*baseline.borrow_mut()=Some(r);if let Some(s)=save.upgrade(){s.set_sensitive(true);}status.set_text("Saved wallpaper loaded. Preview or choose another image.");}
            Err(e)=>{baseline.borrow_mut().take();if let Some(s)=save.upgrade(){s.set_sensitive(false);}status.set_text(&format!("Cannot load appearance: {e:#}"));}
        });
    }
    {
        let (baseline,image,fit,status)=(baseline.clone(),image.downgrade(),fit.downgrade(),status.downgrade());
        previous.connect_clicked(move|_|{
            let (Some(image),Some(fit),Some(status))=(image.upgrade(),fit.upgrade(),status.upgrade())else{return;};
            let state=baseline.borrow();let Some(review)=state.as_ref()else{status.set_text("Reload saved appearance first.");return;};
            match review.previous(){
                Ok(value)=>{image.set_text(&value.wallpaper_path);fit.set_active_id(Some(match value.wallpaper_fit{WallpaperFit::Cover=>"cover",WallpaperFit::Contain=>"contain",WallpaperFit::Stretch=>"stretch"}));status.set_text("Previous wallpaper in draft only. Save to restore wallpaper without changing other appearance preferences.");}
                Err(error)=>status.set_text(&format!("Previous appearance unavailable: {error:#}")),
            }
        });
    }
    for (button,commit) in [(&preview,false),(&save,true)]{
        let (form,image,fit,picture,status,baseline)=(form.downgrade(),image.downgrade(),fit.downgrade(),picture.downgrade(),status.downgrade(),baseline.clone());
        button.connect_clicked(move|_|{
            let (Some(form_widget),Some(image),Some(fit),Some(status_widget))=(form.upgrade(),image.upgrade(),fit.upgrade(),status.upgrade())else{return;};
            let path=image.text().to_string();
            let mode=match fit.active_id().as_deref(){Some("contain")=>WallpaperFit::Contain,Some("stretch")=>WallpaperFit::Stretch,_=>WallpaperFit::Cover};
            let next=if commit{
                let state=baseline.borrow();let Some(r)=state.as_ref()else{return;};
                let mut next=r.value.clone();next.wallpaper_path=path.clone();next.wallpaper_fit=mode;
                if let Err(e)=next.validate(){status_widget.set_text(&format!("{e:#}"));return;}
                Some(next)
            }else{None};
            form_widget.set_sensitive(false);status_widget.set_text("Validating local image…");
            let (tx,rx)=std::sync::mpsc::sync_channel(1);
            std::thread::spawn(move||{let result=if path.is_empty(){Ok(None)}else{wallpaper::decode(&PathBuf::from(path),480).map(Some)};let _=tx.send(result);});
            let (form,picture,status,baseline)=(form.clone(),picture.clone(),status.clone(),baseline.clone());
            glib::timeout_add_local(Duration::from_millis(50),move||{
                let result=match rx.try_recv(){Ok(r)=>r,Err(std::sync::mpsc::TryRecvError::Empty)=>return glib::ControlFlow::Continue,Err(_)=>{if let Some(s)=status.upgrade(){s.set_text("Image worker stopped.");}if let Some(f)=form.upgrade(){f.set_sensitive(true);}return glib::ControlFlow::Break;}};
                if let (Some(picture),Some(status))=(picture.upgrade(),status.upgrade()){
                    match result{
                        Ok(raster)=>{
                            if let Some(r)=raster{picture.set_paintable(Some(&texture(r)));}else{picture.set_paintable(None::<&gtk::gdk::Paintable>);}
                            if let Some(next)=next.clone(){
                                let mut baseline=baseline.borrow_mut();
                                if let Some(r)=baseline.as_mut(){match r.save(next){
                                    Ok(true)=>status.set_text("Wallpaper saved; previous appearance retained. Updated native shell applies wallpaper live."),
                                    Ok(false)=>status.set_text("No changes."),
                                    Err(e)=>status.set_text(&format!("Save failed: {e:#}. Reload before retrying.")),
                                }}
                            }else{status.set_text("Preview only; no settings changed.");}
                        }
                        Err(e)=>status.set_text(&format!("Wallpaper rejected: {e:#}")),
                    }
                }
                if let Some(f)=form.upgrade(){f.set_sensitive(true);}glib::ControlFlow::Break
            });
        });
    }
    reload.emit_clicked();
}
