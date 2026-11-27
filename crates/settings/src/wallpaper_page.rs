//! Local wallpaper catalog and ordered playlist editor. All decoded images
//! use the shared bounded raster loader; saves preserve unrelated preferences.
use atelier_config::{appearance_path,Appearance,AppearanceReview,WallpaperFit,WallpaperSlideshow,wallpaper};
use gtk::{glib,prelude::*};
use std::{cell::RefCell,rc::Rc,path::PathBuf,time::Duration};
use super::{description,heading};
fn texture(r:wallpaper::Raster)->gtk::gdk::MemoryTexture{
    gtk::gdk::MemoryTexture::new(r.width as i32,r.height as i32,gtk::gdk::MemoryFormat::R8g8b8a8,&glib::Bytes::from_owned(r.rgba),r.width as usize*4)
}
fn choose(button:&gtk::Button,entry:&gtk::Entry,folder:bool){
    let entry=entry.downgrade();
    button.connect_clicked(move|button|{
        let parent=button.root().and_downcast::<gtk::Window>();
        let dialog=gtk::FileChooserNative::new(Some(if folder{"Choose wallpaper folder"}else{"Choose wallpaper image"}),parent.as_ref(),
            if folder{gtk::FileChooserAction::SelectFolder}else{gtk::FileChooserAction::Open},Some("Choose"),Some("Cancel"));
        if !folder{let filter=gtk::FileFilter::new();filter.set_name(Some("Raster images"));for pattern in ["*.png","*.jpg","*.jpeg","*.webp","*.PNG","*.JPG","*.JPEG","*.WEBP"]{filter.add_pattern(pattern);}dialog.add_filter(&filter);}
        let entry=entry.clone();dialog.connect_response(move|dialog,response|{
            if response==gtk::ResponseType::Accept{
                if let (Some(entry),Some(path))=(entry.upgrade(),dialog.file().and_then(|f|f.path()).and_then(|p|p.into_os_string().into_string().ok())){entry.set_text(&path);}
            }
            dialog.destroy();
        });dialog.show();
    });
}
fn paths(view:&gtk::TextView)->Vec<String>{
    let buffer=view.buffer();buffer.text(&buffer.start_iter(),&buffer.end_iter(),true).lines()
        .filter(|line|!line.is_empty()).map(str::to_owned).collect()
}
fn preview_source(image:&str,slideshow:Option<&WallpaperSlideshow>)->anyhow::Result<Option<String>>{
    if let Some(list)=slideshow{list.validate()?;return Ok(list.images.first().cloned());}
    Ok((!image.is_empty()).then(||image.to_owned()))
}
fn content_fit(id:Option<&str>)->gtk::ContentFit{
    match id{Some("contain")=>gtk::ContentFit::Contain,Some("stretch")=>gtk::ContentFit::Fill,_=>gtk::ContentFit::Cover}
}
#[derive(Clone)]
struct Fields{
    image:gtk::Entry,fit:gtk::ComboBoxText,enabled:gtk::CheckButton,interval:gtk::SpinButton,playlist:gtk::TextView,blur:gtk::SpinButton,
}
impl Fields{
    fn populate(&self,a:&Appearance){
        self.image.set_text(&a.wallpaper_path);self.blur.set_value(a.wallpaper_blur as f64);
        self.fit.set_active_id(Some(match a.wallpaper_fit{WallpaperFit::Cover=>"cover",WallpaperFit::Contain=>"contain",WallpaperFit::Stretch=>"stretch"}));
        self.enabled.set_active(a.wallpaper_slideshow.is_some());
        self.interval.set_value(a.wallpaper_slideshow.as_ref().map_or(5,|s|s.interval_minutes) as f64);
        self.playlist.buffer().set_text(&a.wallpaper_slideshow.as_ref().map_or(String::new(),|s|s.images.join("\n")));
    }
    fn slideshow(&self)->Option<WallpaperSlideshow>{
        self.enabled.is_active().then(||WallpaperSlideshow{images:paths(&self.playlist),interval_minutes:self.interval.value_as_int() as u32})
    }
    fn collect(&self,a:&Appearance)->anyhow::Result<Appearance>{
        let mut next=a.clone();next.wallpaper_path=self.image.text().to_string();next.wallpaper_blur=self.blur.value_as_int() as u32;
        next.wallpaper_fit=match self.fit.active_id().as_deref(){Some("contain")=>WallpaperFit::Contain,Some("stretch")=>WallpaperFit::Stretch,_=>WallpaperFit::Cover};
        next.wallpaper_slideshow=self.slideshow();
        next.validate()?;Ok(next)
    }
}
/// Disable the form during a single worker operation; late results cannot
/// mutate a destroyed page. Return only data, never GTK objects, from workers.
fn work<T:Send+'static>(form:&gtk::Box,job:impl FnOnce()->anyhow::Result<T>+Send+'static,mut finish:impl FnMut(anyhow::Result<T>)+'static){
    form.set_sensitive(false);
    let (tx,rx)=std::sync::mpsc::sync_channel(1);std::thread::spawn(move||{let _=tx.send(job());});
    let form=form.downgrade();
    glib::timeout_add_local(Duration::from_millis(100),move||{
        let Some(form)=form.upgrade()else{return glib::ControlFlow::Break;};
        let result=match rx.try_recv(){Ok(result)=>result,Err(std::sync::mpsc::TryRecvError::Empty)=>return glib::ControlFlow::Continue,Err(_)=>Err(anyhow::anyhow!("Wallpaper worker stopped"))};
        finish(result);form.set_sensitive(true);glib::ControlFlow::Break
    });
}
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(p)=>p,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    let form=gtk::Box::new(gtk::Orientation::Vertical,10);parent.append(&form);
    form.append(&heading("Wallpaper library"));
    let folder=gtk::Entry::new();folder.set_placeholder_text(Some("Absolute local wallpaper folder"));form.append(&folder);
    let browse=gtk::Button::with_label("Choose folder");choose(&browse,&folder,true);form.append(&browse);
    let scan=gtk::Button::with_label("Browse folder");form.append(&scan);
    let gallery=gtk::ComboBoxText::new();gallery.set_hexpand(true);form.append(&gallery);
    let select=gtk::Button::with_label("Select image from library");form.append(&select);
    form.append(&heading("Image and fitting"));
    let image=gtk::Entry::new();image.set_placeholder_text(Some("Absolute image path; blank uses solid background"));form.append(&image);
    let browse=gtk::Button::with_label("Choose image directly");choose(&browse,&image,false);form.append(&browse);
    let fit=gtk::ComboBoxText::new();for (id,label) in [("cover","Fill (crop)"),("contain","Fit (letterbox)"),("stretch","Stretch")]{fit.append(Some(id),label);}fit.set_active_id(Some("cover"));form.append(&fit);
    form.append(&description("Static wallpaper blur (0 disables; not live window backdrop blur)"));let blur=gtk::SpinButton::with_range(0.0,24.0,1.0);form.append(&blur);
    let picture=gtk::Picture::new();picture.set_can_shrink(true);picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_overflow(gtk::Overflow::Hidden);
    let frame=gtk::AspectFrame::builder().ratio(16.0/9.0).obey_child(false).child(&picture).build();
    frame.set_size_request(240,180);frame.set_hexpand(true);form.append(&frame);
    let preview_note=description("Sample 16:9 viewport · Preview the static image or first slideshow item.");form.append(&preview_note);
    let preview=gtk::Button::with_label("Preview draft wallpaper");form.append(&preview);
    let clear=gtk::Button::with_label("Use solid background");form.append(&clear);
    form.append(&heading("Slideshow"));
    let enabled=gtk::CheckButton::with_label("Enable ordered slideshow");form.append(&enabled);
    form.append(&description("Interval in minutes (1–1440). Add selected images or edit the list, one absolute path per line, in playback order."));
    let interval=gtk::SpinButton::with_range(1.0,1440.0,1.0);interval.set_value(5.0);form.append(&interval);
    let playlist=gtk::TextView::new();playlist.set_monospace(true);playlist.set_wrap_mode(gtk::WrapMode::WordChar);
    form.append(&gtk::ScrolledWindow::builder().min_content_height(100).child(&playlist).build());
    let add=gtk::Button::with_label("Append selected image to slideshow");form.append(&add);
    let all=gtk::Button::with_label("Replace playlist with library (up to 32 images)");form.append(&all);
    let reload=gtk::Button::with_label("Reload saved wallpaper preferences");form.append(&reload);
    let previous=gtk::Button::with_label("Load previous wallpaper preferences into draft");form.append(&previous);
    let save=gtk::Button::with_label("Save wallpaper preferences");save.add_css_class("suggested-action");save.set_sensitive(false);form.append(&save);
    let status=description("Local images remain in their original location. Changes apply after saving.");status.set_selectable(true);parent.append(&status);
    let fields=Fields{image:image.clone(),fit,enabled,interval,playlist:playlist.clone(),blur:blur.clone()};
    let baseline=Rc::new(RefCell::new(None::<AppearanceReview>));let catalog=Rc::new(RefCell::new(Vec::<String>::new()));
    {let (gallery,catalog)=(gallery.downgrade(),catalog.clone());folder.connect_changed(move|_|{catalog.borrow_mut().clear();if let Some(g)=gallery.upgrade(){g.remove_all();}});}
    {let (form,folder,gallery,status,catalog)=(form.downgrade(),folder.downgrade(),gallery.downgrade(),status.downgrade(),catalog.clone());scan.connect_clicked(move|_|{
        let (Some(f),Some(folder),Some(g),Some(s))=(form.upgrade(),folder.upgrade(),gallery.upgrade(),status.upgrade())else{return;};
        let path=PathBuf::from(folder.text().as_str());g.remove_all();catalog.borrow_mut().clear();s.set_text("Scanning local folder…");
        let (gallery,status,catalog)=(gallery.clone(),status.clone(),catalog.clone());
        work(&f,move||wallpaper::gallery(&path),move|result|{
            if let (Some(g),Some(s))=(gallery.upgrade(),status.upgrade()){
                match result{Ok(items)=>{
                    for (i,path) in items.iter().enumerate(){let name=std::path::Path::new(path).file_name().and_then(|s|s.to_str()).unwrap_or(path);g.append(Some(&i.to_string()),name);}
                    if !items.is_empty(){g.set_active(Some(0));}
                    s.set_text(&format!("{} candidate images. Select one and preview before saving.",items.len()));*catalog.borrow_mut()=items;
                },Err(e)=>s.set_text(&format!("Scan failed: {e:#}"))}
            }
        });
    });}
    {let (gallery,image,catalog)=(gallery.downgrade(),image.downgrade(),catalog.clone());select.connect_clicked(move|_|{
        if let (Some(g),Some(i))=(gallery.upgrade(),image.upgrade()){if let Some(index)=g.active(){if let Some(path)=catalog.borrow().get(index as usize){i.set_text(path);}}}
    });}
    {let (image,playlist,status)=(image.downgrade(),playlist.downgrade(),status.downgrade());add.connect_clicked(move|_|{
        let (Some(i),Some(p),Some(s))=(image.upgrade(),playlist.upgrade(),status.upgrade())else{return;};
        let path=i.text().to_string();let mut items=paths(&p);
        if path.is_empty()||items.contains(&path)||items.len()>=32{s.set_text("Select a new image; playlist holds up to 32 unique paths.");return;}
        items.push(path);p.buffer().set_text(&items.join("\n"));s.set_text("Added to draft playlist.");
    });}
    {let (playlist,status,catalog)=(playlist.downgrade(),status.downgrade(),catalog.clone());all.connect_clicked(move|_|{
        let (Some(p),Some(s))=(playlist.upgrade(),status.upgrade())else{return;};let items=catalog.borrow();
        if !(2..=32).contains(&items.len()){s.set_text("Use a library with 2–32 images, or append individual selections.");return;}
        p.buffer().set_text(&items.join("\n"));s.set_text("Library copied to draft playlist in sorted order.");
    });}
    {let fields=fields.clone();clear.connect_clicked(move|_|{fields.image.set_text("");fields.enabled.set_active(false);});}
    // Fit reuses decoded pixels. Source changes invalidate the old raster.
    {let picture=picture.downgrade();fields.fit.connect_changed(move|fit|{if let Some(p)=picture.upgrade(){p.set_content_fit(content_fit(fit.active_id().as_deref()));}});}
    let invalidate={let (picture,note)=(picture.downgrade(),preview_note.downgrade());move||{
        if let Some(p)=picture.upgrade(){p.set_paintable(None::<&gtk::gdk::Paintable>);}
        if let Some(n)=note.upgrade(){n.set_text("Draft changed · Preview again to decode the current wallpaper.");}
    }};
    {let f=invalidate.clone();image.connect_changed(move|_|f());}
    {let f=invalidate.clone();blur.connect_value_changed(move|_|f());}
    {let f=invalidate.clone();fields.enabled.connect_toggled(move|_|f());}
    {let f=invalidate.clone();fields.playlist.buffer().connect_changed(move|_|f());}
    {let f=invalidate.clone();fields.interval.connect_value_changed(move|_|f());}
    {let (baseline,fields,status,save,invalidate)=(baseline.clone(),fields.clone(),status.clone(),save.downgrade(),invalidate.clone());
    reload.connect_clicked(move|_|{invalidate();match AppearanceReview::load(&path){
        Ok(r)=>{fields.populate(&r.value);*baseline.borrow_mut()=Some(r);if let Some(s)=save.upgrade(){s.set_sensitive(true);}status.set_text("Saved wallpaper preferences loaded.");}
        Err(e)=>{baseline.borrow_mut().take();if let Some(s)=save.upgrade(){s.set_sensitive(false);}status.set_text(&format!("Cannot load appearance: {e:#}"));}
    }});}
    {let (baseline,fields,status)=(baseline.clone(),fields.clone(),status.clone());previous.connect_clicked(move|_|{
        let state=baseline.borrow();let Some(r)=state.as_ref()else{return;};
        match r.previous(){Ok(a)=>{fields.populate(&a);status.set_text("Previous wallpaper and slideshow preferences loaded into draft.");}Err(e)=>status.set_text(&format!("{e:#}"))}
    });}
    {let (form,fields,picture,status,note,invalidate)=(form.downgrade(),fields.clone(),picture.downgrade(),status.downgrade(),preview_note.downgrade(),invalidate.clone());preview.connect_clicked(move|_|{
        let (Some(f),Some(s))=(form.upgrade(),status.upgrade())else{return;};
        invalidate();
        let slideshow=fields.slideshow();
        let path=match preview_source(fields.image.text().as_str(),slideshow.as_ref()){
            Ok(Some(path))=>PathBuf::from(path),
            Ok(None)=>{if let Some(n)=note.upgrade(){n.set_text("Solid background selected · No image to decode.");}s.set_text("Preview only; desktop unchanged.");return;}
            Err(e)=>{s.set_text(&format!("Preview rejected: {e:#}"));return;}
        };
        let strength=fields.blur.value_as_int() as u32;s.set_text("Decoding preview…");
        let (picture,status,note)=(picture.clone(),status.clone(),note.clone());
        let is_slideshow=slideshow.is_some();
        work(&f,move||wallpaper::decode_background(&path,strength),move|result|{
            if let (Some(p),Some(s),Some(n))=(picture.upgrade(),status.upgrade(),note.upgrade()){match result{
                Ok(r)=>{p.set_paintable(Some(&texture(r)));n.set_text(if is_slideshow{"First slideshow item · Sample 16:9 viewport"}else{"Static image · Sample 16:9 viewport"});s.set_text("Preview only; desktop unchanged.");}
                Err(e)=>{n.set_text("No valid preview.");s.set_text(&format!("Preview rejected: {e:#}"));}
            }}
        });
    });}
    {let (form,fields,picture,status,baseline,note)=(form.downgrade(),fields.clone(),picture.downgrade(),status.downgrade(),baseline.clone(),preview_note.downgrade());save.connect_clicked(move|_|{
        let (Some(f),Some(s))=(form.upgrade(),status.upgrade())else{return;};
        let next={let state=baseline.borrow();let Some(r)=state.as_ref()else{return;};match fields.collect(&r.value){Ok(a)=>a,Err(e)=>{s.set_text(&format!("{e:#}"));return;}}};
        s.set_text("Validating wallpaper preferences…");
        let (picture,status,baseline,note)=(picture.clone(),status.clone(),baseline.clone(),note.clone());
        work(&f,move||{
            if let Some(playlist)=&next.wallpaper_slideshow{
                for path in &playlist.images{wallpaper::validate_candidate(std::path::Path::new(path))?;}
            }
            let first=preview_source(&next.wallpaper_path,next.wallpaper_slideshow.as_ref())?;
            let raster=first.map(|p|wallpaper::decode_background(std::path::Path::new(&p),next.wallpaper_blur)).transpose()?;
            Ok((next,raster))
        },move|result|{
            if let Some(s)=status.upgrade(){match result{
                Ok((next,raster))=>{
                    if let Some(p)=picture.upgrade(){if let Some(r)=raster{p.set_paintable(Some(&texture(r)));}else{p.set_paintable(None::<&gtk::gdk::Paintable>);}}
                    if let Some(n)=note.upgrade(){n.set_text(if next.wallpaper_slideshow.is_some(){"First slideshow item · Sample 16:9 viewport"}else if next.wallpaper_path.is_empty(){"Solid background selected · No image to decode."}else{"Static image · Sample 16:9 viewport"});}
                    if let Some(r)=baseline.borrow_mut().as_mut(){match r.save(next){
                        Ok(true)=>s.set_text("Wallpaper preferences saved. Updated native shell applies them live."),
                        Ok(false)=>s.set_text("No changes."),
                        Err(e)=>s.set_text(&format!("Save failed: {e:#}. Reload before retrying.")),
                    }}
                }
                Err(e)=>s.set_text(&format!("Wallpaper rejected: {e:#}")),
            }}
        });
    });}
    reload.emit_clicked();
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn slideshow_preview_uses_first_item_not_static_path(){
        let list=WallpaperSlideshow{images:vec!["/a.png".into(),"/b.png".into()],interval_minutes:5};
        assert_eq!(preview_source("/static.png",Some(&list)).unwrap().as_deref(),Some("/a.png"));
        assert_eq!(preview_source("/static.png",None).unwrap().as_deref(),Some("/static.png"));
        assert_eq!(preview_source("",None).unwrap(),None);
        let bad=WallpaperSlideshow{images:vec!["/a.png".into()],interval_minutes:5};
        assert!(preview_source("/static.png",Some(&bad)).is_err());
    }
    #[test]
    fn wallpaper_fit_matches_draft_choice(){
        assert_eq!(content_fit(Some("cover")),gtk::ContentFit::Cover);
        assert_eq!(content_fit(Some("contain")),gtk::ContentFit::Contain);
        assert_eq!(content_fit(Some("stretch")),gtk::ContentFit::Fill);
        assert_eq!(content_fit(None),gtk::ContentFit::Cover);
    }
}
