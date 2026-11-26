//! Offline native login/lock theme draft editor. Never connects to a greeter
//! or collects real credentials. The drawing uses the shared layout contract.
use atelier_config::access_theme::{self,Theme,Rect,Typography,Layout,Palette,PRESETS};
use gtk::prelude::*;
use std::{cell::{Cell,RefCell},path::PathBuf,rc::Rc};
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
// One form owns the editable draft; all paths share Theme::validate.
// The preview holds only the last validated value, never partial input.
struct Fields{
    name:gtk::Entry,layout:gtk::ComboBoxText,typography:gtk::ComboBoxText,
    colors:[gtk::ColorButton;5],radius:gtk::SpinButton,motion:gtk::SpinButton,
    clock:gtk::CheckButton,date:gtk::CheckButton,
}
fn row(parent:&gtk::Box,title:&str,control:&impl IsA<gtk::Widget>){
    let row=gtk::Box::new(gtk::Orientation::Horizontal,12);
    let title=label(title,"body");title.set_hexpand(true);title.set_xalign(0.0);
    row.append(&title);row.append(control);parent.append(&row);
}
fn choice(parent:&gtk::Box,title:&str,options:&[(&str,&str)])->gtk::ComboBoxText{
    let control=gtk::ComboBoxText::new();for (id,name) in options{control.append(Some(id),name);}
    row(parent,title,&control);control
}
fn spin(parent:&gtk::Box,title:&str,max:f64,step:f64)->gtk::SpinButton{
    let control=gtk::SpinButton::with_range(0.0,max,step);row(parent,title,&control);control
}
fn hex(value:gtk::gdk::RGBA)->String{
    let channel=|c:f32|(c.clamp(0.0,1.0)*255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}",channel(value.red()),channel(value.green()),channel(value.blue()))
}
impl Fields{
    fn build(parent:&gtk::Box)->Self{
        let name=gtk::Entry::new();name.set_max_length(80);row(parent,"Theme name",&name);
        let layout=choice(parent,"Layout",&[("centered","Centered"),("split","Split screen"),("editorial","Editorial"),("bento","Bento")]);
        let typography=choice(parent,"Typography",&[("sans","Sans serif"),("monospace","Monospace"),("display","Display")]);
        parent.append(&label("Palette","title-3"));
        let colors=["Background","Authentication panel","Primary text","Secondary text","Accent"].map(|title|{
            let button=gtk::ColorButton::new();button.set_use_alpha(false);button.set_title(title);row(parent,title,&button);button
        });
        parent.append(&label("Geometry and widgets","title-3"));
        let radius=spin(parent,"Corner radius",48.0,1.0);let motion=spin(parent,"Motion duration (ms)",400.0,10.0);
        let clock=gtk::CheckButton::with_label("Show clock");let date=gtk::CheckButton::with_label("Show date");
        parent.append(&clock);parent.append(&date);
        Self{name,layout,typography,colors,radius,motion,clock,date}
    }
    fn populate(&self,theme:&Theme){
        self.name.set_text(&theme.name);
        self.layout.set_active_id(Some(match theme.layout{Layout::Centered=>"centered",Layout::Split=>"split",Layout::Editorial=>"editorial",Layout::Bento=>"bento"}));
        self.typography.set_active_id(Some(match theme.typography{Typography::Sans=>"sans",Typography::Monospace=>"monospace",Typography::Display=>"display"}));
        for (button,value) in self.colors.iter().zip([&theme.palette.background,&theme.palette.panel,&theme.palette.text,&theme.palette.muted,&theme.palette.accent]){
            button.set_rgba(&gtk::gdk::RGBA::parse(value).expect("validated palette"));
        }
        self.radius.set_value(f64::from(theme.corner_radius));self.motion.set_value(f64::from(theme.motion_ms));
        self.clock.set_active(theme.clock);self.date.set_active(theme.date);
    }
    fn read(&self)->anyhow::Result<Theme>{
        let layout=match self.layout.active_id().as_deref(){
            Some("centered")=>Layout::Centered,Some("split")=>Layout::Split,Some("editorial")=>Layout::Editorial,Some("bento")=>Layout::Bento,
            _=>anyhow::bail!("select a layout"),
        };
        let typography=match self.typography.active_id().as_deref(){
            Some("sans")=>Typography::Sans,Some("monospace")=>Typography::Monospace,Some("display")=>Typography::Display,
            _=>anyhow::bail!("select typography"),
        };
        let [background,panel,text,muted,accent]=self.colors.each_ref().map(|button|hex(button.rgba()));
        let theme=Theme{format:"atelier-access-theme".into(),version:1,name:self.name.text().to_string(),layout,typography,
            palette:Palette{background,panel,text,muted,accent},corner_radius:self.radius.value_as_int() as u32,
            motion_ms:self.motion.value_as_int() as u32,clock:self.clock.is_active(),date:self.date.is_active()};
        theme.validate()?;Ok(theme)
    }
    fn connect_changes(&self,refresh:Rc<dyn Fn()>){
        {let refresh=refresh.clone();self.name.connect_changed(move |_|refresh());}
        for combo in [&self.layout,&self.typography]{let refresh=refresh.clone();combo.connect_changed(move |_|refresh());}
        for color in &self.colors{let refresh=refresh.clone();color.connect_rgba_notify(move |_|refresh());}
        for spin in [&self.radius,&self.motion]{let refresh=refresh.clone();spin.connect_value_changed(move |_|refresh());}
        for check in [&self.clock,&self.date]{let refresh=refresh.clone();check.connect_toggled(move |_|refresh());}
    }
}
pub(super) fn build()->gtk::Box{
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Native access theme Studio","title-1"));
    outer.append(&label("Offline login and lock appearance drafts. No live session, authentication, wallpaper assets or external QML are loaded.","dim-label"));
    let form=gtk::Box::new(gtk::Orientation::Vertical,12);
    let preset=gtk::ComboBoxText::new();for (id,name) in PRESETS{preset.append(Some(id),name);}preset.set_active_id(Some("classic"));
    row(&form,"Built-in preset",&preset);
    let load=gtk::Button::with_label("Load preset / reset draft");form.append(&load);
    let initial=access_theme::preset("classic").expect("built-in theme");
    let compact=gtk::CheckButton::with_label("Compact display");let login=gtk::CheckButton::with_label("Greeter preview");
    let mode=gtk::Box::new(gtk::Orientation::Horizontal,8);mode.append(&compact);mode.append(&login);form.append(&mode);
    let preview=gtk::DrawingArea::new();preview.set_content_height(300);preview.set_hexpand(true);form.append(&preview);
    let status=label("Valid Classic draft. No settings changed.","dim-label");status.set_selectable(true);form.append(&status);
    let fields=Rc::new(Fields::build(&form));fields.populate(&initial);
    let path=gtk::Entry::new();path.set_placeholder_text(Some("Absolute .json file path"));form.append(&path);
    let actions=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let import=gtk::Button::with_label("Import draft");let export=gtk::Button::with_label("Export new file");
    actions.append(&import);actions.append(&export);form.append(&actions);
    let current=Rc::new(RefCell::new(initial));
    {
        let (current,compact,login)=(current.clone(),compact.downgrade(),login.downgrade());
        preview.set_draw_func(move|_,cr,w,h|draw(cr,w,h,&current.borrow(),compact.upgrade().is_some_and(|c|c.is_active()),login.upgrade().is_some_and(|c|c.is_active())));
    }
    for button in [&compact,&login]{let preview=preview.downgrade();button.connect_toggled(move|_|{if let Some(p)=preview.upgrade(){p.queue_draw();}});}
    let populating=Rc::new(Cell::new(false));
    let refresh:Rc<dyn Fn()>={
        let (fields,preview,status,export)=(Rc::downgrade(&fields),preview.downgrade(),status.downgrade(),export.downgrade());
        let populating=populating.clone();
        Rc::new(move||{
            if populating.get(){return;}
            let (Some(fields),Some(preview),Some(status),Some(export))=(fields.upgrade(),preview.upgrade(),status.upgrade(),export.upgrade())else{return;};
            match fields.read(){
                Ok(theme)=>{*current.borrow_mut()=theme;preview.queue_draw();status.set_text("Valid draft. No settings changed.");export.set_sensitive(true);}
                Err(e)=>{status.set_text(&format!("Invalid draft: {e:#}. Last valid preview retained."));export.set_sensitive(false);}
            }
        })
    };
    fields.connect_changes(refresh.clone());
    let populate:Rc<dyn Fn(Theme)>={
        let fields=fields.clone();let refresh=refresh.clone();
        Rc::new(move|theme|{populating.set(true);fields.populate(&theme);populating.set(false);refresh();})
    };
    {
        let (preset,status)=(preset.downgrade(),status.downgrade());let populate=populate.clone();
        load.connect_clicked(move|_|{
            let (Some(id),Some(status))=(preset.upgrade().and_then(|p|p.active_id()),status.upgrade())else{return;};
            match access_theme::preset(&id){Ok(theme)=>populate(theme),Err(e)=>status.set_text(&format!("{e:#}"))}
        });
    }
    {
        let (path,status)=(path.downgrade(),status.downgrade());
        import.connect_clicked(move|_|{
            let (Some(path),Some(status))=(path.upgrade(),status.upgrade())else{return;};
            match Theme::import(&PathBuf::from(path.text().as_str())){
                Ok(theme)=>populate(theme),Err(e)=>status.set_text(&format!("Import failed: {e:#}")),
            }
        });
    }
    {
        let (path,status)=(path.downgrade(),status.downgrade());
        export.connect_clicked(move|_|{
            let (Some(path),Some(status))=(path.upgrade(),status.upgrade())else{return;};
            match fields.read().and_then(|t|t.export(&PathBuf::from(path.text().as_str()))){
                Ok(())=>status.set_text("Exported validated theme to a new file. No settings changed."),
                Err(e)=>status.set_text(&format!("Export failed: {e:#}")),
            }
        });
    }
    outer.append(&gtk::ScrolledWindow::builder().vexpand(true).child(&form).build());outer
}
