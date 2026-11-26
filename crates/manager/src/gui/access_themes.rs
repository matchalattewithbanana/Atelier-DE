//! Offline native login/lock theme draft editor. Never connects to a greeter
//! or collects real credentials. The drawing uses the shared layout contract.
use atelier_config::access_theme::{self,Theme,Rect,Typography,PRESETS};
use gtk::prelude::*;
use std::{cell::RefCell,path::PathBuf,rc::Rc};
use super::label;

fn color(cr:&gtk::cairo::Context,value:&str){
    let n=u32::from_str_radix(&value[1..],16).expect("validated theme color");
    cr.set_source_rgb(f64::from((n>>16)&255)/255.0,f64::from((n>>8)&255)/255.0,f64::from(n&255)/255.0);
}
fn card(cr:&gtk::cairo::Context,r:Rect,w:f64,h:f64,radius:u32)->(f64,f64,f64,f64){
    let (x,y,w,h)=(w*f64::from(r.x)/1000.0,h*f64::from(r.y)/1000.0,w*f64::from(r.w)/1000.0,h*f64::from(r.h)/1000.0);
    let rad=(f64::from(radius)*0.55).min(w/2.0).min(h/2.0);
    let pi=std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x+w-rad,y+rad,rad,-pi/2.0,0.0);
    cr.arc(x+w-rad,y+h-rad,rad,0.0,pi/2.0);
    cr.arc(x+rad,y+h-rad,rad,pi/2.0,pi);
    cr.arc(x+rad,y+rad,rad,pi,pi*1.5);
    cr.close_path();let _=cr.fill();
    (x,y,w,h)
}
fn text(cr:&gtk::cairo::Context,area:(f64,f64,f64,f64),line:&str,relative_y:f64,size:f64){
    let (x,y,w,h)=area;
    cr.set_font_size(size.min(w/5.0).max(1.0));
    if let Ok(e)=cr.text_extents(line){
        if e.width()>w*0.88{cr.set_font_size(size*(w*0.88/e.width()));}
    }
    if let Ok(e)=cr.text_extents(line){
        cr.move_to(x+(w-e.width())/2.0-e.x_bearing(),y+h*relative_y-e.y_bearing()-e.height()/2.0);
        let _=cr.show_text(line);
    }
}
fn draw(cr:&gtk::cairo::Context,w:i32,h:i32,theme:&Theme,compact:bool,login:bool){
    let (w,h)=(f64::from(w.max(1)),f64::from(h.max(1)));
    let (w,offset)=if compact{let narrow=w.min(h*0.82);(narrow,(w-narrow)/2.0)}else{(w,0.0)};
    let _=cr.save();cr.translate(offset,0.0);
    cr.rectangle(0.0,0.0,w,h);cr.clip();
    color(cr,&theme.palette.background);let _=cr.paint();
    let geometry=theme.layout.geometry(compact||w<h*1.1);
    let family=match theme.typography{Typography::Sans=>"sans-serif",Typography::Monospace=>"monospace",Typography::Display=>"serif"};
    cr.select_font_face(family,gtk::cairo::FontSlant::Normal,gtk::cairo::FontWeight::Bold);
    let scale=(w/480.0).clamp(0.5,1.5);
    if theme.clock||theme.date{
        color(cr,&theme.palette.panel);
        let clock=card(cr,geometry.clock,w,h,theme.corner_radius);
        color(cr,&theme.palette.accent);
        if theme.clock{text(cr,clock,"09:41",0.40,32.0*scale);}
        color(cr,&theme.palette.muted);
        if theme.date{text(cr,clock,"Thursday · 26 November",0.68,11.0*scale);}
    }
    color(cr,&theme.palette.panel);
    let auth=card(cr,geometry.auth,w,h,theme.corner_radius);
    color(cr,&theme.palette.accent);text(cr,auth,"ATELIER",0.17,11.0*scale);
    color(cr,&theme.palette.text);text(cr,auth,if login{"Sign in"}else{"Session locked"},0.36,18.0*scale);
    let (x,y,cw,ch)=auth;
    color(cr,&theme.palette.background);cr.rectangle(x+cw*0.12,y+ch*0.53,cw*0.76,ch*0.17);let _=cr.fill();
    color(cr,&theme.palette.muted);text(cr,auth,"Password preview",0.615,10.0*scale);
    color(cr,&theme.palette.accent);text(cr,auth,if login{"Sign in · session"}else{"Unlock"},0.83,11.0*scale);
    let _=cr.restore();
}
pub(super) fn build()->gtk::Box{
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Native access theme Studio","title-1"));
    outer.append(&label("Offline login and lock appearance drafts. No live session, authentication, wallpaper assets or external QML are loaded.","dim-label"));
    let form=gtk::Box::new(gtk::Orientation::Vertical,8);
    let preset=gtk::ComboBoxText::new();for (id,name) in PRESETS{preset.append(Some(id),name);}preset.set_active_id(Some("classic"));form.append(&preset);
    let load=gtk::Button::with_label("Load preset into draft");form.append(&load);
    let initial=access_theme::preset("classic").expect("built-in theme");
    let buffer=gtk::TextBuffer::new(None::<&gtk::TextTagTable>);buffer.set_text(&initial.json().unwrap());
    let draft=gtk::TextView::with_buffer(&buffer);draft.set_monospace(true);draft.set_wrap_mode(gtk::WrapMode::WordChar);
    form.append(&gtk::ScrolledWindow::builder().min_content_height(200).child(&draft).build());
    let path=gtk::Entry::new();path.set_placeholder_text(Some("Absolute .json file path"));form.append(&path);
    let actions=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let import=gtk::Button::with_label("Import draft");let validate=gtk::Button::with_label("Validate and preview");let export=gtk::Button::with_label("Export new file");
    actions.append(&import);actions.append(&validate);actions.append(&export);form.append(&actions);
    let compact=gtk::CheckButton::with_label("Compact display");let login=gtk::CheckButton::with_label("Greeter preview");
    form.append(&compact);form.append(&login);
    let preview=gtk::DrawingArea::new();preview.set_content_height(300);preview.set_hexpand(true);form.append(&preview);
    let status=label("Built-in Classic preview. Editing the draft requires validation to update the preview.","dim-label");status.set_selectable(true);form.append(&status);
    let current=Rc::new(RefCell::new(initial));
    {
        let (current,compact,login)=(current.clone(),compact.downgrade(),login.downgrade());
        preview.set_draw_func(move|_,cr,w,h|draw(cr,w,h,&current.borrow(),compact.upgrade().is_some_and(|c|c.is_active()),login.upgrade().is_some_and(|c|c.is_active())));
    }
    for button in [&compact,&login]{let preview=preview.downgrade();button.connect_toggled(move|_|{if let Some(p)=preview.upgrade(){p.queue_draw();}});}
    {
        let (preset,buffer,status)=(preset.downgrade(),buffer.downgrade(),status.downgrade());
        load.connect_clicked(move|_|{
            let (Some(id),Some(buffer),Some(status))=(preset.upgrade().and_then(|p|p.active_id()),buffer.upgrade(),status.upgrade())else{return;};
            match access_theme::preset(&id).and_then(|t|t.json()){Ok(json)=>{buffer.set_text(&json);status.set_text("Preset loaded. Validate to update the preview.");},Err(e)=>status.set_text(&format!("{e:#}"))}
        });
    }
    {
        let (path,buffer,status)=(path.downgrade(),buffer.downgrade(),status.downgrade());
        import.connect_clicked(move|_|{
            let (Some(path),Some(buffer),Some(status))=(path.upgrade(),buffer.upgrade(),status.upgrade())else{return;};
            match Theme::import(&PathBuf::from(path.text().as_str())).and_then(|t|t.json()){
                Ok(json)=>{buffer.set_text(&json);status.set_text("Imported draft. Validate to update the preview.");}
                Err(e)=>status.set_text(&format!("Import failed: {e:#}")),
            }
        });
    }
    {
        let (buffer,status,preview)=(buffer.downgrade(),status.downgrade(),preview.downgrade());
        validate.connect_clicked(move|_|{
            let (Some(buffer),Some(status),Some(preview))=(buffer.upgrade(),status.upgrade(),preview.upgrade())else{return;};
            let text=buffer.text(&buffer.start_iter(),&buffer.end_iter(),false);
            match Theme::parse(&text){Ok(theme)=>{*current.borrow_mut()=theme;preview.queue_draw();status.set_text("Validated preview. No settings changed.");},Err(e)=>status.set_text(&format!("Invalid draft: {e:#}. Previous preview retained."))}
        });
    }
    {
        let (path,buffer,status)=(path.downgrade(),buffer.downgrade(),status.downgrade());
        export.connect_clicked(move|_|{
            let (Some(path),Some(buffer),Some(status))=(path.upgrade(),buffer.upgrade(),status.upgrade())else{return;};
            let text=buffer.text(&buffer.start_iter(),&buffer.end_iter(),false);
            match Theme::parse(&text).and_then(|t|t.export(&PathBuf::from(path.text().as_str()))){
                Ok(())=>status.set_text("Exported validated theme to a new file. No settings changed."),
                Err(e)=>status.set_text(&format!("Export failed: {e:#}")),
            }
        });
    }
    outer.append(&gtk::ScrolledWindow::builder().vexpand(true).child(&form).build());outer
}
