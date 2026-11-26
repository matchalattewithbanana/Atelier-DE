//! Offline native login/lock theme draft editor. Never connects to a greeter
//! or collects real credentials. The drawing uses the shared layout contract.
use atelier_config::access_theme::{self,Theme,Rect,Typography,Layout,Palette,Backdrop,GradientDirection,ImageFit,DashboardWidgets,Widget,PRESETS};
use gtk::prelude::*;
use atelier_config::{access_bundle::{self,Bundle},wallpaper::{self,Raster},dynamic_palette::{self,Options,Variant,Contrast}};
use std::{cell::{Cell,RefCell},path::PathBuf,rc::Rc};
use super::{label,background};
mod resources;

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
#[derive(Clone)]
struct Wallpaper{raster:Rc<Raster>,surface:gtk::cairo::ImageSurface}
impl Wallpaper{
    fn new(raster:Raster)->anyhow::Result<Self>{
        let (w,h)=(raster.width as i32,raster.height as i32);
        let mut pixels=Vec::with_capacity(raster.rgba.len());
        for p in raster.rgba.chunks_exact(4){
            let a=u32::from(p[3]);let premultiply=|c:u8|(u32::from(c)*a+127)/255;
            let argb=(a<<24)|(premultiply(p[0])<<16)|(premultiply(p[1])<<8)|premultiply(p[2]);
            pixels.extend_from_slice(&argb.to_ne_bytes());
        }
        let surface=gtk::cairo::ImageSurface::create_for_data(pixels,gtk::cairo::Format::ARgb32,w,h,w*4)?;
        Ok(Self{raster:Rc::new(raster),surface})
    }
}
fn backdrop(cr:&gtk::cairo::Context,w:f64,h:f64,theme:&Theme,image:Option<&Wallpaper>){
    color(cr,&theme.palette.background);let _=cr.paint();
    match &theme.backdrop{
        Backdrop::Solid=>{},
        Backdrop::Gradient{end,direction}=>{
            let (x,y)=match direction{GradientDirection::Horizontal=>(w,0.0),GradientDirection::Vertical=>(0.0,h),GradientDirection::Diagonal=>(w,h)};
            let pattern=gtk::cairo::LinearGradient::new(0.0,0.0,x,y);
            for (offset,value) in [(0.0,&theme.palette.background),(1.0,end)]{
                let n=u32::from_str_radix(&value[1..],16).expect("validated gradient");
                pattern.add_color_stop_rgb(offset,f64::from((n>>16)&255)/255.0,f64::from((n>>8)&255)/255.0,f64::from(n&255)/255.0);
            }
            if cr.set_source(&pattern).is_ok(){let _=cr.paint();}
        }
        Backdrop::Wallpaper{fit,dim}=>{
            if let Some(image)=image{
                let iw=f64::from(image.raster.width);let ih=f64::from(image.raster.height);
                let (sx,sy)=match fit{ImageFit::Stretch=>(w/iw,h/ih),ImageFit::Cover=>{let s=(w/iw).max(h/ih);(s,s)},ImageFit::Contain=>{let s=(w/iw).min(h/ih);(s,s)}};
                let _=cr.save();cr.translate((w-iw*sx)/2.0,(h-ih*sy)/2.0);cr.scale(sx,sy);
                if cr.set_source_surface(&image.surface,0.0,0.0).is_ok(){let _=cr.paint();}
                let _=cr.restore();
                cr.set_source_rgba(0.0,0.0,0.0,f64::from(*dim)/100.0);let _=cr.paint();
            }
        }
    }
}
/// Offline-only samples. Never read notifications, weather, media metadata
/// or account details while previewing a theme.
fn widget_preview(cr:&gtk::cairo::Context,area:(f64,f64,f64,f64),widget:Widget,theme:&Theme,scale:f64){
    let (heading,main,detail)=match widget{
        Widget::Weather=>("WEATHER","24°","Sample forecast"),
        Widget::System=>("SYSTEM","Atelier Desktop","Battery 82% · Sample"),
        Widget::Media=>("NOW PLAYING","Sample track","Previous · Play · Next"),
        Widget::Resources=>{resources::draw(cr,area,&theme.palette);return;},
        Widget::Notifications=>("NOTIFICATIONS","Private","Contents hidden"),
    };
    color(cr,&theme.palette.muted);text(cr,area,heading,0.20,9.0*scale);
    color(cr,&theme.palette.accent);text(cr,area,main,0.48,if widget==Widget::Weather{28.0*scale}else{14.0*scale});
    color(cr,&theme.palette.text);text(cr,area,detail,0.73,10.0*scale);
}
fn draw(cr:&gtk::cairo::Context,w:i32,h:i32,theme:&Theme,image:Option<&Wallpaper>,compact:bool,login:bool){
    let (available_w,available_h)=(f64::from(w.max(1)),f64::from(h.max(1)));
    // Fit the selected display aspect ratio into the preview allocation;
    // a narrow settings window should not silently switch the display mode.
    let ratio=if compact{0.82}else{16.0/9.0};
    let w=available_w.min(available_h*ratio);let h=w/ratio;
    let _=cr.save();cr.translate((available_w-w)/2.0,(available_h-h)/2.0);
    cr.rectangle(0.0,0.0,w,h);cr.clip();
    backdrop(cr,w,h,theme,image);
    let compact=compact||w<h*1.55;
    let dashboard=theme.layout==Layout::Dashboard&&!compact&&!login;
    let geometry=theme.layout.geometry(compact);
    if dashboard{
        color(cr,&theme.palette.background);
        card(cr,Rect{x:20,y:40,w:960,h:920},w,h,theme.corner_radius);
    }
    let family=match theme.typography{Typography::Sans=>"sans-serif",Typography::Monospace=>"monospace",Typography::Display=>"serif"};
    cr.select_font_face(family,gtk::cairo::FontSlant::Normal,gtk::cairo::FontWeight::Bold);
    let scale=(w/480.0).clamp(0.5,1.5);
    for tile in theme.dashboard_tiles(compact,login){
        color(cr,&theme.palette.panel);
        let area=card(cr,tile.rect,w,h,theme.corner_radius);
        widget_preview(cr,area,tile.widget,theme,scale);
    }
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
    color(cr,&theme.palette.accent);text(cr,auth,"ATELIER",0.13,11.0*scale);
    if dashboard&&theme.dashboard.profile{
        let (x,y,cw,ch)=auth;
        let r=(cw*0.10).min(ch*0.12);
        let (cx,cy)=(x+cw/2.0,y+ch*0.32);
        color(cr,&theme.palette.background);cr.arc(cx,cy,r,0.0,2.0*std::f64::consts::PI);let _=cr.fill();
        color(cr,&theme.palette.accent);cr.arc(cx,cy-r*0.22,r*0.30,0.0,2.0*std::f64::consts::PI);let _=cr.fill();
        cr.arc(cx,cy+r*0.40,r*0.48,std::f64::consts::PI,2.0*std::f64::consts::PI);cr.close_path();let _=cr.fill();
    }else{
        color(cr,&theme.palette.text);text(cr,auth,if login{"Sign in"}else{"Session locked"},0.36,18.0*scale);
    }
    let (x,y,cw,ch)=auth;
    color(cr,&theme.palette.background);cr.rectangle(x+cw*0.12,y+ch*0.53,cw*0.76,ch*0.17);let _=cr.fill();
    color(cr,&theme.palette.muted);text(cr,auth,"Password preview",0.615,10.0*scale);
    color(cr,&theme.palette.accent);text(cr,auth,if login{"Sign in · session"}else if dashboard{"Session locked · Unlock"}else{"Unlock"},0.83,11.0*scale);
    let _=cr.restore();
}
// One form owns the editable draft; all paths share Theme::validate.
// The preview holds only the last validated value, never partial input.
struct Fields{
    name:gtk::Entry,layout:gtk::ComboBoxText,typography:gtk::ComboBoxText,
    colors:[gtk::ColorButton;5],radius:gtk::SpinButton,motion:gtk::SpinButton,
    clock:gtk::CheckButton,date:gtk::CheckButton,
    background:gtk::ComboBoxText,gradient_end:gtk::ColorButton,direction:gtk::ComboBoxText,
    fit:gtk::ComboBoxText,dim:gtk::SpinButton,
    widgets:[gtk::CheckButton;6],
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
        let layout=choice(parent,"Layout",&[("centered","Centered"),("split","Split screen"),("editorial","Editorial"),("bento","Bento"),("dashboard","Dashboard"),("immersive","Immersive")]);
        let typography=choice(parent,"Typography",&[("sans","Sans serif"),("monospace","Monospace"),("display","Display")]);
        parent.append(&label("Palette","title-3"));
        let colors=["Background","Authentication panel","Primary text","Secondary text","Accent"].map(|title|{
            let button=gtk::ColorButton::new();button.set_use_alpha(false);button.set_title(title);row(parent,title,&button);button
        });
        parent.append(&label("Background","title-3"));
        let background=choice(parent,"Background type",&[("solid","Solid"),("gradient","Gradient"),("wallpaper","Wallpaper")]);
        let gradient_end=gtk::ColorButton::new();gradient_end.set_use_alpha(false);gradient_end.set_title("Gradient end");row(parent,"Gradient end",&gradient_end);
        let direction=choice(parent,"Gradient direction",&[("horizontal","Horizontal"),("vertical","Vertical"),("diagonal","Diagonal")]);
        let fit=choice(parent,"Image fit",&[("cover","Cover"),("contain","Contain"),("stretch","Stretch")]);
        let dim=spin(parent,"Wallpaper dim (%)",80.0,1.0);
        parent.append(&label("Geometry and widgets","title-3"));
        let radius=spin(parent,"Corner radius",48.0,1.0);let motion=spin(parent,"Motion duration (ms)",400.0,10.0);
        let clock=gtk::CheckButton::with_label("Show clock");let date=gtk::CheckButton::with_label("Show date");
        parent.append(&clock);parent.append(&date);
        parent.append(&label("Dashboard widgets","title-3"));
        parent.append(&label("Visibility only; sample data in Studio. Notification contents stay private. Session tiles are omitted on greeter and compact previews.","dim-label"));
        let widgets=["Weather","System information","Media","Resources","Notifications (private indicator)","Profile placeholder"].map(|title|{
            let button=gtk::CheckButton::with_label(title);parent.append(&button);button
        });
        Self{name,layout,typography,colors,radius,motion,clock,date,background,gradient_end,direction,fit,dim,widgets}
    }
    fn populate(&self,theme:&Theme){
        self.name.set_text(&theme.name);
        self.layout.set_active_id(Some(match theme.layout{Layout::Centered=>"centered",Layout::Split=>"split",Layout::Editorial=>"editorial",Layout::Bento=>"bento",Layout::Dashboard=>"dashboard",Layout::Immersive=>"immersive"}));
        self.typography.set_active_id(Some(match theme.typography{Typography::Sans=>"sans",Typography::Monospace=>"monospace",Typography::Display=>"display"}));
        self.set_palette(&theme.palette);
        self.background.set_active_id(Some("solid"));
        self.gradient_end.set_rgba(&gtk::gdk::RGBA::parse("#334455").unwrap());
        self.direction.set_active_id(Some("diagonal"));self.fit.set_active_id(Some("cover"));self.dim.set_value(25.0);
        match &theme.backdrop{
            Backdrop::Solid=>{},
            Backdrop::Gradient{end,direction}=>{
                self.background.set_active_id(Some("gradient"));self.gradient_end.set_rgba(&gtk::gdk::RGBA::parse(end).unwrap());
                self.direction.set_active_id(Some(match direction{GradientDirection::Horizontal=>"horizontal",GradientDirection::Vertical=>"vertical",GradientDirection::Diagonal=>"diagonal"}));
            }
            Backdrop::Wallpaper{fit,dim}=>{
                self.background.set_active_id(Some("wallpaper"));
                self.fit.set_active_id(Some(match fit{ImageFit::Cover=>"cover",ImageFit::Contain=>"contain",ImageFit::Stretch=>"stretch"}));self.dim.set_value(f64::from(*dim));
            }
        }
        self.radius.set_value(f64::from(theme.corner_radius));self.motion.set_value(f64::from(theme.motion_ms));
        self.clock.set_active(theme.clock);self.date.set_active(theme.date);
        let w=theme.dashboard;
        for (widget,active) in self.widgets.iter().zip([w.weather,w.system,w.media,w.resources,w.notifications,w.profile]){widget.set_active(active);}
        self.update_widget_sensitivity();
    }
    fn set_palette(&self,palette:&Palette){
        for (button,value) in self.colors.iter().zip([&palette.background,&palette.panel,&palette.text,&palette.muted,&palette.accent]){
            button.set_rgba(&gtk::gdk::RGBA::parse(value).expect("validated palette"));
        }
    }
    fn read(&self)->anyhow::Result<Theme>{
        let layout=match self.layout.active_id().as_deref(){
            Some("centered")=>Layout::Centered,Some("split")=>Layout::Split,Some("editorial")=>Layout::Editorial,Some("bento")=>Layout::Bento,Some("dashboard")=>Layout::Dashboard,Some("immersive")=>Layout::Immersive,
            _=>anyhow::bail!("select a layout"),
        };
        let typography=match self.typography.active_id().as_deref(){
            Some("sans")=>Typography::Sans,Some("monospace")=>Typography::Monospace,Some("display")=>Typography::Display,
            _=>anyhow::bail!("select typography"),
        };
        let [background,panel,text,muted,accent]=self.colors.each_ref().map(|button|hex(button.rgba()));
        let backdrop=match self.background.active_id().as_deref(){
            Some("solid")=>Backdrop::Solid,
            Some("gradient")=>Backdrop::Gradient{end:hex(self.gradient_end.rgba()),direction:match self.direction.active_id().as_deref(){
                Some("horizontal")=>GradientDirection::Horizontal,Some("vertical")=>GradientDirection::Vertical,_=>GradientDirection::Diagonal,
            }},
            Some("wallpaper")=>Backdrop::Wallpaper{fit:match self.fit.active_id().as_deref(){
                Some("contain")=>ImageFit::Contain,Some("stretch")=>ImageFit::Stretch,_=>ImageFit::Cover,
            },dim:self.dim.value_as_int() as u32},
            _=>anyhow::bail!("select a background type"),
        };
        let [weather,system,media,resources,notifications,profile]=self.widgets.each_ref().map(|w|w.is_active());
        let dashboard=DashboardWidgets{weather,system,media,resources,notifications,profile};
        let theme=Theme{format:"atelier-access-theme".into(),version:1,name:self.name.text().to_string(),layout,typography,
            palette:Palette{background,panel,text,muted,accent},corner_radius:self.radius.value_as_int() as u32,
            motion_ms:self.motion.value_as_int() as u32,clock:self.clock.is_active(),date:self.date.is_active(),backdrop,dashboard};
        theme.validate()?;Ok(theme)
    }
    fn update_widget_sensitivity(&self){
        let enabled=self.layout.active_id().as_deref()==Some("dashboard");
        for widget in &self.widgets{widget.set_sensitive(enabled);}
    }
    fn connect_changes(&self,refresh:Rc<dyn Fn()>){
        let widgets=self.widgets.each_ref().map(|w|w.downgrade());
        self.layout.connect_changed(move|combo|{
            let enabled=combo.active_id().as_deref()==Some("dashboard");
            for widget in &widgets{if let Some(widget)=widget.upgrade(){widget.set_sensitive(enabled);}}
        });
        {let refresh=refresh.clone();self.name.connect_changed(move |_|refresh());}
        for combo in [&self.layout,&self.typography,&self.background,&self.direction,&self.fit]{let refresh=refresh.clone();combo.connect_changed(move |_|refresh());}
        for color in self.colors.iter().chain(std::iter::once(&self.gradient_end)){let refresh=refresh.clone();color.connect_rgba_notify(move |_|refresh());}
        for spin in [&self.radius,&self.motion,&self.dim]{let refresh=refresh.clone();spin.connect_value_changed(move |_|refresh());}
        for check in [&self.clock,&self.date].into_iter().chain(self.widgets.iter()){let refresh=refresh.clone();check.connect_toggled(move |_|refresh());}
    }
}
pub(super) fn build()->gtk::Box{
    let outer=gtk::Box::new(gtk::Orientation::Vertical,12);
    outer.set_margin_top(24);outer.set_margin_bottom(24);outer.set_margin_start(24);outer.set_margin_end(24);
    outer.append(&label("Native access theme Studio","title-1"));
    outer.append(&label("Offline login and lock appearance drafts. No live session, authentication or external QML are loaded. Only explicitly chosen static wallpapers are decoded.","dim-label"));
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
    form.append(&label("Material You palette generator","title-3"));
    form.append(&label("Generate a new editable palette from a color or the loaded wallpaper. Only the five palette colors change.","dim-label"));
    let seed=gtk::ColorButton::new();seed.set_use_alpha(false);seed.set_title("Palette seed");
    seed.set_rgba(&gtk::gdk::RGBA::parse("#6750a4").unwrap());row(&form,"Source color",&seed);
    let variant=choice(&form,"Color scheme",Variant::OPTIONS);variant.set_active_id(Some("tonal-spot"));
    let light=gtk::CheckButton::with_label("Light palette");form.append(&light);
    let high=gtk::CheckButton::with_label("High contrast (7:1)");form.append(&high);
    let from_wallpaper=gtk::Button::with_label("Use loaded wallpaper color");form.append(&from_wallpaper);
    let generate=gtk::Button::with_label("Generate palette into draft");form.append(&generate);
    let image_path=gtk::Entry::new();image_path.set_placeholder_text(Some("Absolute PNG, JPEG or WebP path"));form.append(&image_path);
    let choose_image=gtk::Button::with_label("Load static wallpaper");form.append(&choose_image);
    let asset_status=label("No wallpaper loaded.","dim-label");form.append(&asset_status);
    let path=gtk::Entry::new();path.set_placeholder_text(Some("Absolute .json file or .atelier-access bundle path"));form.append(&path);
    let actions=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let import=gtk::Button::with_label("Import draft");let export=gtk::Button::with_label("Export new file");
    actions.append(&import);actions.append(&export);form.append(&actions);
    let selected_image=Rc::new(RefCell::new(None::<Wallpaper>));
    let current=Rc::new(RefCell::new((initial,None::<Wallpaper>)));
    {
        let (current,compact,login)=(current.clone(),compact.downgrade(),login.downgrade());
        preview.set_draw_func(move|_,cr,w,h|{let current=current.borrow();draw(cr,w,h,&current.0,current.1.as_ref(),compact.upgrade().is_some_and(|c|c.is_active()),login.upgrade().is_some_and(|c|c.is_active()));});
    }
    for button in [&compact,&login]{let preview=preview.downgrade();button.connect_toggled(move|_|{if let Some(p)=preview.upgrade(){p.queue_draw();}});}
    let populating=Rc::new(Cell::new(false));
    let refresh:Rc<dyn Fn()>={
        let (fields,preview,status,export)=(Rc::downgrade(&fields),preview.downgrade(),status.downgrade(),export.downgrade());
        let populating=populating.clone();let selected_image=selected_image.clone();
        Rc::new(move||{
            if populating.get(){return;}
            let (Some(fields),Some(preview),Some(status),Some(export))=(fields.upgrade(),preview.upgrade(),status.upgrade(),export.upgrade())else{return;};
            match fields.read(){
                Ok(theme) if !theme.backdrop.needs_image()||selected_image.borrow().is_some()=>{
                    let image=if theme.backdrop.needs_image(){selected_image.borrow().clone()}else{None};
                    *current.borrow_mut()=(theme,image);preview.queue_draw();status.set_text("Valid draft. No settings changed.");export.set_sensitive(true);
                }
                Ok(_)=>{status.set_text("Load a wallpaper to preview or export this draft. Previous preview retained.");export.set_sensitive(false);}
                Err(e)=>{status.set_text(&format!("Invalid draft: {e:#}. Last valid preview retained."));export.set_sensitive(false);}
            }
        })
    };
    fields.connect_changes(refresh.clone());
    {
        let (seed,status)=(seed.downgrade(),status.downgrade());
        let selected_image=selected_image.clone();
        from_wallpaper.connect_clicked(move|_|{
            let (Some(seed),Some(status))=(seed.upgrade(),status.upgrade())else{return;};
            let selected=selected_image.borrow();
            let Some(image)=selected.as_ref()else{status.set_text("Load a wallpaper first.");return;};
            match wallpaper::seed_from_raster(&image.raster){
                Ok([r,g,b])=>{
                    seed.set_rgba(&gtk::gdk::RGBA::new(f32::from(r)/255.0,f32::from(g)/255.0,f32::from(b)/255.0,1.0));
                    status.set_text("Wallpaper source color selected. Generate to update the draft.");
                }
                Err(e)=>status.set_text(&format!("No usable wallpaper color: {e:#}")),
            }
        });
    }
    {
        let (seed,variant,light,high,status)=(seed.downgrade(),variant.downgrade(),light.downgrade(),high.downgrade(),status.downgrade());
        let fields=fields.clone();let refresh=refresh.clone();let populating=populating.clone();
        generate.connect_clicked(move|_|{
            let (Some(seed),Some(variant),Some(light),Some(high),Some(status))=(seed.upgrade(),variant.upgrade(),light.upgrade(),high.upgrade(),status.upgrade())else{return;};
            let result=(||{
                let variant=Variant::parse(variant.active_id().as_deref().unwrap_or(""))?;
                let seed=atelier_config::design::Color::parse(&hex(seed.rgba()))?.rgb();
                dynamic_palette::generate(seed,Options{variant,light:light.is_active(),contrast:if high.is_active(){Contrast::High}else{Contrast::Normal}})
            })();
            match result{
                Ok(palette)=>{populating.set(true);fields.set_palette(&palette.access);populating.set(false);refresh();}
                Err(e)=>status.set_text(&format!("Palette not changed: {e:#}")),
            }
        });
    }
    let populate:Rc<dyn Fn(Theme,Option<Wallpaper>)>={
        let fields=fields.clone();let refresh=refresh.clone();let selected_image=selected_image.clone();let asset_status=asset_status.downgrade();
        let populating=populating.clone();
        Rc::new(move|theme,image|{
            if let Some(status)=asset_status.upgrade(){status.set_text(if image.is_some(){"Wallpaper loaded from bundle."}else{"No wallpaper loaded."});}
            *selected_image.borrow_mut()=image;
            populating.set(true);fields.populate(&theme);populating.set(false);refresh();
        })
    };
    {
        let (preset,status)=(preset.downgrade(),status.downgrade());let populate=populate.clone();
        load.connect_clicked(move|_|{
            let (Some(id),Some(status))=(preset.upgrade().and_then(|p|p.active_id()),status.upgrade())else{return;};
            match access_theme::preset(&id){Ok(theme)=>populate(theme,None),Err(e)=>status.set_text(&format!("{e:#}"))}
        });
    }
    {
        let (form,image_path,status,asset_status)=(form.downgrade(),image_path.downgrade(),status.downgrade(),asset_status.downgrade());
        let fields=fields.clone();let refresh=refresh.clone();let selected_image=selected_image.clone();
        choose_image.connect_clicked(move|_|{
            let (Some(form),Some(path),Some(status))=(form.upgrade(),image_path.upgrade(),status.upgrade())else{return;};
            let path=PathBuf::from(path.text().as_str());form.set_sensitive(false);status.set_text("Decoding bounded local wallpaper…");
            let (form,status,asset_status)=(form.downgrade(),status.downgrade(),asset_status.clone());
            let (fields,refresh,selected_image)=(fields.clone(),refresh.clone(),selected_image.clone());
            background(move||access_bundle::load_wallpaper(&path),move|result|{
                if let Some(form)=form.upgrade(){form.set_sensitive(true);}
                let Some(status)=status.upgrade()else{return;};
                match result.and_then(Wallpaper::new){
                    Ok(image)=>{
                        *selected_image.borrow_mut()=Some(image);
                        if let Some(label)=asset_status.upgrade(){label.set_text("Static wallpaper loaded into draft.");}
                        fields.background.set_active_id(Some("wallpaper"));refresh();
                    }
                    Err(e)=>status.set_text(&format!("Wallpaper not changed: {e:#}")),
                }
            });
        });
    }
    {
        let (form,path,status)=(form.downgrade(),path.downgrade(),status.downgrade());
        import.connect_clicked(move|_|{
            let (Some(form),Some(path),Some(status))=(form.upgrade(),path.upgrade(),status.upgrade())else{return;};
            let path=PathBuf::from(path.text().as_str());form.set_sensitive(false);status.set_text("Reading theme draft…");
            let (form,status)=(form.downgrade(),status.downgrade());let populate=populate.clone();
            background(move||{
                if path.extension().and_then(|v|v.to_str())==Some("atelier-access"){
                    let bundle=Bundle::import(&path)?;Ok((bundle.theme,Some(bundle.wallpaper)))
                }else{
                    let theme=Theme::import(&path)?;
                    if theme.backdrop.needs_image(){anyhow::bail!("wallpaper themes require an .atelier-access bundle");}
                    Ok((theme,None))
                }
            },move|result|{
                if let Some(form)=form.upgrade(){form.set_sensitive(true);}
                let Some(status)=status.upgrade()else{return;};
                match result.and_then(|(theme,image)|Ok((theme,image.map(Wallpaper::new).transpose()?))){
                    Ok((theme,image))=>populate(theme,image),Err(e)=>status.set_text(&format!("Import failed: {e:#}")),
                }
            });
        });
    }
    {
        let (form,path,status)=(form.downgrade(),path.downgrade(),status.downgrade());
        export.connect_clicked(move|_|{
            let (Some(form),Some(path),Some(status))=(form.upgrade(),path.upgrade(),status.upgrade())else{return;};
            let theme=match fields.read(){Ok(theme)=>theme,Err(e)=>{status.set_text(&format!("{e:#}"));return;}};
            let image=if theme.backdrop.needs_image(){
                let selected=selected_image.borrow();
                let Some(image)=selected.as_ref()else{status.set_text("Load a wallpaper first.");return;};
                Some(Raster{width:image.raster.width,height:image.raster.height,rgba:image.raster.rgba.clone()})
            }else{None};
            let path=PathBuf::from(path.text().as_str());form.set_sensitive(false);status.set_text("Exporting validated draft…");
            let (form,status)=(form.downgrade(),status.downgrade());
            background(move||match image{Some(image)=>Bundle::export(&theme,&image,&path),None=>theme.export(&path)},move|result|{
                if let Some(form)=form.upgrade(){form.set_sensitive(true);}
                if let Some(status)=status.upgrade(){match result{
                    Ok(())=>status.set_text("Exported a new theme file or bundle. No settings changed."),
                    Err(e)=>status.set_text(&format!("Export failed: {e:#}")),
                }}
            });
        });
    }
    outer.append(&gtk::ScrolledWindow::builder().vexpand(true).child(&form).build());outer
}
