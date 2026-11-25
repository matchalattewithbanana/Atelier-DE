//! Graphical draft editor. Only an explicitly reviewed theme reaches the shared appearance file.
use atelier_config::{appearance_path,Appearance,AppearanceReview,WindowBorder,WindowSpacing,themes::{PortableTheme,ShellPalette,from_seed},wallpaper};
use gtk::{gdk,glib,prelude::*};
use std::{cell::{Cell,RefCell},rc::Rc,path::PathBuf,time::Duration};
use super::{description,heading};

fn field(parent:&gtk::Box,label:&str,widget:&impl IsA<gtk::Widget>){
    let row=gtk::Box::new(gtk::Orientation::Horizontal,12);
    let title=gtk::Label::new(Some(label));title.set_xalign(0.0);title.set_hexpand(true);title.set_wrap(true);
    row.append(&title);row.append(widget);parent.append(&row);
}
fn color(parent:&gtk::Box,label:&str)->gtk::ColorButton{
    let button=gtk::ColorButton::new();button.set_use_alpha(false);button.set_title(label);
    button.update_property(&[gtk::accessible::Property::Label(label)]);field(parent,label,&button);button
}
fn spin(parent:&gtk::Box,label:&str,min:f64,max:f64)->gtk::SpinButton{
    let spin=gtk::SpinButton::with_range(min,max,1.0);spin.update_property(&[gtk::accessible::Property::Label(label)]);
    field(parent,label,&spin);spin
}
fn hex(button:&gtk::ColorButton)->String{
    let rgba=button.rgba();
    let byte=|v:f32|(v.clamp(0.0,1.0)*255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}",byte(rgba.red()),byte(rgba.green()),byte(rgba.blue()))
}
fn set_color(button:&gtk::ColorButton,value:&str){
    if let Ok(rgba)=gdk::RGBA::parse(value){button.set_rgba(&rgba);}
}
struct Fields{
    name:gtk::Entry,custom_accent:gtk::CheckButton,accent:gtk::ColorButton,
    custom_palette:gtk::CheckButton,surface:gtk::ColorButton,elevated:gtk::ColorButton,text:gtk::ColorButton,muted:gtk::ColorButton,
    radius:gtk::SpinButton,panel_height:gtk::SpinButton,panel_spacing:gtk::SpinButton,launcher_icon:gtk::SpinButton,dock_icon:gtk::SpinButton,
    inner:gtk::SpinButton,outer:gtk::SpinButton,border:gtk::SpinButton,focused:gtk::ColorButton,unfocused:gtk::ColorButton,
}
impl Fields{
    fn new(parent:&gtk::Box)->Self{
        let name=gtk::Entry::new();name.set_max_length(80);name.set_hexpand(true);field(parent,"Theme name",&name);
        parent.append(&heading("Colors"));
        let custom_accent=gtk::CheckButton::with_label("Custom accent");parent.append(&custom_accent);
        let accent=color(parent,"Accent");
        let custom_palette=gtk::CheckButton::with_label("Custom shell surfaces and text");parent.append(&custom_palette);
        let surface=color(parent,"Surface");let elevated=color(parent,"Elevated surface");
        let text=color(parent,"Primary text");let muted=color(parent,"Muted text");
        parent.append(&heading("Panel and launcher"));
        let radius=spin(parent,"Button radius",0.0,24.0);let panel_height=spin(parent,"Panel minimum height",32.0,80.0);
        let panel_spacing=spin(parent,"Panel spacing",0.0,24.0);let launcher_icon=spin(parent,"Launcher icon size",16.0,64.0);
        let dock_icon=spin(parent,"Dock icon size",16.0,64.0);
        parent.append(&heading("Window appearance"));
        let inner=spin(parent,"Inner grid gap",0.0,48.0);let outer=spin(parent,"Outer grid gap",0.0,64.0);
        let border=spin(parent,"Border width (0 disables)",0.0,8.0);
        let focused=color(parent,"Focused border");let unfocused=color(parent,"Unfocused border");
        Self{name,custom_accent,accent,custom_palette,surface,elevated,text,muted,radius,panel_height,panel_spacing,launcher_icon,dock_icon,inner,outer,border,focused,unfocused}
    }
    fn populate(&self,t:&PortableTheme){
        self.name.set_text(&t.name);self.custom_accent.set_active(!t.accent.is_empty());
        set_color(&self.accent,if t.accent.is_empty(){"#91a8ff"}else{&t.accent});
        self.custom_palette.set_active(t.shell_palette.is_some());
        let fallback=ShellPalette{surface:"#191e29".into(),elevated:"#303747".into(),text:"#f5f5f5".into(),muted:"#bababa".into()};
        let p=t.shell_palette.as_ref().unwrap_or(&fallback);
        for (button,value) in [(&self.surface,&p.surface),(&self.elevated,&p.elevated),(&self.text,&p.text),(&self.muted,&p.muted)]{set_color(button,value);}
        for (spin,value) in [(&self.radius,t.button_radius),(&self.panel_height,t.panel_height),(&self.panel_spacing,t.panel_spacing),
            (&self.launcher_icon,t.launcher_icon_size),(&self.dock_icon,t.dock_icon_size),(&self.inner,t.window_spacing.inner_gap),
            (&self.outer,t.window_spacing.outer_gap),(&self.border,t.window_border.width)]{spin.set_value(value as f64);}
        set_color(&self.focused,&t.window_border.focused);set_color(&self.unfocused,&t.window_border.unfocused);
        self.sensitivity();
    }
    fn sensitivity(&self){
        self.accent.set_sensitive(self.custom_accent.is_active());
        for button in [&self.surface,&self.elevated,&self.text,&self.muted]{button.set_sensitive(self.custom_palette.is_active());}
    }
    fn collect(&self)->PortableTheme{
        let n=|s:&gtk::SpinButton|s.value_as_int() as u32;
        PortableTheme{format:"atelier-native-theme".into(),version:1,name:self.name.text().to_string(),
            accent:if self.custom_accent.is_active(){hex(&self.accent)}else{String::new()},
            shell_palette:self.custom_palette.is_active().then(||ShellPalette{surface:hex(&self.surface),elevated:hex(&self.elevated),text:hex(&self.text),muted:hex(&self.muted)}),
            button_radius:n(&self.radius),panel_height:n(&self.panel_height),panel_spacing:n(&self.panel_spacing),
            launcher_icon_size:n(&self.launcher_icon),dock_icon_size:n(&self.dock_icon),
            window_spacing:WindowSpacing{inner_gap:n(&self.inner),outer_gap:n(&self.outer)},
            window_border:WindowBorder{width:n(&self.border),focused:hex(&self.focused),unfocused:hex(&self.unfocused)}}
    }
    fn changed(&self,callback:impl Fn()+Clone+'static){
        let cb=callback.clone();self.name.connect_changed(move |_|cb());
        for button in [&self.custom_accent,&self.custom_palette]{let cb=callback.clone();button.connect_toggled(move |_|cb());}
        for button in [&self.accent,&self.surface,&self.elevated,&self.text,&self.muted,&self.focused,&self.unfocused]{let cb=callback.clone();button.connect_color_set(move |_|cb());}
        for spin in [&self.radius,&self.panel_height,&self.panel_spacing,&self.launcher_icon,&self.dock_icon,&self.inner,&self.outer,&self.border]{
            let cb=callback.clone();spin.connect_value_changed(move |_|cb());
        }
    }
}
// A schematic, not a screenshot or an application of uncommitted preferences.
fn draw(cr:&gtk::cairo::Context,width:i32,height:i32,t:&PortableTheme){
    let scale=(width as f64/440.0).min(height as f64/240.0);
    if scale<=0.0{return;}
    let _=cr.save();cr.translate((width as f64-440.0*scale)/2.0,(height as f64-240.0*scale)/2.0);cr.scale(scale,scale);
    let fallback=ShellPalette{surface:"#191e29".into(),elevated:"#303747".into(),text:"#f5f5f5".into(),muted:"#bababa".into()};
    let p=t.shell_palette.as_ref().unwrap_or(&fallback);let accent=if t.accent.is_empty(){"#91a8ff"}else{&t.accent};
    let source=|color:&str|{if let Ok(c)=gdk::RGBA::parse(color){cr.set_source_rgb(c.red() as f64,c.green() as f64,c.blue() as f64);}};
    let rect=|x:f64,y:f64,w:f64,h:f64,r:f64,color:&str|{
        if w<=0.0||h<=0.0{return;}
        let r=r.max(0.0).min(w/2.0).min(h/2.0);let pi=std::f64::consts::PI;
        cr.new_sub_path();cr.arc(x+w-r,y+r,r,-pi/2.0,0.0);cr.arc(x+w-r,y+h-r,r,0.0,pi/2.0);
        cr.arc(x+r,y+h-r,r,pi/2.0,pi);cr.arc(x+r,y+r,r,pi,pi*1.5);cr.close_path();source(color);let _=cr.fill();
    };
    let label=|x:f64,y:f64,size:f64,value:&str,color:&str|{source(color);cr.set_font_size(size);cr.move_to(x,y);let _=cr.show_text(value);};
    rect(0.0,0.0,440.0,240.0,10.0,&p.surface);
    let panel=t.panel_height as f64*0.55;let radius=t.button_radius as f64*0.45;
    rect(6.0,6.0,428.0,panel,radius,&p.elevated);
    rect(12.0,10.0,58.0,(panel-8.0).max(8.0),radius,accent);
    for i in 0..4{let x=78.0+i as f64*(17.0+t.panel_spacing as f64*0.45);rect(x,14.0,12.0,8.0,2.0,if i==0{accent}else{&p.muted});}
    label(365.0,6.0+panel/2.0+4.0,10.0,"12:00",&p.text);
    let outer=t.window_spacing.outer_gap as f64*0.65;let gap=t.window_spacing.inner_gap as f64*0.65;
    let y=panel+14.0+outer;let bottom=194.0-outer;
    let w=(424.0-2.0*outer-gap)/2.0;let h=bottom-y;
    for i in 0..2{
        let x=8.0+outer+i as f64*(w+gap);let border=(t.window_border.width as f64*0.6).min(h.max(0.0)/2.0);
        if border>0.0{rect(x,y,w,h,0.0,if i==0{&t.window_border.focused}else{&t.window_border.unfocused});}
        rect(x+border,y+border,w-2.0*border,h-2.0*border,0.0,&p.elevated);
        if h>30.0{
            label(x+8.0+border,y+17.0+border,11.0,if i==0{"Focused window"}else{"Inactive window"},&p.text);
            for j in 0..3{let line_y=y+28.0+j as f64*9.0;if line_y+3.0<bottom-border{rect(x+8.0+border,line_y,(w-20.0-border*2.0)*(1.0-j as f64*0.15),3.0,1.0,&p.muted);}}
        }
    }
    let icon=t.dock_icon_size as f64*0.43;let dock_w=5.0*(icon+9.0)+8.0;
    let dock_h=icon+10.0;let x=(440.0-dock_w)/2.0;
    rect(x,240.0-dock_h-5.0,dock_w,dock_h,radius,&p.elevated);
    for i in 0..5{rect(x+8.0+i as f64*(icon+9.0),240.0-dock_h,icon,icon,radius,if i==0{accent}else{&p.muted});}
    let _=cr.restore();
}
fn chooser(button:&gtk::Button,entry:&gtk::Entry,save:bool,image:bool){
    let entry=entry.downgrade();
    button.connect_clicked(move|button|{
        let parent=button.root().and_downcast::<gtk::Window>();
        let dialog=gtk::FileChooserNative::new(Some(if image{"Choose palette image"}else if save{"Choose new theme file"}else{"Import native theme"}),parent.as_ref(),
            if save{gtk::FileChooserAction::Save}else{gtk::FileChooserAction::Open},Some(if save{"Select destination"}else{"Select"}),Some("Cancel"));
        let filter=gtk::FileFilter::new();filter.set_name(Some(if image{"Raster images"}else{"Native theme JSON"}));
        for pattern in if image{vec!["*.png","*.jpg","*.jpeg","*.webp","*.PNG","*.JPG","*.JPEG","*.WEBP"]}else{vec!["*.json"]}{filter.add_pattern(pattern);}
        dialog.add_filter(&filter);if save{dialog.set_current_name("atelier-theme.json");}
        let entry=entry.clone();dialog.connect_response(move|dialog,response|{
            if response==gtk::ResponseType::Accept{
                if let (Some(entry),Some(path))=(entry.upgrade(),dialog.file().and_then(|f|f.path()).and_then(|p|p.into_os_string().into_string().ok())){entry.set_text(&path);}
            }
            dialog.destroy();
        });dialog.show();
    });
}
pub fn build(parent:&gtk::Box){
    let path=match appearance_path(){Ok(p)=>p,Err(e)=>{parent.append(&description(&format!("{e:#}")));return;}};
    let form=gtk::Box::new(gtk::Orientation::Vertical,12);parent.append(&form);
    let canvas=gtk::DrawingArea::new();canvas.set_content_width(440);canvas.set_content_height(240);canvas.set_hexpand(true);
    canvas.update_property(&[gtk::accessible::Property::Label("Illustrative desktop theme preview")]);form.append(&canvas);
    let preview_state=Rc::new(RefCell::new(None::<PortableTheme>));
    {let model=preview_state.clone();canvas.set_draw_func(move|_,cr,w,h|{if let Some(t)=model.borrow().as_ref(){draw(cr,w,h,t);}});}
    let validity=description("Load a draft to begin.");form.append(&validity);
    let fields=Rc::new(Fields::new(&form));
    form.append(&heading("Wallpaper palette"));
    let image=gtk::Entry::new();image.set_placeholder_text(Some("Absolute local image path"));form.append(&image);
    let browse=gtk::Button::with_label("Choose image");chooser(&browse,&image,false,true);form.append(&browse);
    let variant=gtk::ComboBoxText::new();variant.append(Some("dark"),"Dark palette");variant.append(Some("light"),"Light palette");variant.set_active_id(Some("dark"));form.append(&variant);
    let generate=gtk::Button::with_label("Generate colors into draft");form.append(&generate);
    form.append(&heading("Portable themes"));
    let file=gtk::Entry::new();file.set_placeholder_text(Some("Absolute .json theme path"));form.append(&file);
    let open=gtk::Button::with_label("Choose import file");chooser(&open,&file,false,false);
    let dest=gtk::Button::with_label("Choose export destination");chooser(&dest,&file,true,false);
    let row=gtk::Box::new(gtk::Orientation::Horizontal,6);row.append(&open);row.append(&dest);form.append(&row);
    let row=gtk::Box::new(gtk::Orientation::Horizontal,6);
    let import=gtk::Button::with_label("Import draft");let export=gtk::Button::with_label("Export draft (new file)");
    row.append(&import);row.append(&export);form.append(&row);
    let json=gtk::TextView::new();json.set_editable(false);json.set_monospace(true);json.set_wrap_mode(gtk::WrapMode::WordChar);
    let expander=gtk::Expander::new(Some("Inspect validated JSON"));expander.set_child(Some(&gtk::ScrolledWindow::builder().min_content_height(160).child(&json).build()));form.append(&expander);
    let load=gtk::Button::with_label("Reload saved theme");form.append(&load);
    let previous=gtk::Button::with_label("Load previous visual preferences into draft");form.append(&previous);
    let defaults=gtk::Button::with_label("Load default visual preferences into draft");form.append(&defaults);
    let check=gtk::Button::with_label("Review draft");form.append(&check);
    let apply=gtk::Button::with_label("Apply reviewed theme");apply.add_css_class("suggested-action");apply.set_sensitive(false);form.append(&apply);
    let status=description("Themes exclude wallpaper paths, pins, widget selection and commands.");status.set_selectable(true);parent.append(&status);
    let baseline=Rc::new(RefCell::new(None::<AppearanceReview>));
    let reviewed=Rc::new(RefCell::new(None::<PortableTheme>));
    let populating=Rc::new(Cell::new(false));
    let refresh={
        let (fields,canvas,validity,json,apply)=(Rc::downgrade(&fields),canvas.downgrade(),validity.downgrade(),json.downgrade(),apply.downgrade());
        let (model,reviewed,populating)=(preview_state.clone(),reviewed.clone(),populating.clone());
        move||{
            if populating.get(){return;}
            let (Some(f),Some(canvas),Some(validity),Some(json),Some(apply))=(fields.upgrade(),canvas.upgrade(),validity.upgrade(),json.upgrade(),apply.upgrade())else{return;};
            f.sensitivity();reviewed.borrow_mut().take();apply.set_sensitive(false);
            let theme=f.collect();
            match theme.json(){
                Ok(value)=>{*model.borrow_mut()=Some(theme);json.buffer().set_text(&value);validity.set_text("Valid draft · preview only. Review before applying.");}
                Err(e)=>{model.borrow_mut().take();json.buffer().set_text("");validity.set_text(&format!("Invalid draft: {e:#}"));}
            }
            canvas.queue_draw();
        }
    };
    fields.changed(refresh.clone());
    let populate={
        let (fields,populating,refresh)=(Rc::downgrade(&fields),populating.clone(),refresh.clone());
        move|theme:&PortableTheme|{
            if let Some(fields)=fields.upgrade(){populating.set(true);fields.populate(theme);populating.set(false);refresh();}
        }
    };
    {let (populate,image,status,baseline,reviewed,apply)=(populate.clone(),image.clone(),status.clone(),baseline.clone(),reviewed.clone(),apply.downgrade());
    load.connect_clicked(move|_|{
        reviewed.borrow_mut().take();if let Some(a)=apply.upgrade(){a.set_sensitive(false);}
        match AppearanceReview::load(&path){
            Ok(review)=>{image.set_text(&review.value.wallpaper_path);populate(&PortableTheme::capture("Current native theme".into(),&review.value));*baseline.borrow_mut()=Some(review);status.set_text("Saved preferences loaded into draft.");}
            Err(e)=>{baseline.borrow_mut().take();status.set_text(&format!("Cannot load profile: {e:#}"));}
        }
    });}
    {let (populate,status,baseline)=(populate.clone(),status.clone(),baseline.clone());previous.connect_clicked(move|_|{
        let state=baseline.borrow();let Some(review)=state.as_ref()else{status.set_text("Reload saved preferences first.");return;};
        match review.previous(){Ok(a)=>{populate(&PortableTheme::capture("Previous native theme".into(),&a));status.set_text("Previous visual preferences loaded into draft.");}Err(e)=>status.set_text(&format!("{e:#}"))}
    });}
    {let (populate,status)=(populate.clone(),status.clone());defaults.connect_clicked(move|_|{
        populate(&PortableTheme::capture("Default native theme".into(),&Appearance::default()));status.set_text("Defaults loaded into draft only.");
    });}
    {let (populate,file,status)=(populate.clone(),file.clone(),status.clone());import.connect_clicked(move|_|{
        match PortableTheme::import(&PathBuf::from(file.text().as_str())){
            Ok(t)=>{populate(&t);status.set_text("Imported draft only.");}
            Err(e)=>status.set_text(&format!("Import failed: {e:#}")),
        }
    });}
    {let (fields,file,status)=(fields.clone(),file.clone(),status.clone());export.connect_clicked(move|_|{
        let result=fields.collect().export(&PathBuf::from(file.text().as_str()));
        status.set_text(&match result{Ok(())=>"Exported new theme file; desktop unchanged.".into(),Err(e)=>format!("Export failed: {e:#}")});
    });}
    {let (fields,status,apply,baseline,reviewed)=(fields.clone(),status.clone(),apply.downgrade(),baseline.clone(),reviewed.clone());check.connect_clicked(move|_|{
        reviewed.borrow_mut().take();if let Some(a)=apply.upgrade(){a.set_sensitive(false);}
        let theme=fields.collect();let state=baseline.borrow();let Some(r)=state.as_ref()else{status.set_text("Reload saved preferences first.");return;};
        match theme.applied(&r.value){
            Ok(_)=>{*reviewed.borrow_mut()=Some(theme);if let Some(a)=apply.upgrade(){a.set_sensitive(true);}status.set_text("Reviewed visual settings. Apply to save and update the native desktop.");}
            Err(e)=>status.set_text(&format!("Invalid draft: {e:#}")),
        }
    });}
    {let (fields,status,baseline,reviewed)=(fields.clone(),status.clone(),baseline.clone(),reviewed.clone());apply.connect_clicked(move|button|{
        button.set_sensitive(false);
        let Some(theme)=reviewed.borrow_mut().take()else{return;};
        if fields.collect()!=theme{status.set_text("Draft changed; review again.");return;}
        let mut state=baseline.borrow_mut();let Some(r)=state.as_mut()else{return;};
        let result=theme.applied(&r.value).and_then(|next|r.save(next));
        status.set_text(&match result{Ok(true)=>"Theme saved; updated native desktop applies supported preferences live.".into(),Ok(false)=>"No changes.".into(),Err(e)=>format!("Save failed: {e:#}. Reload before retrying.")});
    });}
    {let (fields,form,image,variant,status,baseline,populate)=(fields.clone(),form.downgrade(),image.downgrade(),variant.downgrade(),status.downgrade(),baseline.clone(),populate.clone());
    generate.connect_clicked(move|_|{
        let (Some(form_widget),Some(image_widget),Some(variant_widget),Some(status_widget))=(form.upgrade(),image.upgrade(),variant.upgrade(),status.upgrade())else{return;};
        let base={let state=baseline.borrow();let Some(r)=state.as_ref()else{status_widget.set_text("Reload saved preferences first.");return;};
            // Preserve unsaved geometry even when the palette itself needs replacement.
            let mut draft=fields.collect();draft.shell_palette=None;draft.accent.clear();
            match draft.applied(&r.value){Ok(a)=>a,Err(e)=>{status_widget.set_text(&format!("Fix draft geometry/name: {e:#}"));return;}}
        };
        let path=PathBuf::from(image_widget.text().as_str());let light=variant_widget.active_id().as_deref()==Some("light");
        form_widget.set_sensitive(false);status_widget.set_text("Sampling image…");
        let (tx,rx)=std::sync::mpsc::sync_channel(1);std::thread::spawn(move||{let _=tx.send(wallpaper::palette_seed(&path).and_then(|seed|from_seed(seed,light,&base)));});
        let (form,status,populate)=(form.clone(),status.clone(),populate.clone());
        glib::timeout_add_local(Duration::from_millis(100),move||{
            if form.upgrade().is_none(){return glib::ControlFlow::Break;}
            let result=match rx.try_recv(){Ok(r)=>r,Err(std::sync::mpsc::TryRecvError::Empty)=>return glib::ControlFlow::Continue,Err(_)=>Err(anyhow::anyhow!("Palette worker stopped"))};
            if let Some(s)=status.upgrade(){match result{Ok(t)=>{populate(&t);s.set_text("Generated colors in draft; review before applying.");}Err(e)=>s.set_text(&format!("Generation failed: {e:#}"))}}
            if let Some(f)=form.upgrade(){f.set_sensitive(true);}glib::ControlFlow::Break
        });
    });}
    load.emit_clicked();
}
