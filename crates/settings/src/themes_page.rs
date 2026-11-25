//! Draft-only portable theme studio. Apply is a reviewed native profile save.
use atelier_config::{appearance_path,AppearanceReview,themes::{PortableTheme,from_seed},wallpaper};
use gtk::{glib,prelude::*};
use std::{cell::RefCell,rc::Rc,path::PathBuf,time::Duration};
use super::description;
fn text(view:&gtk::TextView)->String{
    let buffer=view.buffer();buffer.text(&buffer.start_iter(),&buffer.end_iter(),true).to_string()
}
fn populate(view:&gtk::TextView,theme:&PortableTheme)->anyhow::Result<()>{view.buffer().set_text(&theme.json()?);Ok(())}
fn preview(theme:&PortableTheme,label:&gtk::Label){
    let mut colors=vec![("Accent",theme.accent.as_str()),("Focused",theme.window_border.focused.as_str()),("Unfocused",theme.window_border.unfocused.as_str())];
    if let Some(p)=&theme.shell_palette{colors.extend([("Surface",p.surface.as_str()),("Elevated",p.elevated.as_str()),("Text",p.text.as_str()),("Muted",p.muted.as_str())]);}
    let markup=colors.into_iter().filter(|(_,color)|!color.is_empty()).map(|(name,color)|format!("<span foreground=\"{color}\">■■■</span> {name} {color}")).collect::<Vec<_>>().join("\n");
    label.set_markup(&markup);
}
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(p)=>p,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    let form=gtk::Box::new(gtk::Orientation::Vertical,10);parent.append(&form);
    let image=gtk::Entry::new();image.set_placeholder_text(Some("Absolute local image path"));form.append(&image);
    let variant=gtk::ComboBoxText::new();variant.append(Some("dark"),"Dark palette");variant.append(Some("light"),"Light palette");variant.set_active_id(Some("dark"));form.append(&variant);
    let generate=gtk::Button::with_label("Generate wallpaper palette into draft");form.append(&generate);
    let file=gtk::Entry::new();file.set_placeholder_text(Some("Absolute .json theme import/export path"));form.append(&file);
    let row=gtk::Box::new(gtk::Orientation::Horizontal,6);let import=gtk::Button::with_label("Import draft");let export=gtk::Button::with_label("Export draft (new file)");row.append(&import);row.append(&export);form.append(&row);
    let draft=gtk::TextView::new();draft.set_monospace(true);draft.set_wrap_mode(gtk::WrapMode::WordChar);
    form.append(&gtk::ScrolledWindow::builder().min_content_height(220).child(&draft).build());
    let swatches=description("");form.append(&swatches);
    let check=gtk::Button::with_label("Validate and preview draft");form.append(&check);
    let load=gtk::Button::with_label("Load saved visual preferences");form.append(&load);
    let previous=gtk::Button::with_label("Load previous visual preferences");form.append(&previous);
    let apply=gtk::Button::with_label("Apply theme");apply.add_css_class("suggested-action");apply.set_sensitive(false);form.append(&apply);
    let status=description("Themes exclude wallpaper paths, application pins and commands.");status.set_selectable(true);parent.append(&status);
    let state=Rc::new(RefCell::new(None::<AppearanceReview>));
    {let (swatches,apply)=(swatches.downgrade(),apply.downgrade());draft.buffer().connect_changed(move|_|{if let Some(s)=swatches.upgrade(){s.set_text("");}if let Some(a)=apply.upgrade(){a.set_sensitive(false);}});}
    {let (draft,image,status,state)=(draft.clone(),image.clone(),status.clone(),state.clone());
    load.connect_clicked(move|_|match AppearanceReview::load(&path){
        Ok(review)=>{image.set_text(&review.value.wallpaper_path);let result=populate(&draft,&PortableTheme::capture("Current native theme".into(),&review.value));*state.borrow_mut()=Some(review);status.set_text(&match result{Ok(())=>"Current visual preferences loaded into draft.".into(),Err(e)=>format!("{e:#}")});}
        Err(e)=>{state.borrow_mut().take();status.set_text(&format!("Cannot load profile: {e:#}"));}
    });}
    {let (draft,status,state)=(draft.clone(),status.clone(),state.clone());previous.connect_clicked(move|_|{
        let state=state.borrow();let Some(review)=state.as_ref()else{return;};
        let result=review.previous().and_then(|a|populate(&draft,&PortableTheme::capture("Previous native theme".into(),&a)));
        status.set_text(&match result{Ok(())=>"Previous visual preferences in draft only.".into(),Err(e)=>format!("{e:#}")});
    });}
    {let (draft,file,status)=(draft.clone(),file.clone(),status.clone());import.connect_clicked(move|_|{
        let result=PortableTheme::import(&PathBuf::from(file.text().as_str())).and_then(|theme|populate(&draft,&theme));
        status.set_text(&match result{Ok(())=>"Imported draft only; validate before applying.".into(),Err(e)=>format!("Import failed: {e:#}")});
    });}
    {let (draft,file,status)=(draft.clone(),file.clone(),status.clone());export.connect_clicked(move|_|{
        let result=PortableTheme::parse(&text(&draft)).and_then(|theme|theme.export(&PathBuf::from(file.text().as_str())));
        status.set_text(&match result{Ok(())=>"Exported new theme file; desktop unchanged.".into(),Err(e)=>format!("Export failed: {e:#}")});
    });}
    {let (draft,swatches,status,apply,state)=(draft.clone(),swatches.clone(),status.clone(),apply.downgrade(),state.clone());
    check.connect_clicked(move|_|{
        match PortableTheme::parse(&text(&draft)){
            Ok(theme)=>{preview(&theme,&swatches);if let Some(a)=apply.upgrade(){a.set_sensitive(state.borrow().is_some());}status.set_text("Valid draft. Apply explicitly to save; current desktop unchanged.");}
            Err(e)=>{if let Some(a)=apply.upgrade(){a.set_sensitive(false);}status.set_text(&format!("Invalid draft: {e:#}"));}
        }
    });}
    {let (draft,status,state)=(draft.clone(),status.clone(),state.clone());apply.connect_clicked(move|button|{
        let mut state=state.borrow_mut();let Some(review)=state.as_mut()else{return;};
        let result=PortableTheme::parse(&text(&draft)).and_then(|theme|theme.applied(&review.value)).and_then(|next|review.save(next));
        status.set_text(&match result{Ok(true)=>"Theme saved. Updated native desktop applies supported preferences live.".into(),Ok(false)=>"No changes.".into(),Err(e)=>format!("Save failed: {e:#}. Reload before retrying.")});button.set_sensitive(false);
    });}
    {let (form,draft,image,variant,status,state)=(form.downgrade(),draft.downgrade(),image.downgrade(),variant.downgrade(),status.downgrade(),state.clone());generate.connect_clicked(move|_|{
        let (Some(f),Some(i),Some(v),Some(s))=(form.upgrade(),image.upgrade(),variant.upgrade(),status.upgrade())else{return;};
        let Some(base)=state.borrow().as_ref().map(|r|r.value.clone())else{s.set_text("Load saved preferences first.");return;};
        let path=PathBuf::from(i.text().as_str());let light=v.active_id().as_deref()==Some("light");
        f.set_sensitive(false);s.set_text("Sampling image…");
        let (tx,rx)=std::sync::mpsc::sync_channel(1);std::thread::spawn(move||{let _=tx.send(wallpaper::palette_seed(&path).and_then(|seed|from_seed(seed,light,&base)));});
        let (form,draft,status)=(form.clone(),draft.clone(),status.clone());
        glib::timeout_add_local(Duration::from_millis(100),move||{
            let result=match rx.try_recv(){Ok(r)=>r,Err(std::sync::mpsc::TryRecvError::Empty)=>return glib::ControlFlow::Continue,Err(_)=>Err(anyhow::anyhow!("Palette worker stopped"))};
            if let (Some(d),Some(s))=(draft.upgrade(),status.upgrade()){
                s.set_text(&match result.and_then(|theme|populate(&d,&theme)){Ok(())=>"Palette generated in draft. Validate and preview before applying.".into(),Err(e)=>format!("Generation failed: {e:#}")});
            }
            if let Some(f)=form.upgrade(){f.set_sensitive(true);}glib::ControlFlow::Break
        });
    });}
    load.emit_clicked();
}
